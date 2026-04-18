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
//! ## Why sync only
//!
//! Events should be fast. An async callback in the dispatch path blocks
//! every subscriber behind it. The correct pattern is: sync callback
//! receives event, enqueues work elsewhere, returns. Whether "elsewhere"
//! is a channel, a spawned task, or an atomic flag is your business.
//! The bus dispatches. You decide what to do with the data.

#![forbid(unsafe_code)]

mod eventbus;

pub use eventbus::{EventBus, SubscriptionId};
