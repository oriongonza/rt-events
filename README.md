# rt-events

Microsecond pub/sub. A couple of Vecs. Four functions.

```rust
let mut bus = EventBus::new();

bus.on(|e: &PlayerDied| {
    println!("{} died", e.name);
});

bus.emit(PlayerDied { name: "Orión".into() });
```

## API

| Function | Description |
|----------|-------------|
| `EventBus::new()` | Create a bus. |
| `.on(callback)` | Subscribe. Returns a `SubscriptionId`. |
| `.emit(event)` | Dispatch to all subscribers of that type. |
| `.off(id)` | Unsubscribe. |

That's it.

## Numbers

| | 1 sub | 10 subs | 100 subs | 1 000 subs |
|---|---|---|---|---|
| ZST (`struct Tick;`) | 38 ns | 79 ns | 563 ns | 5.07 µs |
| Small payload | 30 ns | 83 ns | 583 ns | 5.85 µs |
| Large payload (heap) | 64 ns | 105 ns | 600 ns | — |

No subscribers: **2.8 ns**. Type miss: **25.7 ns**. Bus creation: **27.8 ns**.

Throughput (10 subs, ZST): **~13M emits/sec**.

`cargo bench` to reproduce.

## Properties

- **Zero dependencies.**
- **MSRV: 1.0.**
- **~200 lines of code.** Read the whole thing in 10 minutes.
- **600+ lines of benchmarks.**

## Design

- `T: 'static` is the only bound. No traits, no registration step.
- The type *is* the subscription key.
- Subscribers receive `&E`. Clone if you need owned data.
- Callbacks are sync. [Here's why, and how to work with async.](docs/async-patterns.md)
- Signals are ZST events: `struct Reload; bus.emit(Reload);`

## Do one thing and do it well.

## License

MIT OR Apache-2.0
