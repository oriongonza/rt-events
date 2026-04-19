//! N=1 specialization.
//!
//! One event type, one handler. The subscriber lives inline in the
//! [`Bucket`](crate::eventbus::Bucket), so a lookup is `TypeId → Slot` with no
//! Vec in between. First `on` lands here; a second `on` for the same type
//! promotes this to [`many::List`](crate::many::List).

use crate::eventbus::{Subscriber, SubscriptionId};

// Inline single-subscriber storage. One `Subscriber`, nothing else.
pub struct Slot {
    sub: Subscriber,
}

impl Slot {
    pub const fn new(sub: Subscriber) -> Self {
        Self { sub }
    }

    pub fn matches(&self, id: SubscriptionId) -> bool {
        self.sub.id == id
    }

    // Decompose the slot. Used during Single → Many promotion to move the
    // existing `Subscriber` into a fresh `List` without running its destructor.
    pub fn into_inner(self) -> Subscriber {
        self.sub
    }

    // Dispatch to the one subscriber. Hot path for N=1; `#[inline]` so the
    // match in `EventBus::emit` folds into a single indirect call.
    //
    // # Safety
    // `event_ptr` must be `&e as *const E as *const ()` for a live `&E` whose
    // type matches the `E` this slot was registered against. Established by
    // the outer bus: buckets are partitioned by `TypeId`, so `emit::<E>` only
    // reaches a `Slot` built from `on::<E>`.
    #[inline]
    pub unsafe fn dispatch(&self, event_ptr: *const ()) {
        // SAFETY: by the bucket invariant above, `self.sub.call` is
        // `call_trampoline::<E, F>` for this exact `E`, and `self.sub.data`
        // is a live `Box::<F>::into_raw`. `event_ptr` points to a live `E`.
        unsafe { (self.sub.call)(self.sub.data, event_ptr) }
    }
}
