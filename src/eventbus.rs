use std::any::TypeId;
use std::collections::HashMap;

use crate::{many, single};

/// Opaque handle returned by [`EventBus::on`]. Pass to [`EventBus::off`] to unsubscribe.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SubscriptionId(u64);

// Type-erased subscriber record. `call` and `drop` are monomorphized fn
// pointers baked in at subscribe time for the concrete `(E, F)`, so
// dispatch is one indirect call with no vtable double-hop and no runtime
// downcast.
//
// Shared between `single::Slot` and `many::List`; each stores one of these
// (or a Vec of them) inline.
//
// Rationale, measured impact, safety proof, and the rejected SoA layout all
// live in `docs/internal/trampoline.md` (§2–§5 for safety, §8 for numbers,
// §10 for SoA).
pub struct Subscriber {
    // `Box::<F>::into_raw` for the closure; live until this `Subscriber` drops.
    pub data: *const (),
    // Trampoline for the exact `(E, F)` that produced `data`: casts `data` to
    // `&F`, event to `&E`, invokes `F`.
    pub call: unsafe fn(data: *const (), event: *const ()),
    pub id: SubscriptionId,
    // Destructor for the same `F`.
    pub drop: unsafe fn(data: *const ()),
}

impl Drop for Subscriber {
    fn drop(&mut self) {
        // SAFETY: by the Subscriber invariant (see docs/internal/trampoline.md §2),
        // `self.drop` is `drop_trampoline::<F>` paired with `self.data =
        // Box::<F>::into_raw(_)` which is still live. `Subscriber::drop` runs
        // at most once, so the box is freed at most once.
        unsafe { (self.drop)(self.data) }
    }
}

// Per-type storage. Auto-specialized by subscriber count:
//
//   N = 0  — no entry in `buckets` (HashMap miss path)
//   N = 1  — `Single(Slot)`, inline, no Vec, no second heap indirection
//   N ≥ 2  — `Many(List)`, Vec of subscribers
//
// Promote on the second `on` for a type; demote on `off` when one survives.
enum Bucket {
    Single(single::Slot),
    Many(many::List),
}

/// A typed, sync, zero-dependency event bus.
//
// `HashMap<TypeId, Bucket>` where each `Bucket` is one of `single::Slot` or
// `many::List`. Dispatch is O(subscribers) for the matching type. Subscribe
// is O(1) amortized. Unsubscribe is O(types) to locate the bucket plus
// O(subscribers) inside it.
pub struct EventBus {
    buckets: HashMap<TypeId, Bucket>,
    // Monotonic counter for subscription IDs.
    next_id: u64,
}

