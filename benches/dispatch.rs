//! `emit` microbenchmarks.
//!
//! Subscribers are swept as powers of two so the per-subscriber slope and
//! any cache-capacity cliff (L1D = 32 KB on this machine, ~1024 subs at
//! 32 B stride) are both visible in one chart.
//!
//! Every handler calls `black_box` on the event so no subscriber body can
//! be folded away — measuring the dispatch path, not the compiler's
//! ability to delete empty closures.
//!
//! Reproducibility: see `BENCHING.md`.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rt_events::EventBus;

struct Tick;

struct Hit {
    damage: u32,
}

struct ChatMessage {
    #[allow(dead_code, reason = "payload size contributor")]
    sender: String,
    content: String,
}

const SUB_COUNTS: &[usize] = &[1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024];
const LARGE_SUB_COUNTS: &[usize] = &[1, 2, 4, 8, 16, 32, 64, 128, 256];

fn bench_emit_zst(c: &mut Criterion) {
    let mut group = c.benchmark_group("emit_zst");

    for &n in SUB_COUNTS {
        let mut bus = EventBus::new();
        for _ in 0..n {
            bus.on(|e: &Tick| {
                black_box(e);
            });
        }

        group.bench_with_input(
            BenchmarkId::from_parameter(n),
            &n,
            |b: &mut criterion::Bencher, _| {
                b.iter(|| bus.emit(black_box(Tick)));
            },
        );
    }

    group.finish();
}

fn bench_emit_small(c: &mut Criterion) {
    let mut group = c.benchmark_group("emit_small_payload");

    for &n in SUB_COUNTS {
        let mut bus = EventBus::new();
        for _ in 0..n {
            bus.on(|e: &Hit| {
                black_box(e.damage);
            });
        }

        group.bench_with_input(
            BenchmarkId::from_parameter(n),
            &n,
            |b: &mut criterion::Bencher, _| {
                b.iter(|| bus.emit(black_box(Hit { damage: 42 })));
            },
        );
    }

    group.finish();
}

fn bench_emit_large(c: &mut Criterion) {
    let mut group = c.benchmark_group("emit_large_payload");

    for &n in LARGE_SUB_COUNTS {
        let mut bus = EventBus::new();
        for _ in 0..n {
            bus.on(|e: &ChatMessage| {
                black_box(&e.content);
            });
        }

        group.bench_with_input(
            BenchmarkId::from_parameter(n),
            &n,
            |b: &mut criterion::Bencher, _| {
                b.iter(|| {
                    bus.emit(black_box(ChatMessage {
                        sender: "alice".into(),
                        content: "hello world, this is a longer message to test larger payloads"
                            .into(),
                    }))
                });
            },
        );
    }

    group.finish();
}

// Empty bus: measures the HashMap<TypeId, _> lookup + miss branch. Floor
// for any emit that finds nothing to dispatch to.
fn bench_emit_no_subscribers(c: &mut Criterion) {
    let bus = EventBus::new();

    c.bench_function("emit_no_subscribers", |b: &mut criterion::Bencher| {
        b.iter(|| bus.emit(black_box(Tick)));
    });
}

// Populated bus, wrong type: same lookup cost as the no-subscribers case
// but with the TypeId distinct from anything registered. Measures the
// HashMap miss cost in a realistic populated-bus state.
fn bench_emit_miss(c: &mut Criterion) {
    let mut bus = EventBus::new();
    for _ in 0..100 {
        bus.on(|e: &Hit| {
            black_box(e.damage);
        });
    }

    c.bench_function("emit_type_miss", |b: &mut criterion::Bencher| {
        b.iter(|| bus.emit(black_box(Tick)));
    });
}

// First emit after subscribe — I-cache and D-cache cold. Uses
// `iter_batched` so setup cost doesn't bleed into the measurement.
fn bench_emit_cold(c: &mut Criterion) {
    let mut group = c.benchmark_group("emit_cold");

    for &n in &[1usize, 16, 256] {
        group.bench_with_input(
            BenchmarkId::from_parameter(n),
            &n,
            |b: &mut criterion::Bencher, &n| {
                b.iter_batched(
                    || {
                        let mut bus = EventBus::new();
                        for _ in 0..n {
                            bus.on(|e: &Tick| {
                                black_box(e);
                            });
                        }
                        bus
                    },
                    |bus| bus.emit(black_box(Tick)),
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_emit_zst,
    bench_emit_small,
    bench_emit_large,
    bench_emit_no_subscribers,
    bench_emit_miss,
    bench_emit_cold,
);
criterion_main!(benches);
