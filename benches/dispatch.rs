use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rt_events::EventBus;

struct Tick;

struct Hit {
    damage: u32,
}

struct ChatMessage {
    sender: String,
    content: String,
}

fn bench_emit_zst(c: &mut Criterion) {
    let mut group = c.benchmark_group("emit_zst");

    for n_subscribers in [1, 10, 100, 1000] {
        let mut bus = EventBus::new();
        for _ in 0..n_subscribers {
            bus.on(|_: &Tick| {});
        }

        group.bench_with_input(
            BenchmarkId::from_parameter(n_subscribers),
            &n_subscribers,
            |b: &mut criterion::Bencher, _| {
                b.iter(|| bus.emit(black_box(Tick)));
            },
        );
    }

    group.finish();
}

fn bench_emit_small(c: &mut Criterion) {
    let mut group = c.benchmark_group("emit_small_payload");

    for n_subscribers in [1, 10, 100, 1000] {
        let mut bus = EventBus::new();
        for _ in 0..n_subscribers {
            bus.on(|e: &Hit| {
                black_box(e.damage);
            });
        }

        group.bench_with_input(
            BenchmarkId::from_parameter(n_subscribers),
            &n_subscribers,
            |b: &mut criterion::Bencher, _| {
                b.iter(|| bus.emit(black_box(Hit { damage: 42 })));
            },
        );
    }

    group.finish();
}

fn bench_emit_large(c: &mut Criterion) {
    let mut group = c.benchmark_group("emit_large_payload");

    for n_subscribers in [1, 10, 100] {
        let mut bus = EventBus::new();
        for _ in 0..n_subscribers {
            bus.on(|e: &ChatMessage| {
                black_box(&e.content);
            });
        }

        group.bench_with_input(
            BenchmarkId::from_parameter(n_subscribers),
            &n_subscribers,
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

fn bench_emit_no_subscribers(c: &mut Criterion) {
    let bus = EventBus::new();

    c.bench_function("emit_no_subscribers", |b: &mut criterion::Bencher| {
        b.iter(|| bus.emit(black_box(Tick)));
    });
}

fn bench_emit_miss(c: &mut Criterion) {
    let mut bus = EventBus::new();
    // Register subscribers for a DIFFERENT type
    for _ in 0..100 {
        bus.on(|_: &Hit| {});
    }

    c.bench_function("emit_type_miss", |b: &mut criterion::Bencher| {
        b.iter(|| bus.emit(black_box(Tick)));
    });
}

criterion_group!(
    benches,
    bench_emit_zst,
    bench_emit_small,
    bench_emit_large,
    bench_emit_no_subscribers,
    bench_emit_miss,
);
criterion_main!(benches);
