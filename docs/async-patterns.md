# Working with async or blocking code

rt-events callbacks are sync by design. Blocking in the subscriber is generally an antipattern.
If your callback needs to do I/O, network calls, or anything that waits, it's too slow for the hot path. Move the work elsewhere and return immediately.

## Patterns

### Channel pattern

Send the event data to an async task via a channel. The callback
returns instantly. The async task processes at its own pace.

```rust
use std::sync::mpsc;

let (tx, rx) = mpsc::channel();

bus.on(move |e: &PlayerDied| {
    let _ = tx.send(e.name.clone());
});

// Elsewhere, in your async runtime:
// while let Ok(name) = rx.recv() { ... }
```

### Spawn Pattern

Spawn a new task with owned data. The callback clones what it needs
and hands it off.

```rust
bus.on(move |e: &PlayerDied| {
    let name = e.name.clone();
    tokio::spawn(async move {
        save_to_database(&name).await;
    });
});
```

### Flag pattern

Set an atomic flag or update shared state. An async loop picks it up
on its next iteration.

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

let needs_reload = Arc::new(AtomicBool::new(false));
let flag = needs_reload.clone();

bus.on(move |_: &ConfigChanged| {
    flag.store(true, Ordering::Release);
});

// Elsewhere:
// if needs_reload.load(Ordering::Acquire) { ... }
```
