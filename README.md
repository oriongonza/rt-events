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
| ZST | 38 ns | 79 ns | 563 ns | 5.07 µs |
| Small struct | 30 ns | 83 ns | 583 ns | 5.85 µs |
| Heap payload | 64 ns | 105 ns | 600 ns | — |

Miss: **2.8 ns**. Wrong type: **25.7 ns**. ~13M emits/sec sustained.

`cargo bench` to reproduce.

Zero dependencies. MSRV 1.0. ~200 lines of code, 600+ lines of benchmarks.

`T: 'static` is the only bound. The type *is* the key. No traits, no registration.
Callbacks are sync. [Async patterns here.](docs/async-patterns.md)
Signals are ZST events: `struct Reload; bus.emit(Reload);`

## License

MIT OR Apache-2.0
