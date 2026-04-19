use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rt_events::EventBus;

struct Event;

fn bench_subscribe(c: &mut Criterion) {
    c.bench_function("subscribe_single", |b: &mut criterion::Bencher| {
        b.iter_with_setup(
            EventBus::new,
            |mut bus: EventBus| {
                black_box(bus.on(|_: &Event| {}));
            },
        );
    });
}

fn bench_subscribe_many(c: &mut Criterion) {
    let mut group = c.benchmark_group("subscribe_nth");

    for existing in [0, 10, 100, 1000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(existing),
            &existing,
            |b: &mut criterion::Bencher, &existing| {
                b.iter_with_setup(
                    || {
                        let mut bus = EventBus::new();
                        for _ in 0..existing {
                            bus.on(|_: &Event| {});
                        }
                        bus
                    },
                    |mut bus: EventBus| {
                        black_box(bus.on(|_: &Event| {}));
                    },
                );
            },
        );
    }

    group.finish();
}

fn bench_unsubscribe_first(c: &mut Criterion) {
    let mut group = c.benchmark_group("unsubscribe_first");

    for n in [10, 100, 1000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b: &mut criterion::Bencher, &n| {
            b.iter_with_setup(
                || {
                    let mut bus = EventBus::new();
                    let first = bus.on(|_: &Event| {});
                    for _ in 1..n {
                        bus.on(|_: &Event| {});
                    }
                    (bus, first)
                },
                |(mut bus, id): (EventBus, _)| {
                    black_box(bus.off(id));
                },
            );
        });
    }

    group.finish();
}

fn bench_unsubscribe_last(c: &mut Criterion) {
    let mut group = c.benchmark_group("unsubscribe_last");

    for n in [10, 100, 1000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b: &mut criterion::Bencher, &n| {
            b.iter_with_setup(
                || {
                    let mut bus = EventBus::new();
                    let mut last = bus.on(|_: &Event| {});
                    for _ in 1..n {
                        last = bus.on(|_: &Event| {});
                    }
                    (bus, last)
                },
                |(mut bus, id): (EventBus, _)| {
                    black_box(bus.off(id));
                },
            );
        });
    }

    group.finish();
}

fn bench_new(c: &mut Criterion) {
    c.bench_function("eventbus_new", |b: &mut criterion::Bencher| {
        b.iter(|| black_box(EventBus::new()));
    });
}

fn bench_many_types(c: &mut Criterion) {
    // Measure overhead of many different event types registered
    struct E0;
    struct E1;
    struct E2;
    struct E3;
    struct E4;
    struct E5;
    struct E6;
    struct E7;
    struct E8;
    struct E9;

    let mut bus = EventBus::new();
    bus.on(|_: &E0| {});
    bus.on(|_: &E1| {});
    bus.on(|_: &E2| {});
    bus.on(|_: &E3| {});
    bus.on(|_: &E4| {});
    bus.on(|_: &E5| {});
    bus.on(|_: &E6| {});
    bus.on(|_: &E7| {});
    bus.on(|_: &E8| {});
    bus.on(|_: &E9| {});

    c.bench_function("emit_with_10_types_registered", |b: &mut criterion::Bencher| {
        b.iter(|| bus.emit(black_box(E5)));
    });
}

criterion_group!(
    benches,
    bench_subscribe,
    bench_subscribe_many,
    bench_unsubscribe_first,
    bench_unsubscribe_last,
    bench_new,
    bench_many_types,
);
criterion_main!(benches);
