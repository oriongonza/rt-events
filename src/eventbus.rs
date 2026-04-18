use std::any::TypeId;
use std::collections::HashMap;

/// Opaque handle returned by [`EventBus::on`]. Pass to [`EventBus::off`] to unsubscribe.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SubscriptionId(u64);

// Type-erased subscriber. `call` and `drop` are monomorphized fn pointers
// baked in at subscribe time for the concrete `(E, F)`, so dispatch is one
// indirect call with no vtable double-hop and no runtime downcast.
//
// Rationale, measured impact, safety proof, and the rejected SoA layout all
// live in `docs/internal/trampoline.md` (§2–§5 for safety, §8 for numbers,
// §10 for SoA).
struct Subscriber {
    // `Box::<F>::into_raw` for the closure; live until this `Subscriber` drops.
    data: *const (),
    // Trampoline for the exact `(E, F)` that produced `data`: casts `data` to
    // `&F`, event to `&E`, invokes `F`.
    call: unsafe fn(data: *const (), event: *const ()),
    id: SubscriptionId,
    // Destructor for the same `F`.
    drop: unsafe fn(data: *const ()),
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

/// A typed, sync, zero-dependency event bus.
//
// `Vec<Vec<Subscriber>>` indexed by event type.
// `TypeId` maps to a `Vec` index.
// Dispatch is O(subscribers).
// Subscribe is O(1) amortized.
// Unsubscribe is O(subscribers) for that event type.
pub struct EventBus {
    // Outer Vec: one entry per registered event type. Inner Vec: subscribers
    // for that type. Partitioned by `TypeId` — every subscriber at index `i`
    // was registered for the unique `E` with `type_index[TypeId::of::<E>()] == i`.
    subscribers: Vec<Vec<Subscriber>>,
    // TypeId → index into `subscribers`.
    type_index: HashMap<TypeId, usize>,
    // Monotonic counter for subscription IDs.
    next_id: u64,
}

impl EventBus {
    /// Create an empty bus.
    // Allocates nothing until the first subscription.
    pub fn new() -> Self {
        Self {
            subscribers: Vec::new(),
            type_index: HashMap::new(),
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

        let idx = self.index_of::<E>();

        self.subscribers[idx].push(Subscriber {
            data: Box::into_raw(Box::new(callback)) as *const (),
            call: call_trampoline::<E, F>,
            id,
            drop: drop_trampoline::<F>,
        });

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
        let type_id = TypeId::of::<E>();
        if let Some(&idx) = self.type_index.get(&type_id) {
            let event_ptr = &event as *const E as *const ();
            // Hot loop. On x86_64 this compiles to:
            //   mov    0x10(%r15), %rdi     ; sub.data
            //   mov    <event>, %rsi        ; &event (hoisted)
            //   call   *(%r15)              ; sub.call  — one indirect call,
            //                                            no vtable deref
            //   add    $0x20, %r15
            //   cmp    %end, %r15
            //   jne    loop
            // The indirect call + loop back-edge account for ~96% of dispatch
            // time; the rest is the load of `sub.data`. Measured impact vs
            // `Box<dyn Fn(&dyn Any)>`: `docs/internal/trampoline.md` §8.
            for sub in &self.subscribers[idx] {
                // SAFETY: by the Subscriber invariant (docs/internal/trampoline.md §2),
                // every entry in `subscribers[idx]` was pushed by `on_impl::<E, Fₛ>`
                // for this exact `E` (TypeId injectivity). So `sub.call` is
                // `call_trampoline::<E, Fₛ>` and `sub.data` is a live `Box<Fₛ>`.
                // `event_ptr` is `&event as *const E as *const ()` for a live `&E`.
                // The trampoline's three preconditions are met.
                unsafe { (sub.call)(sub.data, event_ptr) }
            }
        }
    }

    /// Remove a subscription. Returns `true` if the subscription was found.
    pub fn off(&mut self, id: SubscriptionId) -> bool {
        for subs in &mut self.subscribers {
            if let Some(pos) = subs.iter().position(|s| s.id == id) {
                // dispatch order has no semantic meaning (see `docs/internal/formalism.md`).
                subs.swap_remove(pos);
                return true;
            }
        }
        false
    }

    /// Number of registered event types.
    pub fn type_count(&self) -> usize {
        self.subscribers.len()
    }

    /// Number of subscribers for a given event type.
    pub fn subscriber_count<E: 'static>(&self) -> usize {
        let type_id = TypeId::of::<E>();
        self.type_index
            .get(&type_id)
            .map(|&idx| self.subscribers[idx].len())
            .unwrap_or(0)
    }

    // Resolve or allocate the Vec index for event type `E`.
    fn index_of<E: 'static>(&mut self) -> usize {
        let type_id = TypeId::of::<E>();
        if let Some(&idx) = self.type_index.get(&type_id) {
            return idx;
        }
        let idx = self.subscribers.len();
        self.subscribers.push(Vec::new());
        self.type_index.insert(type_id, idx);
        idx
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
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
// `subscribers`; proof in `docs/internal/trampoline.md` §4.
unsafe fn call_trampoline<E: 'static, F: Fn(&E)>(data: *const (), event: *const ()) {
    // SAFETY: precondition (1) makes `data as *const F` point to a live `F`.
    let f = unsafe { &*(data as *const F) };
    // SAFETY: precondition (2) makes `event as *const E` point to a live `E`.
    let e = unsafe { &*(event as *const E) };
    f(e);
}

// Destructor trampoline. Monomorphized once per `F`.
//
// Safety: caller must ensure
//   1. `data` came from `Box::<F>::into_raw` and is still live.
//   2. This function is called at most once for this `data`.
// Invoked only from `Subscriber::drop`, which Rust guarantees runs exactly
// once per `Subscriber`. Proof in `docs/internal/trampoline.md` §5.
unsafe fn drop_trampoline<F>(data: *const ()) {
    // SAFETY: by (1) and (2), the box is live and reclaimed exactly once.
    unsafe {
        drop(Box::from_raw(data as *mut F));
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
}
