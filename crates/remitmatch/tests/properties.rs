//! Property tests: structural invariants of `reconcile` on small adversarial inputs.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use chrono::{Days, NaiveDate};
use common::{Noise, generate};
use proptest::prelude::*;
use remitmatch::{
    Invoice, MatchConfig, MatchKind, MatchStatus, Payment, ReconciliationResult, reconcile,
};

const SCOPES: [&str; 2] = ["a", "b"];
const CURRENCIES: [&str; 3] = ["EUR", "eur", "USD"];
const NAMES: [&str; 4] = ["Acme S.L.", "ACME", "José Pérez", "Beta Corp"];
const NOISE_TOKENS: [&str; 5] = ["TRANSFER", "pago", "x1", "12", "-"];

fn day(offset: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, 1).unwrap() + Days::new(u64::from(offset))
}

type InvoiceRaw = (usize, usize, i64, usize, u32, usize);
type PaymentRaw = (usize, usize, i64, Vec<usize>, bool, u32, usize);

fn invoice_from(index: usize, raw: InvoiceRaw) -> Invoice {
    let (scope, currency, amount_minor, number, due, name) = raw;
    Invoice {
        id: format!("i{index}"),
        number: if number > 12 {
            String::new()
        } else {
            format!("INV-{number}")
        },
        amount_minor,
        currency: CURRENCIES[currency].to_owned(),
        issue_date: day(due),
        due_date: day(due),
        counterparty: NAMES[name].to_owned(),
        scope: SCOPES[scope].to_owned(),
    }
}

fn payment_from(index: usize, raw: PaymentRaw) -> Payment {
    let (scope, currency, amount_minor, tokens, glued, date, name) = raw;
    let parts: Vec<String> = tokens
        .into_iter()
        .map(|t| {
            if t <= 12 {
                format!("INV-{t}")
            } else {
                NOISE_TOKENS[t - 13].to_owned()
            }
        })
        .collect();
    Payment {
        id: format!("p{index}"),
        amount_minor,
        currency: CURRENCIES[currency].to_owned(),
        value_date: day(date),
        reference: parts.join(if glued { "" } else { " " }),
        counterparty: NAMES[name].to_owned(),
        scope: SCOPES[scope].to_owned(),
    }
}

