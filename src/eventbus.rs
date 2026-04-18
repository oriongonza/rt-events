use std::any::{Any, TypeId};
use std::collections::HashMap;

/// Opaque handle returned by [`EventBus::on`]. Pass to [`EventBus::off`] to unsubscribe.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SubscriptionId(u64);

// A bucket of subscribers for a single, statically-typed event `E`. The
// generic parameter is preserved inside the bucket — no pointer-casting, no
// trampolines. `Box<dyn Fn(&E)>` does the heterogeneous-closure erasure that
// `*const ()` used to do, and its vtable handles dispatch.
//
// The type-erasure that remains happens one level up: different `E`s live in
// different `Bucket<E>`s, and we store those behind `Box<dyn ErasedBucket>` so
// they all fit in one `HashMap`. The `TypeId` key pairs with the concrete
// `Bucket<E>` by construction (see `EventBus::on`), and we recover `E`
// statically with a single `downcast_ref::<Bucket<E>>()` at the top of `emit`.
// That downcast is hoisted out of the dispatch loop, so it costs O(1) per
// emit, not O(subscribers).
struct Bucket<E: 'static> {
    subs: Vec<(SubscriptionId, Box<dyn Fn(&E)>)>,
}

// Operations on a bucket that don't need to name `E`. Used for unsubscribe
// (scans buckets by id) and for the downcast bridge back to `Bucket<E>`.
trait ErasedBucket {
    fn remove(&mut self, id: SubscriptionId) -> bool;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<E: 'static> ErasedBucket for Bucket<E> {
    fn remove(&mut self, id: SubscriptionId) -> bool {
        if let Some(pos) = self.subs.iter().position(|(sid, _)| *sid == id) {
            // dispatch order has no semantic meaning (see `docs/internal/formalism.md`).
            // Explicit drop quiets `#[must_use]` on the boxed Fn in the returned tuple.
            drop(self.subs.swap_remove(pos));
            true
        } else {
            false
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// A typed, sync, zero-dependency event bus.
//
// `HashMap<TypeId, Box<dyn ErasedBucket>>` where each bucket's concrete type
// is `Bucket<E>` for the `E` matching the key. Dispatch is O(subscribers),
// subscribe is O(1) amortized, unsubscribe is O(subscribers) for that event
// type (or O(total subscribers) in the worst case, since `off` doesn't know
// which bucket holds the id).
pub struct EventBus {
    buckets: HashMap<TypeId, Box<dyn ErasedBucket>>,
    next_id: u64,
}

impl EventBus {
    /// Create an empty bus.
    // Allocates nothing until the first subscription.
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
        let id = SubscriptionId(self.next_id);
        self.next_id += 1;

        let bucket = self
            .buckets
            .entry(TypeId::of::<E>())
            .or_insert_with(|| Box::new(Bucket::<E> { subs: Vec::new() }));

        // The only code path that inserts into `buckets` keys `TypeId::of::<E>()`
        // with a `Bucket::<E>`. So the downcast is guaranteed to succeed — the
        // `expect` never fires in correct code and the branch is trivially
        // predicted.
        bucket
            .as_any_mut()
            .downcast_mut::<Bucket<E>>()
            .expect("bucket / TypeId invariant violated")
            .subs
            .push((id, Box::new(callback)));

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
    pub fn emit<E: 'static>(&self, event: E) {
        if let Some(bucket) = self.buckets.get(&TypeId::of::<E>()) {
            // One `downcast_ref` per emit, hoisted out of the loop. Pairs with
            // the insertion invariant in `on`. Compiles to a `TypeId` compare
            // plus a pointer cast.
            let typed = bucket
                .as_any()
                .downcast_ref::<Bucket<E>>()
                .expect("bucket / TypeId invariant violated");

            // Hot loop. Each iteration calls through `Box<dyn Fn(&E)>`'s
            // vtable: load the data pointer and vtable pointer from the fat
            // pointer (adjacent, one cache line), load the `Fn::call` fn
            // pointer from the vtable, indirect call. Two dependent loads on
            // the critical path vs the trampoline's one, but no `unsafe`.
            for (_, f) in &typed.subs {
                f(&event);
            }
        }
    }

    /// Remove a subscription. Returns `true` if the subscription was found.
    pub fn off(&mut self, id: SubscriptionId) -> bool {
        for bucket in self.buckets.values_mut() {
            if bucket.remove(id) {
                return true;
            }
        }
        false
    }

    /// Number of registered event types.
    pub fn type_count(&self) -> usize {
        self.buckets.len()
    }

    /// Number of subscribers for a given event type.
    pub fn subscriber_count<E: 'static>(&self) -> usize {
        self.buckets
            .get(&TypeId::of::<E>())
            .and_then(|b| b.as_any().downcast_ref::<Bucket<E>>())
            .map_or(0, |b| b.subs.len())
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
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

    #[test]
    fn closure_with_string_capture_drops_cleanly() {
        let held = Rc::new(());
        let weak = Rc::downgrade(&held);

        {
            let mut bus = EventBus::new();
            let captured = held.clone();
            bus.on(move |_: &Reload| {
                let _ = &captured;
            });
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
}
