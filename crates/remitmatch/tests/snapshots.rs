//! Regression lock: the exact result of `reconcile` on a fixed matrix of datasets and
//! configurations, pinned as 64-bit FNV-1a hashes of a canonical dump.
//!
//! The expected hashes were recorded from the engine before the performance work. A change
//! of any match, status, score bit, breakdown bit, residual or unmatched list changes the
//! hash of at least one case. Never edit a hash to make a test pass.

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::OnceLock;

use common::{Dataset, Noise, generate, generate_dense};
use remitmatch::{MatchConfig, MatchKind, MatchStatus, ReconciliationResult, reconcile};

/// A dataset of the matrix.
#[derive(Clone, Copy)]
enum Data {
    /// Realistic noise: seed and invoice count.
    Realistic(u64, usize),
    /// No noise: seed and invoice count.
    Clean(u64, usize),
    /// Dense, collision-heavy data: seed and invoice count.
    Dense(u64, usize),
}

impl Data {
    /// Returns a short label for the case name.
    fn label(self) -> String {
        match self {
            Self::Realistic(seed, n) => format!("real-s{seed}-n{n}"),
            Self::Clean(seed, n) => format!("clean-s{seed}-n{n}"),
            Self::Dense(seed, n) => format!("dense-s{seed}-n{n}"),
        }
    }

    /// Generates the dataset.
    fn build(self) -> Dataset {
        match self {
            Self::Realistic(seed, n) => generate(seed, n, &Noise::realistic()),
            Self::Clean(seed, n) => generate(seed, n, &Noise::none()),
            Self::Dense(seed, n) => generate_dense(seed, n),
        }
    }
}

/// A configuration of the matrix.
#[derive(Clone, Copy)]
enum Cfg {
    /// The RFC defaults.
    Default,
    /// Amount tolerance of 300 minor units.
    Tol300,
    /// No reference weight and lowered thresholds, so counterparty matches are produced.
    NoRef,
    /// A tolerance of 100,000 minor units.
    BigTol,
    /// Groups of at most 2.
    Group2,
    /// Groups disabled.
    Group0,
    /// No ambiguity margin.
    Margin0,
    /// A date window of 0 days.
    Win0,
    /// A date window of 5 days.
    Win5,
}

impl Cfg {
    /// Returns a short label for the case name.
    fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Tol300 => "tol300",
            Self::NoRef => "noref",
            Self::BigTol => "bigtol",
            Self::Group2 => "group2",
            Self::Group0 => "group0",
            Self::Margin0 => "margin0",
            Self::Win0 => "win0",
            Self::Win5 => "win5",
        }
    }

    /// Builds the configuration.
    fn build(self) -> MatchConfig {
        let base = MatchConfig::default();
        match self {
            Self::Default => base,
            Self::Tol300 => MatchConfig {
                amount_tolerance_minor: 300,
                ..base
            },
            Self::NoRef => MatchConfig {
                weights: [("amount", 0.5), ("name", 0.3), ("date", 0.2)]
                    .into_iter()
                    .map(|(k, v)| (k.to_owned(), v))
                    .collect(),
                suggest_threshold: 0.55,
                auto_threshold: 0.80,
                ..base
            },
            Self::BigTol => MatchConfig {
                amount_tolerance_minor: 100_000,
                ..base
            },
            Self::Group2 => MatchConfig {
                max_group_size: 2,
                ..base
            },
            Self::Group0 => MatchConfig {
                max_group_size: 0,
                ..base
            },
            Self::Margin0 => MatchConfig {
                ambiguity_margin: 0.0,
                ..base
            },
            Self::Win0 => MatchConfig {
                date_window_days: 0,
                ..base
            },
            Self::Win5 => MatchConfig {
                date_window_days: 5,
                ..base
            },
        }
    }
}

use Cfg::{BigTol, Default as Dflt, Group0, Group2, Margin0, NoRef, Tol300, Win0, Win5};
use Data::{Clean, Dense, Realistic};