fn config_strategy() -> impl Strategy<Value = MatchConfig> {
    let weights = prop_oneof![
        Just(None),
        (0.0..=1.0f64, 0.0..=1.0f64, 0.0..=1.0f64, 0.0..=1.0f64).prop_map(Some),
    ];
    (
        0..=5i64,
        0..=60u32,
        0.0..=1.0f64,
        0.0..=1.0f64,
        0.0..0.2f64,
        weights,
    )
        .prop_map(|(tolerance, window, a, b, margin, weights)| {
            let mut config = MatchConfig {
                amount_tolerance_minor: tolerance,
                date_window_days: window,
                auto_threshold: a.max(b),
                suggest_threshold: a.min(b),
                ambiguity_margin: margin,
                ..MatchConfig::default()
            };
            if let Some((r, am, n, d)) = weights {
                // The reference weight is shifted so the sum is always positive.
                config.weights = [
                    ("reference", r + 0.01),
                    ("amount", am),
                    ("name", n),
                    ("date", d),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect();
            }
            config
        })
}

fn input_strategy() -> impl Strategy<Value = (Vec<Invoice>, Vec<Payment>, MatchConfig)> {
    let invoice = (
        0..2usize,
        0..3usize,
        -2..=40i64,
        0..=13usize,
        0..90u32,
        0..4usize,
    );
    let payment = (
        0..2usize,
        0..3usize,
        -2..=40i64,
        prop::collection::vec(0..=17usize, 0..=3),
        any::<bool>(),
        0..90u32,
        0..4usize,
    );
    (
        prop::collection::vec(invoice, 0..=12),
        prop::collection::vec(payment, 0..=12),
        config_strategy(),
    )
        .prop_map(|(invoices, payments, config)| {
            (
                invoices
                    .into_iter()
                    .enumerate()
                    .map(|(i, r)| invoice_from(i, r))
                    .collect(),
                payments
                    .into_iter()
                    .enumerate()
                    .map(|(i, r)| payment_from(i, r))
                    .collect(),
                config,
            )
        })
}

/// Checks every structural invariant of one run.
fn check(
    invoices: &[Invoice],
    payments: &[Payment],
    config: &MatchConfig,
    result: &ReconciliationResult,
) -> Result<(), TestCaseError> {
    let invoice_by_id: BTreeMap<&str, &Invoice> =
        invoices.iter().map(|i| (i.id.as_str(), i)).collect();
    let payment_by_id: BTreeMap<&str, &Payment> =
        payments.iter().map(|p| (p.id.as_str(), p)).collect();
    let mut seen_invoices = BTreeSet::new();
    let mut seen_payments = BTreeSet::new();
    for m in &result.matches {
        prop_assert_eq!(m.kind, MatchKind::OneToOne);
        prop_assert_eq!(m.payment_ids.len(), 1);
        prop_assert_eq!(m.invoice_ids.len(), 1);
        let (pid, iid) = (m.payment_ids[0].as_str(), m.invoice_ids[0].as_str());
        prop_assert!(seen_payments.insert(pid), "payment {pid} matched twice");
        prop_assert!(seen_invoices.insert(iid), "invoice {iid} matched twice");
        let (p, i) = (payment_by_id[pid], invoice_by_id[iid]);
        prop_assert_eq!(&p.scope, &i.scope);
        prop_assert!(p.currency.eq_ignore_ascii_case(&i.currency));
        prop_assert!(p.amount_minor > 0 && i.amount_minor > 0);
        prop_assert!(
            p.amount_minor.abs_diff(i.amount_minor) <= config.amount_tolerance_minor as u64
        );
        prop_assert_eq!(m.residual_minor, p.amount_minor - i.amount_minor);
        let keys: Vec<&str> = m.breakdown.keys().map(String::as_str).collect();
        prop_assert_eq!(keys, vec!["amount", "date", "name", "reference"]);
        prop_assert!((0.0..=1.0).contains(&m.score));
        prop_assert!(m.breakdown.values().all(|v| (0.0..=1.0).contains(v)));
        prop_assert!(m.score >= config.suggest_threshold - 1e-9);
        if m.status == MatchStatus::Auto {
            prop_assert!(m.score >= config.auto_threshold - 1e-9);
        }
    }
    for id in &result.unmatched_payments {
        prop_assert!(
            seen_payments.insert(id.as_str()),
            "payment {id} both matched and unmatched"
        );
    }
    for id in &result.unmatched_invoices {
        prop_assert!(
            seen_invoices.insert(id.as_str()),
            "invoice {id} both matched and unmatched"
        );
    }
    prop_assert_eq!(seen_payments.len(), payments.len());
    prop_assert_eq!(seen_invoices.len(), invoices.len());
    prop_assert!(result.matches.windows(2).all(|w| w[0].score >= w[1].score));
    Ok(())
}

proptest! {
    // No persistence file: integration tests have no lib.rs for proptest to place it next to.
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn invariants_hold_on_adversarial_inputs((invoices, payments, config) in input_strategy()) {
        let result = reconcile(&invoices, &payments, &config);
        prop_assert!(result.is_ok(), "{result:?}");
        let result = result.unwrap();
        check(&invoices, &payments, &config, &result)?;
        prop_assert_eq!(Ok(result), reconcile(&invoices, &payments, &config));
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn invariants_hold_on_generated_datasets(seed in any::<u64>()) {
        let data = generate(seed, 60, &Noise::realistic());
        let config = MatchConfig { amount_tolerance_minor: 300, ..MatchConfig::default() };
        let result = reconcile(&data.invoices, &data.payments, &config).unwrap();
        check(&data.invoices, &data.payments, &config, &result)?;
    }
}
