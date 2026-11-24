//! Accuracy of the 1:1 engine against the synthetic generator's known solution.

mod common;

use std::collections::BTreeMap;

use common::{Dataset, Noise, generate};
use remitmatch::{Invoice, MatchConfig, MatchStatus, Payment, reconcile};

fn config() -> MatchConfig {
    MatchConfig {
        amount_tolerance_minor: 300,
        ..MatchConfig::default()
    }
}

/// Counts accumulated over one or more datasets.
#[derive(Default)]
struct Stats {
    payments: usize,
    with_solution: usize,
    auto: usize,
    suggested: usize,
    ambiguous: usize,
    unmatched: usize,
    correct: usize,
    correct_auto: usize,
}

impl Stats {
    fn matches(&self) -> usize {
        self.auto + self.suggested + self.ambiguous
    }

    fn print(&self) {
        let ratio = |num: usize, den: usize| num as f64 / den.max(1) as f64;
        println!("payments total: {}", self.payments);
        println!("payments with a solution: {}", self.with_solution);
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
        println!("recall: {:.4}", ratio(self.correct, self.with_solution));
        println!("wrong auto matches: {}", self.auto - self.correct_auto);
    }
}

fn evaluate(data: &Dataset, stats: &mut Stats) {
    let result = reconcile(&data.invoices, &data.payments, &config()).unwrap();
    stats.payments += data.payments.len();
    stats.with_solution += data.solution.len();
    stats.unmatched += result.unmatched_payments.len();
    let invoices: BTreeMap<&str, &Invoice> =
        data.invoices.iter().map(|i| (i.id.as_str(), i)).collect();
    let payments: BTreeMap<&str, &Payment> =
        data.payments.iter().map(|p| (p.id.as_str(), p)).collect();
    for m in &result.matches {
        let (pid, iid) = (&m.payment_ids[0], &m.invoice_ids[0]);
        let ok = data.solution.get(pid) == Some(iid);
        stats.correct += usize::from(ok);
        match m.status {
            MatchStatus::Auto => {
                stats.auto += 1;
                stats.correct_auto += usize::from(ok);
                if !ok {
                    let (p, i) = (payments[pid.as_str()], invoices[iid.as_str()]);
                    println!(
                        "WRONG AUTO: ref={:?} payer={:?} amount={} | number={} name={:?} amount={} | {:?}",
                        p.reference,
                        p.counterparty,
                        p.amount_minor,
                        i.number,
                        i.counterparty,
                        i.amount_minor,
                        m.breakdown
                    );
                }
            }
            MatchStatus::Suggested => stats.suggested += 1,
            MatchStatus::Ambiguous => stats.ambiguous += 1,
        }
    }
}

#[test]
fn clean_data_is_matched_perfectly() {
    let data = generate(7, 500, &Noise::none());
    let result = reconcile(&data.invoices, &data.payments, &config()).unwrap();
    assert_eq!(data.solution.len(), data.payments.len());
    assert_eq!(result.matches.len(), data.payments.len());
    assert!(result.unmatched_payments.is_empty());
    for m in &result.matches {
        assert_eq!(
            data.solution.get(&m.payment_ids[0]),
            Some(&m.invoice_ids[0])
        );
    }
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
}
