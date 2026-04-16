use std::any::{Any, TypeId};
use std::collections::HashMap;

/// Opaque handle returned by [`EventBus::on`]. Pass to [`EventBus::off`] to unsubscribe.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SubscriptionId(u64);

struct Subscriber {
    id: SubscriptionId,
    callback: Box<dyn Fn(&dyn Any)>,
}

/// A typed, sync, zero-dependency event bus.
//
// `Vec<Vec<Subscriber>>` indexed by event type.
// `TypeId` maps to a `Vec` index.
// Dispatch is O(subscribers).
// Subscribe is O(1) amortized.
// Unsubscribe is O(subscribers) for that event type
pub struct EventBus {
    /// Outer Vec: one entry per registered event type.
    /// Inner Vec: subscribers for that type.
    subscribers: Vec<Vec<Subscriber>>,
    /// TypeId → index into `subscribers`.
    type_index: HashMap<TypeId, usize>,
    /// Monotonic counter for subscription IDs.
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
        let id = SubscriptionId(self.next_id);
        self.next_id += 1;

        let idx = self.index_of::<E>();

        self.subscribers[idx].push(Subscriber {
            id,
            callback: Box::new(move |any| {
                if let Some(event) = any.downcast_ref::<E>() {
                    callback(event);
                }
            }),
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
            for subscriber in &self.subscribers[idx] {
                (subscriber.callback)(&event);
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

    /// Resolve or allocate the Vec index for event type `E`.
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
}
