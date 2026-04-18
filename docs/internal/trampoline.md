# Dispatch Trampolines

## 1. Setting

Subscribers in `EventBus::subscribers[i]` are stored type-erased — we don't
know their concrete `F: Fn(&E)` type at the dispatch site. The naive encoding
is `Box<dyn Fn(&dyn Any)>`, which forces every dispatch to pay for

- a vtable indirection (to reach `F` through the `Box`), and
- a runtime `downcast_ref::<E>()` (which itself does a vtable call to `Any::type_id`),

even though by the time we're iterating `subscribers[i]` the type is already
pinned down by the outer `TypeId` lookup.

We replace the `Box<dyn Fn>` with a pair `(data, call)`:

- `data: *const ()` — `Box::<F>::into_raw` for the concrete `F`, opaque to `EventBus`.
- `call: unsafe fn(*const (), *const ())` — a trampoline monomorphized at
  subscribe time for the exact `(E, F)`. It casts `data` back to `&F`, casts
  the event pointer back to `&E`, and invokes `F`.

Dispatch becomes one indirect call through a known fn pointer. No vtable
hop, no downcast.

## 2. The Invariant

Let `Sᵢ = EventBus::subscribers[i]`, and when `type_index[TypeId::of::<E>()] = i`
let `τ(i) = E`. By injectivity of `TypeId::of`, `τ` is a partial function.

**(I)** For every `i` in range and every `s ∈ Sᵢ`, there exists a closure
type `Fₛ : Fn(&τ(i)) + 'static` such that:

1. `s.data = Box::<Fₛ>::into_raw(_)` and the backing allocation is live.
2. `s.call = call_trampoline::<τ(i), Fₛ>`.
3. `s.drop = drop_trampoline::<Fₛ>`.

## 3. Establishing (I)

There are four operations that can affect `subscribers`. We show each
preserves (I).

**`on_impl::<E, F>(callback)`.** `index_of::<E>` returns the unique `i` with
`τ(i) = E` — reusing an existing entry or creating a fresh one and inserting
`TypeId::of::<E>() → i` into `type_index`. The push writes

    data = Box::<F>::into_raw(Box::new(callback)),
    call = call_trampoline::<E, F>,
    drop = drop_trampoline::<F>.

With `Fₛ = F` and `τ(i) = E`, (I.1)–(I.3) all hold on the new entry.
Existing entries are untouched.

**`off(id)`.** Only removes entries via `swap_remove`. The remaining entries
are unchanged, so (I) is preserved on them.

**`emit::<E>(event)`.** Reads only; does not mutate `subscribers` or
`type_index`.

**`Subscriber::drop`.** Consumes `data` via `drop_trampoline::<Fₛ>` and the
entry leaves the vec. (I) becomes vacuous for that entry.

Base case: `EventBus::new()` produces empty `subscribers`, so (I) holds
vacuously.

## 4. Dispatch is Sound

Claim. The `unsafe { (sub.call)(sub.data, event_ptr) }` in `emit::<E>`
satisfies `call_trampoline`'s preconditions.

Proof. Let `idx = type_index[TypeId::of::<E>()]`. By definition of `τ`,
`τ(idx) = E`. Pick any `sub ∈ subscribers[idx]`. By (I.2),
`sub.call = call_trampoline::<τ(idx), Fₛ> = call_trampoline::<E, Fₛ>`. So the
trampoline invoked is monomorphized with its `E` parameter equal to the `E`
in `emit::<E>`.

- **Precondition 1** (data is live `Box::<Fₛ>::into_raw`). By (I.1).
- **Precondition 2** (event is `&e as *const E as *const ()` for a live
  `&E`). `event_ptr = &event as *const E as *const ()` and `event` is in
  scope throughout the loop. ✓

Inside the trampoline:

    let f = &*(data as *const Fₛ);   // live Fₛ  (by P1)
    let e = &*(event as *const E);   // live E   (by P2)
    f(e);                             // Fₛ: Fn(&E), matches

All three steps are well-typed and the dereferences are valid. ∎

## 5. Destruction is Sound

Claim. `Subscriber::drop` satisfies `drop_trampoline`'s preconditions.