/// The cases: dataset, configuration and the expected hash of the canonical dump.
const CASES: &[(Data, Cfg, u64)] = &[
    (Realistic(1, 50), Dflt, 0x18e5d7bd9d8cbc72),
    (Realistic(1, 50), Tol300, 0x298b3ad03c9b4c46),
    (Realistic(1, 50), NoRef, 0x358a7846ff9be03c),
    (Realistic(1, 50), BigTol, 0x8f45f87f3f959486),
    (Realistic(1, 50), Group2, 0x30580ea4328ee9bc),
    (Realistic(1, 50), Group0, 0x822d2cbc91298144),
    (Realistic(1, 50), Margin0, 0x18e5d7bd9d8cbc72),
    (Realistic(1, 50), Win0, 0xe030d264820ec6b9),
    (Realistic(1, 50), Win5, 0x2efe89570c0c6383),
    (Realistic(2, 200), Dflt, 0xc6d89f8e3b3c682d),
    (Realistic(2, 200), Tol300, 0x2d9b5722038447e3),
    (Realistic(2, 200), NoRef, 0x82ee2077f6b9c08d),
    (Realistic(2, 200), BigTol, 0x87fceaad055164ca),
    (Realistic(2, 200), Margin0, 0xc6d89f8e3b3c682d),
    (Realistic(3, 600), Dflt, 0x4a819c45b1c4b124),
    (Realistic(3, 600), Tol300, 0xe775812c54e244ba),
    (Realistic(3, 600), NoRef, 0xb7728a507efec140),
    (Realistic(3, 600), Group2, 0x9171ce137c44e58b),
    (Realistic(3, 600), Win5, 0x6540d7c3a344bf6e),
    (Realistic(4, 1500), Tol300, 0x2d6784656364a0f8),
    (Realistic(4, 1500), NoRef, 0xce6899cb2ea203fa),
    (Realistic(4, 1500), Group0, 0x23b40b79fafd7cf1),
    (Realistic(5, 3000), Dflt, 0xc0ebc7e1e3862365),
    (Realistic(5, 3000), Tol300, 0x6f055444e03dc012),
    (Clean(6, 100), Dflt, 0x24e2b73e1246fdd7),
    (Clean(6, 100), Tol300, 0x24e2b73e1246fdd7),
    (Clean(6, 100), NoRef, 0x34d94d65a481d844),
    (Clean(6, 100), Win0, 0xb5667ff40b94e1a6),
    (Clean(7, 800), Dflt, 0xda0d839fba69a7de),
    (Clean(7, 800), NoRef, 0x8861d86fbf0d7e4c),
    (Clean(7, 800), Group2, 0xda0d839fba69a7de),
    (Dense(1, 50), Dflt, 0xc99e970513a6c1b6),
    (Dense(1, 50), Tol300, 0x922e75435391bbac),
    (Dense(1, 50), NoRef, 0xc2acb8b07f05a780),
    (Dense(1, 50), BigTol, 0x9bd6c2136c3a76ea),
    (Dense(1, 50), Group2, 0xc99e970513a6c1b6),
    (Dense(1, 50), Group0, 0xcf1df19bfcb9faa7),
    (Dense(1, 50), Margin0, 0xc99e970513a6c1b6),
    (Dense(1, 50), Win0, 0x81b597fb1d78f017),
    (Dense(1, 50), Win5, 0xff42794c0c046656),
    (Dense(2, 150), Dflt, 0x5ab6b4ba9ccab29a),
    (Dense(2, 150), Tol300, 0x6f8e40b31f978a05),
    (Dense(2, 150), NoRef, 0x5ff4f96faf464284),
    (Dense(2, 150), BigTol, 0xf54e1530fe850c58),
    (Dense(2, 150), Group2, 0x5ab6b4ba9ccab29a),
    (Dense(2, 150), Group0, 0xf998b7758addfe5a),
    (Dense(2, 150), Margin0, 0x2fed5adff305324b),
    (Dense(2, 150), Win0, 0x731879effe959eba),
    (Dense(2, 150), Win5, 0xd2b56dc2b7b143ae),
    (Dense(3, 400), Dflt, 0x40c43da8d7ec9332),
    (Dense(3, 400), Tol300, 0xd7fdb2e8c509e98b),
    (Dense(3, 400), NoRef, 0x687c10328db29b4c),
    (Dense(3, 400), Margin0, 0x89a90a93c8c6f74c),
    (Dense(3, 400), Win5, 0x031de62357b963c5),
    (Dense(4, 1000), Dflt, 0x757842d5ab377f03),
    (Dense(4, 1000), NoRef, 0x0b39ff3a6a7939e9),
    (Dense(4, 1000), Win5, 0x5120b3626feb6847),
    (Dense(5, 2000), Dflt, 0x3558dad0e8dd95f1),
    (Dense(5, 2000), Win5, 0xd749b6119f71e407),
    (Dense(5, 2000), Win0, 0x03c5cd887f5cf058),
];

