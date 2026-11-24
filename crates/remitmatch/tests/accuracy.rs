//! Accuracy of the engine against the synthetic generator's known solution.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{Dataset, Noise, generate};
use remitmatch::{Match, MatchConfig, MatchKind, MatchStatus, reconcile};

/// Payment ids and invoice ids of one group, each sorted ascending.
type Group = (Vec<String>, Vec<String>);

fn config() -> MatchConfig {
    MatchConfig {
        amount_tolerance_minor: 300,
        ..MatchConfig::default()
    }
}

/// Returns the match as a group with both id lists sorted.
fn group_of(m: &Match) -> Group {
    let (mut payments, mut invoices) = (m.payment_ids.clone(), m.invoice_ids.clone());
    payments.sort();
    invoices.sort();
    (payments, invoices)
}

/// Returns the kind a group of this shape has.
fn kind_of(group: &Group) -> MatchKind {
    match (group.0.len(), group.1.len()) {
        (1, 1) => MatchKind::OneToOne,
        (1, _) => MatchKind::OneToMany,
        _ => MatchKind::ManyToOne,
    }
}

const KINDS: [(MatchKind, &str); 3] = [
    (MatchKind::OneToOne, "1:1"),
    (MatchKind::OneToMany, "1:N"),
    (MatchKind::ManyToOne, "N:1"),
];

/// Counts of one kind of group.
#[derive(Default, Clone, Copy)]
struct KindStats {
    solution_groups: usize,
    matches: usize,
    correct: usize,
    auto: usize,
    wrong_auto: usize,
}

/// Counts accumulated over one or more datasets.
#[derive(Default)]
struct Stats {
    payments: usize,
    with_solution: usize,
    solution_groups: usize,
    auto: usize,
    suggested: usize,
    ambiguous: usize,
    unmatched: usize,
    correct: usize,
    correct_auto: usize,
    by_kind: BTreeMap<String, KindStats>,
}

impl Stats {
    fn matches(&self) -> usize {
        self.auto + self.suggested + self.ambiguous
    }

    fn kind(&mut self, kind: MatchKind) -> &mut KindStats {
        let label = KINDS.iter().find(|(k, _)| *k == kind).map_or("?", |k| k.1);
        self.by_kind.entry(label.to_owned()).or_default()
    }

    fn print(&self) {
        let ratio = |num: usize, den: usize| num as f64 / den.max(1) as f64;
        println!("payments total: {}", self.payments);
        println!("payments with a solution: {}", self.with_solution);
        println!("solution groups: {}", self.solution_groups);
        println!("auto: {}", self.auto);
        println!("suggested: {}", self.suggested);
        println!("ambiguous: {}", self.ambiguous);
        println!("unmatched payments: {}", self.unmatched);
        println!("auto rate: {:.4}", ratio(self.auto, self.payments));
        println!("auto precision: {:.4}", ratio(self.correct_auto, self.auto));
        println!(
            "overall precision: {:.4}",
            ratio(self.correct, self.matches())
        );
        println!("recall: {:.4}", ratio(self.correct, self.solution_groups));
        println!("wrong auto matches: {}", self.auto - self.correct_auto);
        for (_, label) in KINDS {
            let k = self.by_kind.get(label).copied().unwrap_or_default();
            println!(
                "kind {label}: solution groups {} | matches {} | correct {} | auto {} | wrong auto {} | precision {:.4} | recall {:.4}",
                k.solution_groups,
                k.matches,
                k.correct,
                k.auto,
                k.wrong_auto,
                ratio(k.correct, k.matches),
                ratio(k.correct, k.solution_groups),
            );
        }
    }
}

/// Describes a match with the rows it links, for failure output.
fn describe(data: &Dataset, m: &Match) -> String {
    let rows = |ids: &[String]| -> Vec<String> {
        ids.iter()
            .map(|id| {
                if let Some(p) = data.payments.iter().find(|p| &p.id == id) {
                    format!(
                        "{id} ref={:?} payer={:?} amount={}",
                        p.reference, p.counterparty, p.amount_minor
                    )
                } else if let Some(i) = data.invoices.iter().find(|i| &i.id == id) {
                    format!(
                        "{id} number={} name={:?} amount={}",
                        i.number, i.counterparty, i.amount_minor
                    )
                } else {
                    id.clone()
                }
            })
            .collect()
    };
    format!(
        "{:?} {:?} payments {:?} | invoices {:?} | {:?}",
        m.kind,
        m.status,
        rows(&m.payment_ids),
        rows(&m.invoice_ids),
        m.breakdown
    )
}