Proof. By (I.1), `self.data` is a live `Box::<Fₛ>::into_raw`, satisfying
precondition 1. Rust's Drop contract guarantees `Subscriber::drop` runs
exactly once per `Subscriber`, satisfying precondition 2. Inside the
trampoline, `Box::from_raw(data as *mut Fₛ)` reconstructs the original box
and drops it. ∎

No other code path calls `drop_trampoline`, so every registered closure is
dropped exactly once: either when its `Subscriber` leaves `subscribers`
(via `off` or the vec itself dropping), or when the `EventBus` drops and
the inner vecs drop.

## 6. Aliasing

`emit(&self)` takes `&self`. A callback receives `&E` via `&Fₛ`.

- If a callback re-invokes `bus.emit::<E'>(...)` — even with `E' = E` — the
  trait bound `Fₛ: Fn(&E)` requires only a shared reference to `Fₛ`, so
  multiple live `&Fₛ` on the same call stack are sound (standard Rust shared
  reference rules). No `&mut Fₛ` ever escapes.

- Subscribe (`on`) and unsubscribe (`off`) require `&mut self`, which cannot
  coexist with the `&self` held by a dispatch in progress. Subscribers
  cannot be added or removed mid-dispatch.

- `Subscriber` contains a raw pointer, so it is `!Send + !Sync` by default —
  matching the original `Box<dyn Fn>` semantics. The bus is single-threaded.

## 7. Panic Safety

If a callback panics, unwinding propagates through `call_trampoline` and out
of `emit`. `emit` held only `&self`, so no state is mid-mutation. `Box<Fₛ>`
allocations remain owned by their `Subscriber`s in the vec; they are freed
when the bus (or the `Subscriber`) is eventually dropped. No leak, no
double-free.

If a closure's own destructor panics during `drop_trampoline`, the panic
propagates into `Subscriber::drop`. Double-panic behaviour (abort) is the
same as with any `Box<F>` where `F::drop` panics — this matches the original
`Box<dyn Fn>` design.

## 8. Comparison

|                                   | `Box<dyn Fn(&dyn Any)>` | Trampoline |
|-----------------------------------|-------------------------|------------|
| Per-subscriber indirections       | 2 (Box vtable, then `Any::type_id` vtable inside `downcast_ref`) | 1 (fn pointer) |
| Runtime type check per call       | Yes                     | No         |
| `Subscriber` size                 | 24 B                    | 32 B       |
| Heap allocs per subscription      | 1                       | 1          |

The 8 B per-subscriber overhead is a constant; the eliminated indirection
and downcast are per-dispatch savings that scale with subscribers × emits.

### Measured

13th-gen Intel Core i7-1360P P-core, `cargo bench --bench dispatch`, same
binary pinned with `SCHED_RR` on core 6, 100 criterion samples per point,
median reported. Same-session A/B against the `Box<dyn Fn>` baseline via
`--save-baseline old` / `--baseline old`.

| subscribers | ZST event           | small payload      | large payload |
|-------------|---------------------|--------------------|---------------|
| 1           | 59 → 46 ns (noise)  | 65 → 61 ns (noise) | 126 → 114 ns (noise) |
| 10          | 122 → 56 ns  **−55%** | 141 → 78 ns **−50%** | 165 → 144 ns **−39%** |
| 100         | 895 → 290 ns **−58%** | 819 → 493 ns **−61%** | 796 → 427 ns (noise) |
| 1000        | 6.87 → 3.62 µs **−56%** | 8.65 → 4.37 µs **−29%** | — |

Miss path (`emit::<E>` with no subscribers for `E`, 46 ns) is unchanged —
it's dominated by the `HashMap<TypeId, _>` lookup (SipHash on 8 bytes),
which this change does not touch. Empty bus (`emit` with `type_index`
empty, ~4 ns) is also unchanged.

### What shifted in the profile

`perf record -F 4999 --call-graph fp --profile-time 10 emit_zst/1000`:

Before (`Box<dyn Fn(&dyn Any)>`):
- 43.7% `EventBus::emit::<Tick>` — dispatch loop
- 41.1% `on::{closure}` — the type-erasing wrapper (doing `downcast_ref`)
- 14.4% `<Tick as Any>::type_id` — vtable call inside the downcast

After (trampoline):
- 73.2% `EventBus::emit::<Tick>` — dispatch loop
- 25.3% `call_trampoline::<Tick, F>` — direct `F(&E)` call, no checks
-  0.9% HashMap hashing

