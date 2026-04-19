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

## How it works

Each subscriber is a `(data, call)` pair, not a `Box<dyn Fn>`:

- `data: *const ()` — a `Box::<F>::into_raw` for the closure.
- `call: unsafe fn(*const (), *const ())` — a fn pointer monomorphized
  at subscribe time for the exact `(E, F)` pair.

`emit::<E>(e)` looks up the Vec of subscribers by `TypeId::of::<E>()`,
then loops `(sub.call)(sub.data, &e as *const _ as *const ())`. One
indirect call per subscriber through a known fn pointer — no vtable hop,
no `downcast_ref`, no `Any::type_id` recheck. The outer `TypeId` key
already proves what would otherwise need rechecking per subscriber.

The `unsafe` reduces to three pointer casts inside each trampoline,
discharged by a single invariant: for every subscriber in the bucket for
`E`, `data` is a live `Box::<F>::into_raw` and `call` is
`call_trampoline::<E, F>` for the same `F`. Established by `on()`,
preserved by `off()`, `emit()`, and drop. Full proof in
[`docs/internal/trampoline.md`](docs/internal/trampoline.md).

A fully-safe `HashMap<TypeId, Bucket<E>>` alternative using a hoisted
per-emit `downcast_ref` was measured and lost by 30–40% on ZST events,
5–10% on payloaded events. Notes in `trampoline.md` §11.

## Numbers

13th-gen i7-1360P, `taskset -c 6,7`, performance governor. 100 criterion
samples, median reported. See [`BENCHING.md`](BENCHING.md) to reproduce.

| subs  | ZST     | small struct | heap payload |
|------:|--------:|-------------:|-------------:|
|     1 | 15.7 ns |      14.6 ns |      35.0 ns |
|    16 | 35.8 ns |      32.4 ns |      53.1 ns |
|   128 |  228 ns |       219 ns |       212 ns |
|  1024 | 1.29 µs |      1.72 µs |            — |

Per-subscriber slope: **~1.25 ns** (ZST), **~1.67 ns** (small struct),
**~1.37 ns** (heap payload, extrapolated). The bucket's Vec of
`(data, call)` is 32 B/entry, 1024 subscribers = 32 KB = the i7-1360P's
L1D, so the curve bends slightly at that end.

Empty bus: **1.0 ns** (HashMap miss on an empty map).
Populated bus, wrong type: **12.2 ns**.
Peak throughput at 1024 subscribers: ~790M handler calls/sec.

`cargo bench` to reproduce.

Zero dependencies. MSRV 1.81. ~100 lines of library code, ~200 lines of
tests, ~400 lines of benchmarks.

`T: 'static` is the only bound. The type *is* the key. No traits, no registration.
Callbacks are sync. [Async patterns here.](docs/async-patterns.md)
Signals are ZST events: `struct Reload; bus.emit(Reload);`

## License

MIT OR Apache-2.0
