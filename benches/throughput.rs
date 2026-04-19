use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rt_events::EventBus;

struct Tick;

struct GameEvent {
    #[allow(dead_code, reason = "payload size contributor")]
    entity_id: u32,
    #[allow(dead_code, reason = "payload size contributor")]
    kind: u8,
    value: f32,
}

fn bench_throughput_zst(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput_zst");

    for n_events in [1_000, 10_000, 100_000, 1_000_000] {
        let mut bus = EventBus::new();
        for _ in 0..10 {
            bus.on(|e: &Tick| {
                black_box(e);
            });
        }

        group.throughput(Throughput::Elements(n_events));
        group.bench_with_input(
            BenchmarkId::from_parameter(n_events),
            &n_events,
            |b: &mut criterion::Bencher, &n| {
                b.iter(|| {
                    for _ in 0..n {
                        bus.emit(black_box(Tick));
                    }
                });
            },
        );
    }

    group.finish();
}

fn bench_throughput_payload(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput_payload");

    for n_events in [1_000, 10_000, 100_000] {
        let mut bus = EventBus::new();
        for _ in 0..10 {
            bus.on(|e: &GameEvent| {
                black_box(e.value);
            });
        }

        group.throughput(Throughput::Elements(n_events));
        group.bench_with_input(
            BenchmarkId::from_parameter(n_events),
            &n_events,
            |b: &mut criterion::Bencher, &n| {
                b.iter(|| {
                    for i in 0..n {
                        bus.emit(black_box(GameEvent {
                            entity_id: i as u32,
                            kind: 1,
                            value: 3.14,
                        }));
                    }
                });
            },
        );
    }

    group.finish();
}

fn bench_mixed_types_throughput(c: &mut Criterion) {
    struct TypeA;
    struct TypeB;
    struct TypeC;

    let mut bus = EventBus::new();
    for _ in 0..10 {
        bus.on(|e: &TypeA| {
            black_box(e);
        });
        bus.on(|e: &TypeB| {
            black_box(e);
        });
        bus.on(|e: &TypeC| {
            black_box(e);
        });
    }

    let n = 100_000u64;
    let mut group = c.benchmark_group("throughput_mixed_types");
    group.throughput(Throughput::Elements(n));

    group.bench_function("3_types_round_robin", |b: &mut criterion::Bencher| {
        b.iter(|| {
            for i in 0..n {
                match i % 3 {
                    0 => bus.emit(black_box(TypeA)),
                    1 => bus.emit(black_box(TypeB)),
                    _ => bus.emit(black_box(TypeC)),
                }
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_throughput_zst,
    bench_throughput_payload,
    bench_mixed_types_throughput,
);
criterion_main!(benches);