impl EventBus {
    /// Create an empty bus.
    // Allocates nothing until the first subscription.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buckets: HashMap::new(),
            next_id: 0,
        }
    }

    /// Subscribe to events of type `E`.
    ///
    /// Returns a [`SubscriptionId`] that can be passed to [`off`](Self::off).
    /// The callback receives `&E` and must be sync.
    ///
    /// ```
    /// # use rt_events::EventBus;
    /// # struct Hit { damage: u32 }
    /// let mut bus = EventBus::new();
    /// let id = bus.on(|e: &Hit| println!("took {} damage", e.damage));
    /// ```
    pub fn on<E: 'static>(&mut self, callback: impl Fn(&E) + 'static) -> SubscriptionId {
        // Forward to the named-F impl so the trampolines can be monomorphized
        // on the concrete closure type. Keeping the public signature with
        // `impl Trait` preserves source compatibility for `bus.on::<E>(...)`.
        self.on_impl::<E, _>(callback)
    }

    fn on_impl<E: 'static, F: Fn(&E) + 'static>(&mut self, callback: F) -> SubscriptionId {
        let id = SubscriptionId(self.next_id);
        self.next_id += 1;

        let sub = Subscriber {
            data: Box::into_raw(Box::new(callback)).cast::<()>(),
            call: call_trampoline::<E, F>,
            id,
            drop: drop_trampoline::<F>,
        };

        let type_id = TypeId::of::<E>();
        // remove + match + insert: two HashMap ops in the populated case,
        // but lets us pattern-match *and* move ownership across variants
        // without fighting the borrow checker. `on` is not hot.
        let new_bucket = match self.buckets.remove(&type_id) {
            None => Bucket::Single(single::Slot::new(sub)),
            Some(Bucket::Single(old)) => {
                Bucket::Many(many::List::from_two(old.into_inner(), sub))
            }
            Some(Bucket::Many(mut list)) => {
                list.push(sub);
                Bucket::Many(list)
            }
        };
        self.buckets.insert(type_id, new_bucket);

        id
    }

    /// Emit an event. All subscribers for `E` are called synchronously
    /// in registration order with `&E`.
    ///
    /// If no subscribers exist for `E`, this is a no-op.
    ///
    /// ```
    /// # use rt_events::EventBus;
    /// # struct Hit { damage: u32 }
    /// # let bus = EventBus::new();
    /// bus.emit(Hit { damage: 42 });
    /// ```
    // Takes `E` by value rather than `&E`: the bus "consumes and delivers",
    // matching stdlib conventions (`mpsc::Sender::send`, `HashMap::insert`)
    // and keeping the call site clean (`bus.emit(Event { .. })` without `&`).
    // Handlers still receive `&E`; the owned value is borrowed internally.
    #[allow(clippy::needless_pass_by_value)]
    pub fn emit<E: 'static>(&self, event: E) {
        let Some(bucket) = self.buckets.get(&TypeId::of::<E>()) else {
            return;
        };
        let event_ptr = core::ptr::from_ref::<E>(&event).cast::<()>();
        // One branch on the N=1 vs N≥2 split; each arm's dispatch lives in
        // its own module. The outer `TypeId` key already proves every
        // `Subscriber` in the bucket was registered for this exact `E`, so
        // the dispatch methods only need to know "here is a live `&E`".
        match bucket {
            // SAFETY: `event_ptr` is a live `&E`; the slot was built from
            // `on::<E, _>`, so its trampoline is monomorphized for this `E`.
            Bucket::Single(slot) => unsafe { slot.dispatch(event_ptr) },
            // SAFETY: same invariant, applied to every entry in the list.
            Bucket::Many(list) => unsafe { list.dispatch(event_ptr) },
        }
    }

    /// Remove a subscription. Returns `true` if the subscription was found.
    pub fn off(&mut self, id: SubscriptionId) -> bool {
        // First pass: walk buckets to locate the id and update in-place where
        // possible. Single-matches and demotions need a second step because
        // they change the bucket variant; record the type id and act after
        // the borrow on `self.buckets` is released.
        let mut action = OffAction::NotFound;
        for (&type_id, bucket) in &mut self.buckets {
            // Pattern guards can't borrow mutably, so the `list.remove(id)`
            // check lives in the arm body, not the guard.
            match bucket {
                Bucket::Single(slot) => {
                    if slot.matches(id) {
                        action = OffAction::DropBucket(type_id);
                        break;
                    }
                }
                Bucket::Many(list) => {
                    if list.remove(id) {
                        action = if list.len() == 1 {
                            OffAction::Demote(type_id)
                        } else {
                            OffAction::Done
                        };
                        break;
                    }
                }
            }
        }

        match action {
            OffAction::NotFound => false,
            OffAction::Done => true,
            OffAction::DropBucket(t) => {
                self.buckets.remove(&t);
                true
            }
            OffAction::Demote(t) => {
                // `list.into_single()` returns the lone survivor; rewrap
                // it as a `Single` so the next emit skips the Vec path.
                if let Some(Bucket::Many(list)) = self.buckets.remove(&t) {
                    if let Some(sub) = list.into_single() {
                        self.buckets
                            .insert(t, Bucket::Single(single::Slot::new(sub)));
                    }
                }
                true
            }
        }
    }

    /// Number of event types with at least one subscriber.
    #[must_use]
    pub fn type_count(&self) -> usize {
        self.buckets.len()
    }

    /// Number of subscribers for a given event type.
    #[must_use]
    pub fn subscriber_count<E: 'static>(&self) -> usize {
        match self.buckets.get(&TypeId::of::<E>()) {
            None => 0,
            Some(Bucket::Single(_)) => 1,
            Some(Bucket::Many(list)) => list.len(),
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

// What `off` wants to do after it finishes walking the buckets. Separated
// from the walk to avoid holding a mutable borrow on `self.buckets` while
// mutating its shape.
enum OffAction {
    NotFound,
    // Removed from a `Many` that still has ≥ 2 subscribers; nothing else to do.
    Done,
    // The subscription was in a `Single`; remove the bucket entirely.
    DropBucket(TypeId),
    // Removed from a `Many` that now has exactly one; demote to `Single`.
    Demote(TypeId),
}

// Dispatch trampoline. Monomorphized once per `(E, F)` that appears in a
// call to `EventBus::on`.
//
// Safety: caller must ensure
//   1. `data` came from `Box::<F>::into_raw` and is still live (not yet
//      freed by `drop_trampoline`).
//   2. `event` is `&e as *const E as *const ()` for a live `&E` whose type
//      matches the `E` this trampoline was monomorphized against.
// Both established by `on_impl` + `emit` + the `TypeId` partitioning of
// `buckets`; proof in `docs/internal/trampoline.md` §4.
pub unsafe fn call_trampoline<E: 'static, F: Fn(&E)>(data: *const (), event: *const ()) {
    // SAFETY: precondition (1) makes `data as *const F` point to a live `F`.
    let f = unsafe { &*data.cast::<F>() };
    // SAFETY: precondition (2) makes `event as *const E` point to a live `E`.
    let e = unsafe { &*event.cast::<E>() };
    f(e);
}

