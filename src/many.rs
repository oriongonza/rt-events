//! N≥2 specialization.
//!
//! Vec of `Subscriber` records. Reached when a type accumulates a second
//! handler — the pre-existing [`single::Slot`](crate::single::Slot) is
//! demoted into a two-element list via [`List::from_two`]. Shrinks back to
//! `Single` when `off` leaves exactly one survivor; see
//! [`List::into_single`].

use crate::eventbus::{Subscriber, SubscriptionId};

// Invariant: `self.subs.len() >= 2` at every public API boundary.
// Transient len == 1 only exists inside `remove`, and the outer bus demotes
// to a `single::Slot` before returning from `off`.
pub struct List {
    subs: Vec<Subscriber>,
}

impl List {
    // Construct from the two subscribers that triggered promotion: the
    // existing Single plus the one that just arrived.
    pub fn from_two(first: Subscriber, second: Subscriber) -> Self {
        Self {
            subs: vec![first, second],
        }
    }

    pub fn push(&mut self, sub: Subscriber) {
        self.subs.push(sub);
    }

    pub fn len(&self) -> usize {
        self.subs.len()
    }

    // Remove by id. Returns whether anything was removed. Uses `swap_remove`:
    // dispatch order has no semantic meaning (see `docs/internal/formalism.md`).
    pub fn remove(&mut self, id: SubscriptionId) -> bool {
        if let Some(pos) = self.subs.iter().position(|s| s.id == id) {
            self.subs.swap_remove(pos);
            true
        } else {
            false
        }
    }

    // If only one subscriber remains, consume self and return it so the outer
    // bus can demote the bucket back to `single::Slot`.
    pub fn into_single(mut self) -> Option<Subscriber> {
        if self.subs.len() == 1 {
            self.subs.pop()
        } else {
            None
        }
    }

    // Dispatch to every subscriber in registration order. Hot path for N≥2;
    // `#[inline]` so the match in `EventBus::emit` folds into a tight loop
    // with no extra call frame.
    //
    // # Safety
    // `event_ptr` must be `&e as *const E as *const ()` for a live `&E` whose
    // type matches the `E` every `Subscriber` in this list was registered
    // against. Established by the outer bus: buckets are partitioned by
    // `TypeId`, so `emit::<E>` only reaches a `List` whose entries were all
    // built from `on::<E>`.
    #[inline]
    pub unsafe fn dispatch(&self, event_ptr: *const ()) {
        for sub in &self.subs {
            // SAFETY: per the bucket invariant above, `sub.call` is
            // `call_trampoline::<E, Fₛ>` for this exact `E`, and `sub.data`
            // is a live `Box::<Fₛ>::into_raw`. `event_ptr` points to a live `E`.
            unsafe { (sub.call)(sub.data, event_ptr) }
        }
    }
}