The `Any::type_id` vtable call and the wrapper closure are gone entirely.
`emit`'s inner loop compiles to a single indirect call through the
subscriber's fn pointer plus a loop branch — the two together account for
~96% of dispatch time.

## 9. What Makes This Work

The trampoline pattern is sound because `EventBus::subscribers` is already
partitioned by `TypeId` — that partitioning is what `type_index` records.
The `downcast_ref` in the naive design is *redundant*: by the time we've
reached `subscribers[idx]` the type check has already happened implicitly
through the `TypeId` → index lookup. We pay for it once per `emit`, not
once per subscriber.

Rust's generics let us specialize a fn pointer on both `E` and the concrete
closure type `F`. Rust's ownership model guarantees the box lives as long
as its `Subscriber`. Together they turn what would be a C-style `void*` +
function pointer pattern into something the compiler type-checks end to
end, with the only remaining `unsafe` being the three pointer casts inside
the trampolines — each discharged by the invariant above.

## 10. Why Not SoA (Split Columns)

Natural next move after this change: split `Vec<Subscriber>` into parallel
`Vec<*const ()>` (`data`) and `Vec<unsafe fn>` (`call`), leaving `id` / `drop`
in cold columns. Two hot fields, densely packed, nothing else pulled into L1.
We measured it. It's ~25% slower.

Same hardware as §8, `emit_zst/1000` with 1000 subs × 2 M emits, `perf stat`,
3 runs each:

|                     | AoS     | SoA     | Δ      |
|---------------------|---------|---------|--------|
| cycles              | 8.14 G  | 10.16 G | +25%   |
| instructions        | 14.30 G | 14.35 G | ~same  |
| IPC                 | 1.56    | 1.22    | −22%   |
| L1-dcache misses    | 15.4 M  | 10.9 M  | **−29%** |

Criterion confirms at the wall-clock level: emit_zst/100 `151 → 178 ns (+17%)`,
emit_zst/1000 `1.36 → 1.51 µs (+11%)`, emit_large/100 `172 → 213 ns (+24%)`.
The regression holds from N=1 to N=1000.

The cache-locality theory was right — L1 misses drop 29% because `id` and
`drop` no longer get pulled into lines the dispatch loop doesn't read. But
at these working set sizes (≤16 KB across both hot columns) everything fits
in L1 anyway, so the miss-count win buys nothing. The IPC collapse is what
we actually feel.

Where the IPC goes: compare the hot loops under `perf annotate`:

    AoS:                                      SoA:
      mov    0x10(%r13), %rdi    3.4%          mov  0x0(%rbp,%r14,8), %rdi   24.7%
      mov    %r14, %rsi          —             mov  %r15, %rsi               —
      call   *0x0(%r13)         61.7%          call *(%rbx,%r14,8)          41.8%
      add    $0x20, %r13         —             inc  %r14                     —
      jne    loop               31.6%          jne  loop                    32.2%

Two mechanisms:

1. **Co-located load fusion.** In AoS, `sub.data` and `sub.call` live at
   offsets 0x10 and 0x0 of the same 32 B struct. Two architecturally distinct
   loads, but they hit the same cache line and often the same L1 bank in the
   same cycle — the CPU effectively folds them. The `mov 0x10(%r13), %rdi`
   costs only 3.4% of cycles because it overlaps with the indirect call's
   setup. In SoA, `data[i]` and `call[i]` are in two independent vecs, so
   the CPU can't fuse them — the data load alone now costs 24.7% of cycles.

2. **Dependency chain through indexed addressing.** `call *0x0(%r13)` is a
   simple displacement load; the address is ready as soon as `%r13` is. In
   SoA, `call *(%rbx,%r14,8)` requires an AGU op (`rbx + r14*8`) before the
   load can issue. Every iteration carries `%r14` forward, so the chain
   from `i` through `call[i]` through the indirect-call target is a cycle
   longer than AoS's simple pointer advance.

The lesson: cache-locality intuition is a real effect but it only helps when
you're actually cache-bound. At these working set sizes the dispatch loop is
dependency-chain bound through the indirect call, and AoS's struct layout
minimizes that chain. A layout choice that looks better on paper can lose to
one that the CPU's load pipeline is better at folding.
