//! Hand-written golden cases for the 1:1 matching engine.

use std::collections::BTreeMap;

use chrono::{Days, NaiveDate};
use remitmatch::{
    Error, Invoice, MatchConfig, MatchKind, MatchStatus, Payment, ReconciliationResult, reconcile,
};

fn day(offset: u64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, 1).unwrap() + Days::new(offset)
}

fn inv(id: &str, number: &str, amount_minor: i64) -> Invoice {
    Invoice {
        id: id.to_owned(),
        number: number.to_owned(),
        amount_minor,
        currency: "EUR".to_owned(),
        issue_date: day(0),
        due_date: day(10),
        counterparty: "Acme S.L.".to_owned(),
        scope: "main".to_owned(),
    }
}

fn pay(id: &str, amount_minor: i64, reference: &str) -> Payment {
    Payment {
        id: id.to_owned(),
        amount_minor,
        currency: "EUR".to_owned(),
        value_date: day(10),
        reference: reference.to_owned(),
        counterparty: "ACME".to_owned(),
        scope: "main".to_owned(),
    }
}

fn weights(entries: &[(&str, f64)]) -> BTreeMap<String, f64> {
    entries.iter().map(|&(k, v)| (k.to_owned(), v)).collect()
}

/// Weights that need no reference signal to reach the suggest threshold.
fn no_reference_config() -> MatchConfig {
    MatchConfig {
        weights: weights(&[("amount", 0.6), ("name", 0.2), ("date", 0.2)]),
        ..MatchConfig::default()
    }
}