/// Hashes the bytes with 64-bit FNV-1a.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Writes the canonical text of a result: one line per match with every field as exact
/// values (floats as bit patterns), then the unmatched payment and invoice ids.
fn dump(result: &ReconciliationResult) -> String {
    let mut out = String::new();
    for m in &result.matches {
        let _ = write!(
            out,
            "{:?} {:?} P[{}] I[{}] score={:016x}",
            m.kind,
            m.status,
            m.payment_ids.join(","),
            m.invoice_ids.join(","),
            m.score.to_bits()
        );
        for (key, value) in &m.breakdown {
            let _ = write!(out, " {key}={:016x}", value.to_bits());
        }
        let _ = writeln!(out, " residual={}", m.residual_minor);
    }
    let _ = writeln!(
        out,
        "unmatched_payments [{}]",
        result.unmatched_payments.join(",")
    );
    let _ = writeln!(
        out,
        "unmatched_invoices [{}]",
        result.unmatched_invoices.join(",")
    );
    out
}

/// What one case produced.
struct Outcome {
    name: String,
    expected: u64,
    actual: u64,
    kinds: BTreeMap<String, usize>,
    statuses: BTreeMap<String, usize>,
}

/// Runs every case once and shares the outcomes between the tests.
fn outcomes() -> &'static [Outcome] {
    static OUTCOMES: OnceLock<Vec<Outcome>> = OnceLock::new();
    OUTCOMES.get_or_init(|| {
        CASES
            .iter()
            .map(|&(data, cfg, expected)| {
                let set = data.build();
                let result = reconcile(&set.invoices, &set.payments, &cfg.build()).unwrap();
                let mut kinds = BTreeMap::new();
                let mut statuses = BTreeMap::new();
                for m in &result.matches {
                    *kinds.entry(format!("{:?}", m.kind)).or_default() += 1;
                    *statuses.entry(format!("{:?}", m.status)).or_default() += 1;
                }
                Outcome {
                    name: format!("{}/{}", data.label(), cfg.label()),
                    expected,
                    actual: fnv1a(dump(&result).as_bytes()),
                    kinds,
                    statuses,
                }
            })
            .collect()
    })
}

#[test]
fn every_case_matches_its_recorded_hash() {
    let failed: Vec<String> = outcomes()
        .iter()
        .filter(|o| o.expected != o.actual)
        .map(|o| {
            format!(
                "{}: expected {:#018x}, got {:#018x}",
                o.name, o.expected, o.actual
            )
        })
        .collect();
    assert!(
        failed.is_empty(),
        "{} of {} snapshot cases changed:\n{}",
        failed.len(),
        CASES.len(),
        failed.join("\n")
    );
}

#[test]
fn the_matrix_covers_every_kind_and_status() {
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut statuses: BTreeMap<String, usize> = BTreeMap::new();
    for o in outcomes() {
        for (k, n) in &o.kinds {
            *kinds.entry(k.clone()).or_default() += n;
        }
        for (s, n) in &o.statuses {
            *statuses.entry(s.clone()).or_default() += n;
        }
    }
    println!("cases: {}", outcomes().len());
    println!("matches by kind: {kinds:?}");
    println!("matches by status: {statuses:?}");
    for kind in [
        MatchKind::OneToOne,
        MatchKind::OneToMany,
        MatchKind::ManyToOne,
    ] {
        let n = kinds.get(&format!("{kind:?}")).copied().unwrap_or(0);
        assert!(n >= MIN_PER_BUCKET, "kind {kind:?} appears only {n} times");
    }
    for status in [
        MatchStatus::Auto,
        MatchStatus::Suggested,
        MatchStatus::Ambiguous,
    ] {
        let n = statuses.get(&format!("{status:?}")).copied().unwrap_or(0);
        assert!(
            n >= MIN_PER_BUCKET,
            "status {status:?} appears only {n} times"
        );
    }
}

/// Fewest matches each kind and each status must have across the matrix.
const MIN_PER_BUCKET: usize = 50;