// Destructor trampoline. Monomorphized once per `F`.
//
// Safety: caller must ensure
//   1. `data` came from `Box::<F>::into_raw` and is still live.
//   2. This function is called at most once for this `data`.
// Invoked only from `Subscriber::drop`, which Rust guarantees runs exactly
// once per `Subscriber`. Proof in `docs/internal/trampoline.md` §5.
pub unsafe fn drop_trampoline<F>(data: *const ()) {
    // SAFETY: by (1) and (2), the box is live and reclaimed exactly once.
    unsafe {
        drop(Box::from_raw(data.cast::<F>().cast_mut()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    struct PlayerDied {
        name: String,
    }

    struct Reload;

    #[test]
    fn emit_and_receive() {
        let mut bus = EventBus::new();
        let received = Rc::new(Cell::new(false));
        let r = received.clone();

        bus.on(move |_: &PlayerDied| {
            r.set(true);
        });

        bus.emit(PlayerDied {
            name: "test".into(),
        });
        assert!(received.get());
    }

    #[test]
    fn receives_correct_data() {
        let mut bus = EventBus::new();
        let name = Rc::new(Cell::new(String::new()));
        let n = name.clone();

        bus.on(move |e: &PlayerDied| {
            n.set(e.name.clone());
        });

        bus.emit(PlayerDied {
            name: "Orión".into(),
        });
        assert_eq!(name.take(), "Orión");
    }

    #[test]
    fn multiple_subscribers() {
        let mut bus = EventBus::new();
        let count = Rc::new(Cell::new(0u32));

        for _ in 0..5 {
            let c = count.clone();
            bus.on(move |_: &Reload| {
                c.set(c.get() + 1);
            });
        }

        bus.emit(Reload);
        assert_eq!(count.get(), 5);
    }

    #[test]
    fn unsubscribe() {
        let mut bus = EventBus::new();
        let count = Rc::new(Cell::new(0u32));
        let c = count.clone();

        let id = bus.on(move |_: &Reload| {
            c.set(c.get() + 1);
        });

        bus.emit(Reload);
        assert_eq!(count.get(), 1);

        assert!(bus.off(id));
        bus.emit(Reload);
        assert_eq!(count.get(), 1); // unchanged
    }

    #[test]
    fn unsubscribe_nonexistent_returns_false() {
        let mut bus = EventBus::new();
        assert!(!bus.off(SubscriptionId(999)));
    }

    #[test]
    fn different_types_are_independent() {
        let mut bus = EventBus::new();
        let died = Rc::new(Cell::new(false));
        let reloaded = Rc::new(Cell::new(false));

        let d = died.clone();
        bus.on(move |_: &PlayerDied| d.set(true));

        let r = reloaded.clone();
        bus.on(move |_: &Reload| r.set(true));

        bus.emit(Reload);
        assert!(!died.get());
        assert!(reloaded.get());
    }

    #[test]
    fn zst_signal() {
        let mut bus = EventBus::new();
        let fired = Rc::new(Cell::new(false));
        let f = fired.clone();

        bus.on(move |_: &Reload| f.set(true));
        bus.emit(Reload);

        assert!(fired.get());
    }

    #[test]
    fn emit_no_subscribers_is_noop() {
        let bus = EventBus::new();
        bus.emit(Reload); // should not panic
    }

    #[test]
    fn subscriber_count() {
        let mut bus = EventBus::new();
        assert_eq!(bus.subscriber_count::<Reload>(), 0);

        let id = bus.on(|_: &Reload| {});
        assert_eq!(bus.subscriber_count::<Reload>(), 1);

        bus.off(id);
        assert_eq!(bus.subscriber_count::<Reload>(), 0);
    }

    #[test]
    fn type_count() {
        let mut bus = EventBus::new();
        assert_eq!(bus.type_count(), 0);

        bus.on(|_: &Reload| {});
        assert_eq!(bus.type_count(), 1);

        bus.on(|_: &PlayerDied| {});
        assert_eq!(bus.type_count(), 2);
    }

    #[test]
    fn many_events_many_subscribers() {
        let mut bus = EventBus::new();
        let total = Rc::new(Cell::new(0u32));

        for _ in 0..100 {
            let t = total.clone();
            bus.on(move |_: &Reload| {
                t.set(t.get() + 1);
            });
        }

        for _ in 0..100 {
            bus.emit(Reload);
        }

        assert_eq!(total.get(), 10_000);
    }

    // Additional tests specific to the trampoline design: exercise drop paths
    // and closures that capture non-trivial state.

    #[test]
    fn closure_with_string_capture_drops_cleanly() {
        // If the drop trampoline is wrong (e.g. wrong F), this either leaks
        // the String or double-frees. Miri / valgrind would catch the latter;
        // a leak check catches the former by observing Rc refcount.
        let held = Rc::new(());
        let weak = Rc::downgrade(&held);

        {
            let mut bus = EventBus::new();
            let captured = held.clone();
            bus.on(move |_: &Reload| {
                let _ = &captured;
            });
            // bus drops here; the subscriber's Box<F> must drop, releasing `captured`.
        }

        drop(held);
        assert!(weak.upgrade().is_none(), "closure capture leaked");
    }

    #[test]
    fn unsubscribe_drops_closure() {
        let held = Rc::new(());
        let weak = Rc::downgrade(&held);

        let mut bus = EventBus::new();
        let captured = held.clone();
        let id = bus.on(move |_: &Reload| {
            let _ = &captured;
        });

        bus.off(id);
        drop(held);
        assert!(weak.upgrade().is_none(), "off() did not drop closure");
    }

    // Tests for the auto-specialization: Single → Many promotion and
    // Many → Single demotion. These are impl-visible boundaries and the
    // public behavior (subscriber_count, emit dispatch, closure lifecycle)
    // must stay invariant across them.

    #[test]
    fn promotes_single_to_many_on_second_subscribe() {
        let mut bus = EventBus::new();
        let a = Rc::new(Cell::new(false));
        let b = Rc::new(Cell::new(false));

        let ac = a.clone();
        bus.on(move |_: &Reload| ac.set(true)); // Single
        let bc = b.clone();
        bus.on(move |_: &Reload| bc.set(true)); // Single → Many

        bus.emit(Reload);
        assert!(a.get(), "first subscriber lost across promotion");
        assert!(b.get(), "second subscriber not delivered");
        assert_eq!(bus.subscriber_count::<Reload>(), 2);
    }

    #[test]
    fn demotes_many_to_single_on_off_leaving_one() {
        let mut bus = EventBus::new();
        let a = Rc::new(Cell::new(0u32));
        let b = Rc::new(Cell::new(0u32));

        let ac = a.clone();
        let id_a = bus.on(move |_: &Reload| ac.set(ac.get() + 1));
        let bc = b.clone();
        bus.on(move |_: &Reload| bc.set(bc.get() + 1));
        assert_eq!(bus.subscriber_count::<Reload>(), 2); // Many

        bus.off(id_a); // Many(2) → off → Single(b)
        assert_eq!(bus.subscriber_count::<Reload>(), 1);

        bus.emit(Reload);
        assert_eq!(a.get(), 0, "off'd subscriber still receiving");
        assert_eq!(b.get(), 1, "surviving subscriber dropped on demotion");
    }

    #[test]
    fn promotion_preserves_closure_state() {
        // Promotion moves the Subscriber from Single into a fresh Many without
        // running its destructor. If that goes wrong, the captured Rc leaks
        // or double-frees.
        let held = Rc::new(());
        let weak = Rc::downgrade(&held);

        {
            let mut bus = EventBus::new();
            let c = held.clone();
            bus.on(move |_: &Reload| {
                let _ = &c;
            });
            bus.on(|_: &Reload| {}); // trigger promotion
        }

        drop(held);
        assert!(weak.upgrade().is_none(), "promotion leaked closure capture");
    }

    #[test]
    fn off_back_to_zero_drops_bucket() {
        let mut bus = EventBus::new();
        let id = bus.on(|_: &Reload| {});
        assert_eq!(bus.type_count(), 1);

        bus.off(id);
        assert_eq!(bus.type_count(), 0, "empty bucket was not removed");
        assert_eq!(bus.subscriber_count::<Reload>(), 0);
    }
}
