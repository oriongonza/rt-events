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

## Properties

- **Zero dependencies.**
- **MSRV: 1.0.**
- **Build time: Xms.** *(fill in after benchmarking)*
- **~200 lines of code.** Read the whole thing in 10 minutes.
- **600+ lines of benchmarks.**

## Benchmarks

Run with:

```
cargo bench
```

Three suites, all using [Criterion](https://github.com/bheisler/criterion.rs):

**`benches/dispatch.rs`** — emit latency

| Group | What it measures |
|-------|-----------------|
| `emit_zst` | ZST event with 1 / 10 / 100 / 1 000 subscribers |
| `emit_small_payload` | 4-byte struct with 1 / 10 / 100 / 1 000 subscribers |
| `emit_large_payload` | Heap-allocated struct with 1 / 10 / 100 subscribers |
| `emit_no_subscribers` | Emit when nothing is listening |
| `emit_type_miss` | Emit a type nobody subscribed to (100 subs for a different type) |

**`benches/throughput.rs`** — sustained event rate

| Group | What it measures |
|-------|-----------------|
| `throughput_zst` | ZST bursts of 1 K – 1 M events, 10 subscribers |
| `throughput_payload` | Payload bursts of 1 K – 100 K events, 10 subscribers |
| `throughput_mixed_types` | 3 types in round-robin, 100 K events |

**`benches/subscribe.rs`** — subscribe / unsubscribe cost

| Group | What it measures |
|-------|-----------------|
| `subscribe_single` | First subscription on a fresh bus |
| `subscribe_nth` | Nth subscription with 0 / 10 / 100 / 1 000 already registered |
| `unsubscribe_first` | Remove the oldest handler from a bus of 10 / 100 / 1 000 |
| `unsubscribe_last` | Remove the newest handler from a bus of 10 / 100 / 1 000 |
| `eventbus_new` | Allocate a new `EventBus` |
| `emit_with_10_types` | Emit one type when 10 distinct types are registered |

## Design

- `T: 'static` is the only bound. No traits, no registration step.
- The type *is* the subscription key.
- Subscribers receive `&E`. Clone if you need owned data.
- Callbacks are sync. [Here's why, and how to work with async.](docs/async-patterns.md)
- Signals are ZST events: `struct Reload; bus.emit(Reload);`

## Do one thing and do it well.

## License

MIT OR Apache-2.0
