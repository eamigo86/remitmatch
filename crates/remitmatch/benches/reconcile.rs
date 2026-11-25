//! Criterion benchmarks for `reconcile`.
//!
//! Every group measures one call on pre-generated data, so generation is never timed. The
//! sizes are the number of invoices; a dataset has about as many payments. The large cases
//! use few samples so the whole suite finishes in a few minutes.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use remitmatch::{MatchConfig, reconcile};

#[path = "../tests/common/mod.rs"]
mod common;

use common::{Dataset, Noise, Shape, generate_dense, generate_shaped};

/// The configuration used for the accuracy targets: a tolerance of 300 minor units.
fn tolerant() -> MatchConfig {
    MatchConfig {
        amount_tolerance_minor: 300,
        ..MatchConfig::default()
    }
}

/// Weights without `reference` and lowered thresholds, so counterparty passes produce matches.
fn no_reference() -> MatchConfig {
    MatchConfig {
        weights: [("amount", 0.5), ("name", 0.3), ("date", 0.2)]
            .into_iter()
            .map(|(signal, weight)| (signal.to_owned(), weight))
            .collect(),
        suggest_threshold: 0.55,
        auto_threshold: 0.80,
        ..tolerant()
    }
}

/// Generates realistic data with the given layout.
fn data(n_invoices: usize, shape: &Shape) -> Dataset {
    generate_shaped(1, n_invoices, &Noise::realistic(), shape)
}

/// Benchmarks `reconcile` on each size of one group.
///
/// `samples` and `seconds` bound the measurement of every size in the group.
fn bench_group(
    c: &mut Criterion,
    name: &str,
    sizes: &[usize],
    shape: &Shape,
    config: &MatchConfig,
    (samples, seconds): (usize, u64),
) {
    let mut group = c.benchmark_group(name);
    group
        .sample_size(samples)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(seconds));
    for &n in sizes {
        let set = data(n, shape);
        group.bench_with_input(BenchmarkId::from_parameter(n), &set, |b, set| {
            b.iter(|| {
                reconcile(black_box(&set.invoices), black_box(&set.payments), config).unwrap()
            });
        });
    }
    group.finish();
}

/// Realistic data in the default layout: two scopes and two currencies, four partitions.
fn realistic(c: &mut Criterion) {
    let config = tolerant();
    bench_group(
        c,
        "realistic",
        &[1_000, 10_000],
        &Shape::default(),
        &config,
        (20, 5),
    );
    bench_group(
        c,
        "realistic",
        &[100_000],
        &Shape::default(),
        &config,
        (10, 30),
    );
}

/// One scope and one currency, so a single partition holds every row; amounts vary.
fn single_scope_varied(c: &mut Criterion) {
    let shape = Shape {
        scopes: Some(1),
        single_currency: true,
        ..Shape::default()
    };
    bench_group(
        c,
        "single_scope_varied",
        &[5_000, 20_000],
        &shape,
        &tolerant(),
        (10, 10),
    );
}

/// One partition in which every invoice has the same amount, the quadratic worst case.
fn single_scope_same_amount(c: &mut Criterion) {
    let shape = Shape {
        scopes: Some(1),
        single_currency: true,
        distinct_amounts: Some(1),
    };
    bench_group(
        c,
        "single_scope_same_amount",
        &[1_000, 2_000, 4_000],
        &shape,
        &tolerant(),
        (10, 10),
    );
}

/// Many small partitions: 200 scopes in two currencies.
fn many_scopes(c: &mut Criterion) {
    let shape = Shape {
        scopes: Some(200),
        ..Shape::default()
    };
    bench_group(
        c,
        "many_scopes",
        &[10_000, 100_000],
        &shape,
        &tolerant(),
        (10, 30),
    );
}

/// Weights without a reference signal, so the counterparty group passes are active.
///
/// The single-scope case puts every row in one partition, where those passes dominate.
fn no_reference_weights(c: &mut Criterion) {
    let config = no_reference();
    bench_group(
        c,
        "no_reference_weights",
        &[1_000, 10_000],
        &Shape::default(),
        &config,
        (10, 10),
    );
    // A small tolerance and stricter thresholds leave many rows free for the group passes.
    let strict = MatchConfig {
        amount_tolerance_minor: 5,
        suggest_threshold: 0.6,
        auto_threshold: 0.9,
        ..no_reference()
    };
    let mut group = c.benchmark_group("no_reference_weights_single_scope");
    group
        .sample_size(10)
        .measurement_time(Duration::from_secs(20));
    for n in [5_000, 10_000] {
        let mut set = generate_dense(1, n);
        // One partition without references: few counterparty names and amounts, so the group
        // pools are large and the pool filters dominate.
        set.invoices.iter_mut().for_each(|i| {
            i.scope = "one".to_owned();
            i.currency = "EUR".to_owned();
        });
        set.payments.iter_mut().for_each(|p| {
            p.scope = "one".to_owned();
            p.currency = "EUR".to_owned();
            p.reference.clear();
        });
        group.bench_with_input(BenchmarkId::from_parameter(n), &set, |b, set| {
            b.iter(|| {
                reconcile(black_box(&set.invoices), black_box(&set.payments), &strict).unwrap()
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    realistic,
    single_scope_varied,
    single_scope_same_amount,
    many_scopes,
    no_reference_weights
);
criterion_main!(benches);