fn evaluate(data: &Dataset, stats: &mut Stats) {
    let result = reconcile(&data.invoices, &data.payments, &config()).unwrap();
    let solution: BTreeSet<&Group> = data.solution.iter().collect();
    stats.payments += data.payments.len();
    stats.with_solution += data.solution.iter().map(|g| g.0.len()).sum::<usize>();
    stats.solution_groups += data.solution.len();
    stats.unmatched += result.unmatched_payments.len();
    for group in &data.solution {
        stats.kind(kind_of(group)).solution_groups += 1;
    }
    for m in &result.matches {
        let ok = solution.contains(&group_of(m));
        stats.correct += usize::from(ok);
        let kind = stats.kind(m.kind);
        kind.matches += 1;
        kind.correct += usize::from(ok);
        match m.status {
            MatchStatus::Auto => {
                kind.auto += 1;
                kind.wrong_auto += usize::from(!ok);
                stats.auto += 1;
                stats.correct_auto += usize::from(ok);
                if !ok {
                    println!("WRONG AUTO: {}", describe(data, m));
                }
            }
            MatchStatus::Suggested => stats.suggested += 1,
            MatchStatus::Ambiguous => stats.ambiguous += 1,
        }
    }
}

/// Asserts that every match is a solution group and every solution group is matched.
fn assert_perfect(data: &Dataset) {
    let result = reconcile(&data.invoices, &data.payments, &config()).unwrap();
    let solution: BTreeSet<&Group> = data.solution.iter().collect();
    let found: BTreeSet<Group> = result.matches.iter().map(group_of).collect();
    let wrong: Vec<&Match> = result
        .matches
        .iter()
        .filter(|m| !solution.contains(&group_of(m)))
        .collect();
    let missing: Vec<&&Group> = solution.iter().filter(|g| !found.contains(**g)).collect();
    for m in wrong.iter().take(10) {
        println!("WRONG: {}", describe(data, m));
    }
    for g in missing.iter().take(10) {
        println!("MISSING: {g:?}");
    }
    assert!(wrong.is_empty(), "{} wrong matches", wrong.len());
    assert!(missing.is_empty(), "{} missing groups", missing.len());
}

#[test]
fn clean_data_is_matched_perfectly() {
    let data = generate(7, 500, &Noise::none());
    let paid: usize = data.solution.iter().map(|g| g.0.len()).sum();
    assert_eq!(paid, data.payments.len());
    assert!(
        data.solution
            .iter()
            .all(|g| g.0.len() == 1 && g.1.len() == 1)
    );
    assert_perfect(&data);
}

#[test]
fn clean_grouped_and_partial_data_is_matched_perfectly() {
    let noise = Noise {
        grouped: 0.3,
        partial: 0.2,
        ..Noise::none()
    };
    let data = generate(7, 500, &noise);
    assert!(data.solution.iter().any(|g| g.1.len() > 1), "no 1:N groups");
    assert!(data.solution.iter().any(|g| g.0.len() > 1), "no N:1 groups");
    assert_perfect(&data);
}

#[test]
fn realistic_noise_keeps_auto_precision_high() {
    let mut stats = Stats::default();
    for seed in [1, 2, 3] {
        evaluate(&generate(seed, 2000, &Noise::realistic()), &mut stats);
    }
    stats.print();
    assert!(stats.auto > 0, "no auto matches at all");
    let precision = stats.correct_auto as f64 / stats.auto as f64;
    assert!(precision >= 0.99, "auto precision {precision}");
    for (_, label) in KINDS {
        let k = stats.by_kind.get(label).copied().unwrap_or_default();
        assert!(k.auto > 0, "no auto matches of kind {label}");
        let precision = (k.auto - k.wrong_auto) as f64 / k.auto as f64;
        assert!(
            precision >= 0.99,
            "auto precision of kind {label}: {precision}"
        );
    }
}