fn run(invoices: &[Invoice], payments: &[Payment], config: &MatchConfig) -> ReconciliationResult {
    reconcile(invoices, payments, config).unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn exact_reference_and_amount_is_auto_with_zero_residual() {
    let r = run(
        &[inv("i1", "INV-001", 10_000)],
        &[pay("p1", 10_000, "Payment INV-001")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    let m = &r.matches[0];
    assert_eq!(m.kind, MatchKind::OneToOne);
    assert_eq!(m.status, MatchStatus::Auto);
    assert_eq!(m.payment_ids, vec!["p1"]);
    assert_eq!(m.invoice_ids, vec!["i1"]);
    assert_eq!(m.residual_minor, 0);
    assert!(close(m.score, 1.0));
    let keys: Vec<&str> = m.breakdown.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["amount", "date", "name", "reference"]);
    assert!(r.unmatched_payments.is_empty() && r.unmatched_invoices.is_empty());
}

#[test]
fn reference_with_spaces_dashes_and_lowercase_still_matches() {
    let r = run(
        &[inv("i1", "INV-001", 10_000)],
        &[pay("p1", 10_000, "pago factura inv - 001 gracias")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    assert!(close(r.matches[0].breakdown["reference"], 1.0));
}

#[test]
fn bank_fee_within_tolerance_matches_with_negative_residual() {
    let config = MatchConfig {
        amount_tolerance_minor: 150,
        ..MatchConfig::default()
    };
    let r = run(
        &[inv("i1", "INV-001", 10_000)],
        &[pay("p1", 9_970, "INV-001")],
        &config,
    );
    assert_eq!(r.matches.len(), 1);
    let m = &r.matches[0];
    assert_eq!(m.residual_minor, -30);
    assert!(close(m.breakdown["amount"], 0.8));
    assert_eq!(m.status, MatchStatus::Auto);
}

#[test]
fn amount_outside_tolerance_leaves_both_sides_unmatched() {
    let config = MatchConfig {
        amount_tolerance_minor: 50,
        ..MatchConfig::default()
    };
    let r = run(
        &[inv("i1", "INV-001", 10_000)],
        &[pay("p1", 9_900, "INV-001")],
        &config,
    );
    assert!(r.matches.is_empty());
    assert_eq!(r.unmatched_payments, vec!["p1"]);
    assert_eq!(r.unmatched_invoices, vec!["i1"]);
}

#[test]
fn cross_currency_never_matches() {
    let invoice = Invoice {
        currency: "USD".to_owned(),
        ..inv("i1", "INV-001", 10_000)
    };
    let r = run(
        &[invoice],
        &[pay("p1", 10_000, "INV-001")],
        &MatchConfig::default(),
    );
    assert!(r.matches.is_empty());
    assert_eq!(r.unmatched_payments, vec!["p1"]);
    assert_eq!(r.unmatched_invoices, vec!["i1"]);
}

#[test]
fn cross_scope_never_matches() {
    let invoice = Invoice {
        scope: "other".to_owned(),
        ..inv("i1", "INV-001", 10_000)
    };
    let r = run(
        &[invoice],
        &[pay("p1", 10_000, "INV-001")],
        &MatchConfig::default(),
    );
    assert!(r.matches.is_empty());
    assert_eq!(r.unmatched_payments, vec!["p1"]);
}

#[test]
fn currency_is_compared_case_insensitively() {
    let payment = Payment {
        currency: "eur".to_owned(),
        ..pay("p1", 10_000, "INV-001")
    };
    let r = run(
        &[inv("i1", "INV-001", 10_000)],
        &[payment],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
}

#[test]
fn shorter_number_is_not_a_mention_inside_a_longer_one() {
    let r = run(
        &[inv("i1", "INV-1", 10_000), inv("i10", "INV-10", 10_000)],
        &[pay("p1", 10_000, "INV-10")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    let m = &r.matches[0];
    assert_eq!(m.invoice_ids, vec!["i10"]);
    assert_eq!(m.status, MatchStatus::Auto);
    assert_eq!(r.unmatched_invoices, vec!["i1"]);
}

#[test]
fn shorter_number_alone_is_never_an_exact_mention_inside_a_longer_one() {
    let r = run(
        &[inv("i1", "INV-1", 10_000)],
        &[pay("p1", 10_000, "INV-10")],
        &MatchConfig::default(),
    );
    assert!(r.matches.iter().all(|m| m.breakdown["reference"] < 1.0));
}

#[test]
fn number_followed_by_a_separated_digit_group_is_a_mention() {
    let r = run(
        &[inv("i1", "INV-1", 10_000)],
        &[pay("p1", 10_000, "INV-1 10")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    assert!(close(r.matches[0].breakdown["reference"], 1.0));
}

#[test]
fn misspelled_reference_yields_a_partial_reference_signal() {
    let r = run(
        &[inv("i1", "INV-2024001", 10_000)],
        &[pay("p1", 10_000, "INV-2024010")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    let reference = r.matches[0].breakdown["reference"];
    assert!(reference > 0.8 && reference < 1.0, "got {reference}");
}

#[test]
fn payment_mentioning_another_open_invoice_gets_no_fuzzy_reference() {
    let config = MatchConfig {
        weights: weights(&[("reference", 0.1), ("amount", 0.9)]),
        ..MatchConfig::default()
    };
    let r = run(
        &[inv("i1", "INV-2024001", 100), inv("i2", "INV-2024010", 200)],
        &[pay("p1", 100, "INV-2024010")],
        &config,
    );
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].invoice_ids, vec!["i1"]);
    assert!(close(r.matches[0].breakdown["reference"], 0.0));
}

#[test]
fn no_reference_stays_unmatched_under_default_weights() {
    let r = run(
        &[inv("i1", "INV-001", 10_000)],
        &[pay("p1", 10_000, "thanks")],
        &MatchConfig::default(),
    );
    assert!(r.matches.is_empty());
    assert_eq!(r.unmatched_payments, vec!["p1"]);
}

#[test]
fn no_reference_matches_under_custom_weights_via_the_candidate_pass() {
    let r = run(
        &[inv("i1", "INV-001", 10_000)],
        &[pay("p1", 10_000, "thanks")],
        &no_reference_config(),
    );
    assert_eq!(r.matches.len(), 1);
    assert!(close(r.matches[0].breakdown["reference"], 0.0));
    assert_eq!(r.matches[0].status, MatchStatus::Auto);
}

#[test]
fn date_window_filters_the_candidate_pass() {
    let far = Payment {
        value_date: day(10 + 40),
        ..pay("p1", 10_000, "thanks")
    };
    let near = Payment {
        value_date: day(10 + 10),
        ..pay("p2", 10_000, "thanks")
    };
    let config = no_reference_config();
    let invoices = [inv("i1", "INV-001", 10_000)];
    let r = run(&invoices, &[far], &config);
    assert!(r.matches.is_empty());
    let r = run(&invoices, &[near], &config);
    assert_eq!(r.matches.len(), 1);
}

#[test]
fn reference_pass_ignores_the_date_window() {
    let invoice = Invoice {
        due_date: day(200),
        ..inv("i1", "INV-001", 10_000)
    };
    let r = run(
        &[invoice],
        &[pay("p1", 10_000, "INV-001")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    let m = &r.matches[0];
    assert!(close(m.breakdown["date"], 0.0));
    // 0.45 + 0.30 + 0.15 sits on the threshold; the epsilon makes it Auto.
    assert!(close(m.score, 0.90));
    assert_eq!(m.status, MatchStatus::Auto);
}

#[test]
fn two_indistinguishable_invoices_give_an_ambiguous_match() {
    let r = run(
        &[inv("i1", "INV-7", 10_000), inv("i2", "INV-7", 10_000)],
        &[pay("p1", 10_000, "INV-7")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].status, MatchStatus::Ambiguous);
    assert_eq!(r.matches[0].invoice_ids, vec!["i1"]);
    assert_eq!(r.unmatched_invoices, vec!["i2"]);
}

#[test]
fn two_payments_for_one_invoice_give_one_match_and_one_unmatched() {
    let r = run(
        &[inv("i1", "INV-1", 10_000)],
        &[pay("p1", 10_000, "INV-1"), pay("p2", 10_000, "INV-1")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].payment_ids, vec!["p1"]);
    assert_eq!(r.unmatched_payments, vec!["p2"]);
}

#[test]
fn payment_mentioning_two_invoices_matches_the_one_whose_amount_fits() {
    let r = run(
        &[inv("i1", "INV-1", 10_000), inv("i2", "INV-2", 25_000)],
        &[pay("p1", 25_000, "INV-1 INV-2")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].invoice_ids, vec!["i2"]);
    assert_eq!(r.matches[0].status, MatchStatus::Auto);
    assert_eq!(r.unmatched_invoices, vec!["i1"]);
}

#[test]
fn accents_and_company_suffixes_are_ignored_in_names() {
    let invoice = Invoice {
        counterparty: "José Pérez S.L.".to_owned(),
        ..inv("i1", "INV-1", 10_000)
    };
    let payment = Payment {
        counterparty: "JOSE PEREZ".to_owned(),
        ..pay("p1", 10_000, "INV-1")
    };
    let r = run(&[invoice], &[payment], &MatchConfig::default());
    assert_eq!(r.matches.len(), 1);
    assert!(close(r.matches[0].breakdown["name"], 1.0));
}

#[test]
fn duplicate_ids_are_rejected() {
    let config = MatchConfig::default();
    let err = reconcile(
        &[inv("i1", "A-1", 100), inv("i1", "A-2", 200)],
        &[],
        &config,
    );
    assert_eq!(err, Err(Error::DuplicateInvoiceId("i1".to_owned())));
    let err = reconcile(&[], &[pay("p1", 100, "x"), pay("p1", 200, "y")], &config);
    assert_eq!(err, Err(Error::DuplicatePaymentId("p1".to_owned())));
}

#[test]
fn every_invalid_config_class_is_rejected_with_the_field_name() {
    let cases: Vec<(&str, MatchConfig)> = vec![
        (
            "auto_threshold",
            MatchConfig {
                auto_threshold: f64::NAN,
                ..MatchConfig::default()
            },
        ),
        (
            "auto_threshold",
            MatchConfig {
                auto_threshold: 1.5,
                ..MatchConfig::default()
            },
        ),
        (
            "suggest_threshold",
            MatchConfig {
                suggest_threshold: -0.1,
                ..MatchConfig::default()
            },
        ),
        (
            "suggest_threshold",
            MatchConfig {
                suggest_threshold: 0.95,
                ..MatchConfig::default()
            },
        ),
        (
            "ambiguity_margin",
            MatchConfig {
                ambiguity_margin: -0.01,
                ..MatchConfig::default()
            },
        ),
        (
            "ambiguity_margin",
            MatchConfig {
                ambiguity_margin: f64::INFINITY,
                ..MatchConfig::default()
            },
        ),
        (
            "amount_tolerance_minor",
            MatchConfig {
                amount_tolerance_minor: -1,
                ..MatchConfig::default()
            },
        ),
        (
            "weights",
            MatchConfig {
                weights: weights(&[("reference", 1.0), ("colour", 1.0)]),
                ..MatchConfig::default()
            },
        ),
        (
            "weights",
            MatchConfig {
                weights: weights(&[("reference", -1.0), ("amount", 2.0)]),
                ..MatchConfig::default()
            },
        ),
        (
            "weights",
            MatchConfig {
                weights: weights(&[("reference", f64::NAN)]),
                ..MatchConfig::default()
            },
        ),
        (
            "weights",
            MatchConfig {
                weights: weights(&[("reference", 0.0), ("amount", 0.0)]),
                ..MatchConfig::default()
            },
        ),
    ];
    for (field, config) in cases {
        match reconcile(&[], &[], &config) {
            Err(Error::InvalidConfig(message)) => {
                assert!(message.contains(field), "{message:?} should name {field}");
            }
            other => panic!("expected InvalidConfig for {field}, got {other:?}"),
        }
    }
}

#[test]
fn non_positive_amounts_stay_unmatched() {
    let r = run(
        &[inv("i0", "INV-0", 0), inv("i1", "INV-1", -100)],
        &[pay("p0", 0, "INV-0"), pay("p1", -100, "INV-1")],
        &MatchConfig::default(),
    );
    assert!(r.matches.is_empty());
    assert_eq!(r.unmatched_payments, vec!["p0", "p1"]);
    assert_eq!(r.unmatched_invoices, vec!["i0", "i1"]);
}

#[test]
fn empty_input_gives_an_empty_result() {
    let r = run(&[], &[], &MatchConfig::default());
    assert!(r.matches.is_empty());
    assert!(r.unmatched_payments.is_empty());
    assert!(r.unmatched_invoices.is_empty());
}

#[test]
fn matches_come_best_score_first_and_unmatched_keep_input_order() {
    let invoices = [
        inv("i1", "INV-2024001", 100),
        inv("i2", "INV-5000001", 200),
        inv("i3", "ZZZ-9", 300),
        inv("i4", "YYY-8", 400),
    ];
    let payments = [
        pay("p1", 100, "INV-2024010"),
        pay("p2", 200, "INV-5000001"),
        pay("p3", 999, "nothing"),
        pay("p4", 998, "nothing"),
    ];
    let r = run(&invoices, &payments, &MatchConfig::default());
    let ids: Vec<&str> = r
        .matches
        .iter()
        .map(|m| m.payment_ids[0].as_str())
        .collect();
    assert_eq!(ids, vec!["p2", "p1"]);
    assert!(r.matches[0].score > r.matches[1].score);
    assert_eq!(r.unmatched_payments, vec!["p3", "p4"]);
    assert_eq!(r.unmatched_invoices, vec!["i3", "i4"]);
}
