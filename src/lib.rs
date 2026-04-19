#![warn(
    clippy::pedantic,
    clippy::nursery,
    clippy::cargo,
    missing_docs,
    clippy::undocumented_unsafe_blocks
)]

//! # rt-events
//!
//! Microsecond pub/sub. A couple of Vecs. Four functions.
//!
//! ```
//! use rt_events::EventBus;
//!
//! struct PlayerDied { name: String }
//!
//! let mut bus = EventBus::new();
//!
//! bus.on(|e: &PlayerDied| {
//!     println!("{} died", e.name);
//! });
//!
//! bus.emit(PlayerDied { name: "Orión".into() });
//! ```
//!
//! ## Design
//!
//! - `T: 'static` is the only bound. No traits, no registration.
//! - The type *is* the subscription key (`TypeId` internally).
//! - Subscribers receive `&E`. If you need owned data, clone it yourself.
//! - Callbacks are sync. If you need async, enqueue work and return.
//!   See the [docs](docs/) for common patterns.
//! - Signals are just ZST events: `struct Reload; bus.emit(Reload);`
//!
//! ## Dispatch specializations
//!
//! Per-event-type storage auto-specializes by subscriber count. The
//! specialization is an impl detail — the public surface is one
//! [`EventBus`], one [`SubscriptionId`]. Each path lives in its own module:
//!
//! - **N = 0** — no entry in the type map. `emit` is a `HashMap` miss.
//! - **N = 1** — [`single`] stores the subscriber inline in the bucket;
//!   `emit` is one indirect call, no Vec indirection.
//! - **N ≥ 2** — [`many`] stores a Vec of subscribers; `emit` loops.
//!
//! Promotion (Single → Many) runs on the second `on` for a type. Demotion
//! (Many → Single) runs on `off` when only one subscriber remains. Both
//! preserve the surviving subscribers' closure state; see the tests in
//! [`eventbus`].
//!
//! ## Why sync only
//!
//! Events should be fast. An async callback in the dispatch path blocks
//! every subscriber behind it. The correct pattern is: sync callback
//! receives event, enqueues work elsewhere, returns. Whether "elsewhere"
//! is a channel, a spawned task, or an atomic flag is your business.
//! The bus dispatches. You decide what to do with the data.

pub(crate) mod eventbus;
pub(crate) mod many;
pub(crate) mod single;

pub use eventbus::{EventBus, SubscriptionId};
