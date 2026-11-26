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
    let keys: Vec<&str> = m.breakdown.iter().map(|(name, _)| name).collect();
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
    assert!(close(r.matches[0].breakdown.reference, 1.0));
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
    assert!(close(m.breakdown.amount, 0.8));
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
fn shorter_number_alone_gets_no_reference_score_inside_a_longer_one() {
    let r = run(
        &[inv("i1", "INV-1", 10_000)],
        &[pay("p1", 10_000, "INV-10")],
        &no_reference_config(),
    );
    assert_eq!(r.matches.len(), 1);
    assert!(close(r.matches[0].breakdown.reference, 0.0));
    let r = run(
        &[inv("i1", "INV-1", 10_000)],
        &[pay("p1", 10_000, "INV-10")],
        &MatchConfig::default(),
    );
    assert!(r.matches.is_empty());
}

#[test]
fn number_glued_to_other_digits_stays_unmatched_under_default_weights() {
    let r = run(
        &[inv("i1", "1001", 10_000)],
        &[pay("p1", 10_000, "PAGO FRA 10010")],
        &MatchConfig::default(),
    );
    assert!(r.matches.is_empty());
    assert_eq!(r.unmatched_payments, vec!["p1"]);
    assert_eq!(r.unmatched_invoices, vec!["i1"]);
}

#[test]
fn number_followed_by_a_separated_digit_group_is_a_mention() {
    let r = run(
        &[inv("i1", "INV-1", 10_000)],
        &[pay("p1", 10_000, "INV-1 10")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    assert!(close(r.matches[0].breakdown.reference, 1.0));
}

#[test]
fn misspelled_reference_yields_a_partial_reference_signal() {
    let r = run(
        &[inv("i1", "INV-2024001", 10_000)],
        &[pay("p1", 10_000, "INV-2024010")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    let reference = r.matches[0].breakdown.reference;
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
    assert!(close(r.matches[0].breakdown.reference, 0.0));
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
    assert!(close(r.matches[0].breakdown.reference, 0.0));
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
    assert!(close(m.breakdown.date, 0.0));
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
    assert!(close(r.matches[0].breakdown.name, 1.0));
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

/// Two payments compete for `i1`; `p2` also mentions `i2`, which is due 20 days later and
/// therefore scores about 0.063 lower. `p1` wins `i1`, so `p2` gets `i2`.
fn contested_invoice(config: &MatchConfig) -> ReconciliationResult {
    let late = Invoice {
        due_date: day(30),
        ..inv("i2", "INV-2", 10_000)
    };
    run(
        &[inv("i1", "INV-1", 10_000), late],
        &[pay("p1", 10_000, "INV-1"), pay("p2", 10_000, "INV-1 INV-2")],
        config,
    )
}

fn status_of(r: &ReconciliationResult, payment_id: &str) -> MatchStatus {
    r.matches
        .iter()
        .find(|m| m.payment_ids[0] == payment_id)
        .unwrap_or_else(|| panic!("{payment_id} is not matched"))
        .status
}

#[test]
fn zero_margin_never_yields_ambiguous() {
    let config = MatchConfig {
        ambiguity_margin: 0.0,
        ..MatchConfig::default()
    };
    let r = contested_invoice(&config);
    assert_eq!(r.matches.len(), 2);
    assert!(r.matches.iter().all(|m| m.status != MatchStatus::Ambiguous));
    let tie = run(
        &[inv("i1", "INV-7", 10_000), inv("i2", "INV-7", 10_000)],
        &[pay("p1", 10_000, "INV-7")],
        &config,
    );
    assert_eq!(tie.matches[0].status, MatchStatus::Auto);
}

#[test]
fn taken_best_candidate_with_a_large_gap_is_not_ambiguous() {
    let r = contested_invoice(&MatchConfig::default());
    assert_eq!(r.matches.len(), 2);
    let p2 = r.matches.iter().find(|m| m.payment_ids[0] == "p2").unwrap();
    assert_eq!(p2.invoice_ids, vec!["i2"]);
    assert_eq!(status_of(&r, "p2"), MatchStatus::Auto);
}

#[test]
fn candidates_closer_than_the_margin_stay_ambiguous() {
    let config = MatchConfig {
        ambiguity_margin: 0.1,
        ..MatchConfig::default()
    };
    let r = contested_invoice(&config);
    assert_eq!(status_of(&r, "p2"), MatchStatus::Ambiguous);
}

fn breakdown_keys(m: &remitmatch::Match) -> Vec<&str> {
    m.breakdown.iter().map(|(name, _)| name).collect()
}

#[test]
fn payment_naming_two_invoices_is_one_to_many_in_input_order() {
    let r = run(
        &[inv("i1", "INV-1", 6_000), inv("i2", "INV-2", 4_000)],
        &[pay("p1", 10_000, "INV-2 INV-1")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    let m = &r.matches[0];
    assert_eq!(m.kind, MatchKind::OneToMany);
    assert_eq!(m.status, MatchStatus::Auto);
    assert_eq!(m.payment_ids, vec!["p1"]);
    assert_eq!(m.invoice_ids, vec!["i1", "i2"]);
    assert_eq!(m.residual_minor, 0);
    assert_eq!(
        breakdown_keys(m),
        vec!["amount", "date", "name", "reference"]
    );
    assert!(close(m.breakdown.reference, 1.0));
    assert!(close(m.score, 1.0));
    assert!(r.unmatched_payments.is_empty() && r.unmatched_invoices.is_empty());
}

#[test]
fn group_fee_within_tolerance_gives_a_negative_residual() {
    let config = MatchConfig {
        amount_tolerance_minor: 150,
        ..MatchConfig::default()
    };
    let r = run(
        &[inv("i1", "INV-1", 6_000), inv("i2", "INV-2", 4_000)],
        &[pay("p1", 9_970, "INV-1 INV-2")],
        &config,
    );
    assert_eq!(r.matches.len(), 1);
    let m = &r.matches[0];
    assert_eq!(m.kind, MatchKind::OneToMany);
    assert_eq!(m.residual_minor, -30);
    assert!(close(m.breakdown.amount, 0.8));
    assert_eq!(m.status, MatchStatus::Auto);
}

#[test]
fn three_mentioned_invoices_where_only_two_add_up_match_those_two() {
    let r = run(
        &[
            inv("i1", "INV-1", 6_000),
            inv("i2", "INV-2", 4_000),
            inv("i3", "INV-3", 7_000),
        ],
        &[pay("p1", 10_000, "INV-1 INV-2 INV-3")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].invoice_ids, vec!["i1", "i2"]);
    assert_eq!(r.matches[0].status, MatchStatus::Auto);
    assert_eq!(r.unmatched_invoices, vec!["i3"]);
}

#[test]
fn two_payments_citing_one_invoice_whose_sum_fits_are_many_to_one() {
    let r = run(
        &[inv("i1", "INV-1", 10_000)],
        &[pay("p1", 4_000, "INV-1"), pay("p2", 6_000, "INV-1")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    let m = &r.matches[0];
    assert_eq!(m.kind, MatchKind::ManyToOne);
    assert_eq!(m.status, MatchStatus::Auto);
    assert_eq!(m.payment_ids, vec!["p1", "p2"]);
    assert_eq!(m.invoice_ids, vec!["i1"]);
    assert_eq!(m.residual_minor, 0);
    assert!(close(m.breakdown.reference, 1.0));
    assert!(r.unmatched_payments.is_empty() && r.unmatched_invoices.is_empty());
}

#[test]
fn partial_payments_that_do_not_add_up_stay_unmatched() {
    let r = run(
        &[inv("i1", "INV-1", 10_000)],
        &[
            pay("p1", 3_000, "INV-1"),
            pay("p2", 3_000, "INV-1"),
            pay("p3", 3_000, "INV-1"),
        ],
        &MatchConfig::default(),
    );
    assert!(r.matches.is_empty());
    assert_eq!(r.unmatched_payments, vec!["p1", "p2", "p3"]);
    assert_eq!(r.unmatched_invoices, vec!["i1"]);
}

#[test]
fn identical_mentioned_invoices_make_the_group_ambiguous() {
    let r = run(
        &[
            inv("i1", "INV-7", 5_000),
            inv("i2", "INV-7", 5_000),
            inv("i3", "INV-7", 5_000),
        ],
        &[pay("p1", 10_000, "INV-7")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    let m = &r.matches[0];
    assert_eq!(m.kind, MatchKind::OneToMany);
    assert_eq!(m.status, MatchStatus::Ambiguous);
    assert_eq!(m.invoice_ids, vec!["i1", "i2"]);
    assert_eq!(r.unmatched_invoices, vec!["i3"]);
}

#[test]
fn several_valid_subsets_are_ambiguous_even_with_zero_margin() {
    // Different amounts, so the two best candidates differ in score only through the
    // amount signal; the group rule alone must flag the match.
    let config = MatchConfig {
        ambiguity_margin: 0.0,
        ..MatchConfig::default()
    };
    let r = run(
        &[
            inv("i1", "INV-1", 6_000),
            inv("i2", "INV-2", 4_000),
            inv("i3", "INV-3", 3_000),
            inv("i4", "INV-4", 7_000),
        ],
        &[pay("p1", 10_000, "INV-1 INV-2 INV-3 INV-4")],
        &config,
    );
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].status, MatchStatus::Ambiguous);
}

#[test]
fn max_group_size_two_blocks_a_group_of_three() {
    let invoices = [
        inv("i1", "INV-1", 3_000),
        inv("i2", "INV-2", 3_000),
        inv("i3", "INV-3", 4_000),
    ];
    let payments = [pay("p1", 10_000, "INV-1 INV-2 INV-3")];
    let allowed = MatchConfig {
        max_group_size: 3,
        ..MatchConfig::default()
    };
    let r = run(&invoices, &payments, &allowed);
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].invoice_ids, vec!["i1", "i2", "i3"]);
    let blocked = MatchConfig {
        max_group_size: 2,
        ..MatchConfig::default()
    };
    let r = run(&invoices, &payments, &blocked);
    assert!(r.matches.is_empty());
    assert_eq!(r.unmatched_payments, vec!["p1"]);
}

#[test]
fn max_group_size_below_two_disables_groups_but_not_one_to_one() {
    let invoices = [
        inv("i1", "INV-1", 6_000),
        inv("i2", "INV-2", 4_000),
        inv("i3", "INV-3", 5_000),
    ];
    let payments = [pay("p1", 10_000, "INV-1 INV-2"), pay("p2", 5_000, "INV-3")];
    for max_group_size in [0, 1] {
        let config = MatchConfig {
            max_group_size,
            ..MatchConfig::default()
        };
        let r = run(&invoices, &payments, &config);
        assert_eq!(r.matches.len(), 1, "max_group_size {max_group_size}");
        assert_eq!(r.matches[0].kind, MatchKind::OneToOne);
        assert_eq!(r.matches[0].payment_ids, vec!["p2"]);
        assert_eq!(r.unmatched_payments, vec!["p1"]);
        assert_eq!(r.unmatched_invoices, vec!["i1", "i2"]);
    }
}

#[test]
fn a_group_never_crosses_scope_or_currency() {
    let other_scope = Invoice {
        scope: "other".to_owned(),
        ..inv("i2", "INV-2", 4_000)
    };
    let other_currency = Invoice {
        currency: "USD".to_owned(),
        ..inv("i2", "INV-2", 4_000)
    };
    for second in [other_scope, other_currency] {
        let r = run(
            &[inv("i1", "INV-1", 6_000), second],
            &[pay("p1", 10_000, "INV-1 INV-2")],
            &MatchConfig::default(),
        );
        assert!(r.matches.is_empty());
        assert_eq!(r.unmatched_payments, vec!["p1"]);
        assert_eq!(r.unmatched_invoices, vec!["i1", "i2"]);
    }
}

#[test]
fn a_group_is_skipped_when_a_better_match_took_one_of_its_invoices() {
    // p1 matches i1 one to one with a perfect score. p2 names i1 and i2; that group would
    // score lower (i2 is due 40 days after the payment) and i1 is taken by then.
    let late = Invoice {
        due_date: day(50),
        ..inv("i2", "INV-2", 4_000)
    };
    let r = run(
        &[inv("i1", "INV-1", 6_000), late],
        &[pay("p1", 6_000, "INV-1"), pay("p2", 10_000, "INV-1 INV-2")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].kind, MatchKind::OneToOne);
    assert_eq!(r.matches[0].payment_ids, vec!["p1"]);
    assert_eq!(r.unmatched_payments, vec!["p2"]);
    assert_eq!(r.unmatched_invoices, vec!["i2"]);
}

#[test]
fn group_sums_beyond_i64_do_not_overflow() {
    let half = 1_i64 << 62;
    let config = MatchConfig {
        amount_tolerance_minor: 1,
        ..MatchConfig::default()
    };
    let r = run(
        &[inv("i1", "INV-1", half), inv("i2", "INV-2", half)],
        &[pay("p1", i64::MAX, "INV-1 INV-2")],
        &config,
    );
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].kind, MatchKind::OneToMany);
    assert_eq!(r.matches[0].residual_minor, -1);
}

// The counterparty group passes find groups with no reference signal. Under the default
// weights such a group scores at most 0.55, below the suggest threshold of 0.60, so the
// tests below that expect a match use `no_reference_config()` (amount 0.6, name 0.2,
// date 0.2), where a perfect group scores 1.0 without any reference.

#[test]
fn one_to_many_is_found_by_counterparty_without_a_reference() {
    let r = run(
        &[inv("i1", "INV-1", 6_000), inv("i2", "INV-2", 4_000)],
        &[pay("p1", 10_000, "thanks")],
        &no_reference_config(),
    );
    assert_eq!(r.matches.len(), 1);
    let m = &r.matches[0];
    assert_eq!(m.kind, MatchKind::OneToMany);
    assert_eq!(m.status, MatchStatus::Auto);
    assert_eq!(m.payment_ids, vec!["p1"]);
    assert_eq!(m.invoice_ids, vec!["i1", "i2"]);
    assert_eq!(m.residual_minor, 0);
    assert!(close(m.breakdown.reference, 0.0));
    assert!(close(m.score, 1.0));
}

#[test]
fn many_to_one_is_found_by_counterparty_without_a_reference() {
    let r = run(
        &[inv("i1", "INV-1", 10_000)],
        &[pay("p1", 4_000, "thanks"), pay("p2", 6_000, "thanks")],
        &no_reference_config(),
    );
    assert_eq!(r.matches.len(), 1);
    let m = &r.matches[0];
    assert_eq!(m.kind, MatchKind::ManyToOne);
    assert_eq!(m.status, MatchStatus::Auto);
    assert_eq!(m.payment_ids, vec!["p1", "p2"]);
    assert_eq!(m.invoice_ids, vec!["i1"]);
    assert!(close(m.breakdown.reference, 0.0));
}

#[test]
fn no_reference_group_stays_unmatched_under_default_weights() {
    let r = run(
        &[inv("i1", "INV-1", 6_000), inv("i2", "INV-2", 4_000)],
        &[pay("p1", 10_000, "thanks")],
        &MatchConfig::default(),
    );
    assert!(r.matches.is_empty());
    assert_eq!(r.unmatched_payments, vec!["p1"]);
    assert_eq!(r.unmatched_invoices, vec!["i1", "i2"]);
    let r = run(
        &[inv("i1", "INV-1", 10_000)],
        &[pay("p1", 4_000, "thanks"), pay("p2", 6_000, "thanks")],
        &MatchConfig::default(),
    );
    assert!(r.matches.is_empty());
}

#[test]
fn partly_named_group_scores_half_the_reference_signal() {
    let r = run(
        &[inv("i1", "INV-1", 6_000), inv("i2", "INV-2", 4_000)],
        &[pay("p1", 10_000, "INV-1")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    let m = &r.matches[0];
    assert_eq!(m.kind, MatchKind::OneToMany);
    assert_eq!(m.invoice_ids, vec!["i1", "i2"]);
    assert!(close(m.breakdown.reference, 0.5));
    // 0.45 * 0.5 + 0.30 + 0.15 + 0.10 = 0.775: above suggest, below auto.
    assert_eq!(m.status, MatchStatus::Suggested);
}

#[test]
fn invoice_with_a_dissimilar_name_is_left_out_of_the_pool() {
    let stranger = Invoice {
        counterparty: "Zeta Holdings".to_owned(),
        ..inv("i2", "INV-2", 4_000)
    };
    let r = run(
        &[inv("i1", "INV-1", 6_000), stranger],
        &[pay("p1", 10_000, "thanks")],
        &no_reference_config(),
    );
    assert!(r.matches.is_empty());
    assert_eq!(r.unmatched_invoices, vec!["i1", "i2"]);
    let stranger = Payment {
        counterparty: "Zeta Holdings".to_owned(),
        ..pay("p2", 6_000, "thanks")
    };
    let r = run(
        &[inv("i1", "INV-1", 10_000)],
        &[pay("p1", 4_000, "thanks"), stranger],
        &no_reference_config(),
    );
    assert!(r.matches.is_empty());
    assert_eq!(r.unmatched_payments, vec!["p1", "p2"]);
}

#[test]
fn rows_outside_the_date_window_are_left_out_of_the_pool() {
    let late = Invoice {
        due_date: day(10 + 40),
        ..inv("i2", "INV-2", 4_000)
    };
    let r = run(
        &[inv("i1", "INV-1", 6_000), late],
        &[pay("p1", 10_000, "thanks")],
        &no_reference_config(),
    );
    assert!(r.matches.is_empty());
    let late = Payment {
        value_date: day(10 + 40),
        ..pay("p2", 6_000, "thanks")
    };
    let r = run(
        &[inv("i1", "INV-1", 10_000)],
        &[pay("p1", 4_000, "thanks"), late],
        &no_reference_config(),
    );
    assert!(r.matches.is_empty());
}

#[test]
fn several_valid_subsets_found_by_counterparty_are_ambiguous() {
    let r = run(
        &[
            inv("i1", "INV-1", 6_000),
            inv("i2", "INV-2", 4_000),
            inv("i3", "INV-3", 3_000),
            inv("i4", "INV-4", 7_000),
        ],
        &[pay("p1", 10_000, "thanks")],
        &no_reference_config(),
    );
    assert_eq!(r.matches.len(), 1);
    let m = &r.matches[0];
    assert_eq!(m.status, MatchStatus::Ambiguous);
    assert_eq!(m.invoice_ids, vec!["i1", "i2"]);
    assert_eq!(r.unmatched_invoices, vec!["i3", "i4"]);
}

/// 24 old invoices of 200 plus two newer ones (300 and 450) that add up to a payment of 750.
fn crowded_pool(old: usize) -> Vec<Invoice> {
    let mut invoices: Vec<Invoice> = (0..old)
        .map(|k| Invoice {
            due_date: day(k as u64 % 10),
            ..inv(&format!("old{k:02}"), &format!("OLD-{k:02}"), 200)
        })
        .collect();
    invoices.push(Invoice {
        due_date: day(20),
        ..inv("new1", "NEW-1", 300)
    });
    invoices.push(Invoice {
        due_date: day(21),
        ..inv("new2", "NEW-2", 450)
    });
    invoices
}

#[test]
fn a_pool_larger_than_24_only_considers_the_24_oldest() {
    let payments = [pay("p1", 750, "thanks")];
    let r = run(&crowded_pool(24), &payments, &no_reference_config());
    assert!(r.matches.is_empty());
    let r = run(&crowded_pool(22), &payments, &no_reference_config());
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].invoice_ids, vec!["new1", "new2"]);
}

#[test]
fn the_one_to_one_pass_runs_before_the_group_passes() {
    let r = run(
        &[
            inv("i1", "INV-1", 6_000),
            inv("i2", "INV-2", 4_000),
            inv("i3", "INV-3", 10_000),
        ],
        &[pay("p1", 10_000, "thanks")],
        &no_reference_config(),
    );
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].kind, MatchKind::OneToOne);
    assert_eq!(r.matches[0].invoice_ids, vec!["i3"]);
    assert_eq!(r.unmatched_invoices, vec!["i1", "i2"]);
}

#[test]
fn counterparty_anchors_choose_among_the_rows_still_free() {
    // One payer, no references: 30 invoices of 100 and 15 payments of 200 pair up exactly.
    // Every payment has more than 24 eligible invoices, so each search must see only the
    // invoices that earlier payments left free.
    let invoices: Vec<Invoice> = (0..30)
        .map(|k| inv(&format!("i{k:02}"), &format!("INV-{k:02}"), 100))
        .collect();
    let payments: Vec<Payment> = (0..15)
        .map(|k| pay(&format!("p{k:02}"), 200, "thanks"))
        .collect();
    let r = run(&invoices, &payments, &no_reference_config());
    assert_eq!(r.matches.len(), 15);
    assert!(r.matches.iter().all(|m| m.kind == MatchKind::OneToMany));
    assert!(r.unmatched_payments.is_empty() && r.unmatched_invoices.is_empty());
}

#[test]
fn counterparty_invoice_anchors_choose_among_the_payments_still_free() {
    let invoices: Vec<Invoice> = (0..15)
        .map(|k| inv(&format!("i{k:02}"), &format!("INV-{k:02}"), 200))
        .collect();
    let payments: Vec<Payment> = (0..30)
        .map(|k| pay(&format!("p{k:02}"), 100, "thanks"))
        .collect();
    let r = run(&invoices, &payments, &no_reference_config());
    assert_eq!(r.matches.len(), 15);
    assert!(r.matches.iter().all(|m| m.kind == MatchKind::ManyToOne));
    assert!(r.unmatched_payments.is_empty() && r.unmatched_invoices.is_empty());
}

#[test]
fn a_group_from_a_truncated_pool_is_never_auto() {
    // I00, I01, I28 and I29 are 1000 each; the rest are distinct small amounts. Only the 24
    // oldest invoices are searched, so I28 + I29 is out of reach and the answer is unsure.
    let invoices: Vec<Invoice> = (0..30)
        .map(|k| {
            let amount = if [0, 1, 28, 29].contains(&k) {
                1_000
            } else {
                100 + k
            };
            inv(&format!("I{k:02}"), &format!("INV-{k:02}"), amount)
        })
        .collect();
    let r = run(
        &invoices,
        &[pay("p1", 2_000, "thanks")],
        &no_reference_config(),
    );
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].kind, MatchKind::OneToMany);
    assert_eq!(r.matches[0].invoice_ids, vec!["I00", "I01"]);
    assert_eq!(r.matches[0].status, MatchStatus::Ambiguous);
}

#[test]
fn a_truncated_mention_pool_is_never_auto() {
    let config = MatchConfig {
        max_group_size: 24,
        ..MatchConfig::default()
    };
    let identical = |count: usize| -> Vec<Invoice> {
        (0..count)
            .map(|k| inv(&format!("i{k:02}"), "INV-7", 100))
            .collect()
    };
    let r = run(&identical(30), &[pay("p1", 2_400, "INV-7")], &config);
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].invoice_ids.len(), 24);
    assert_eq!(r.matches[0].status, MatchStatus::Ambiguous);
    // Exactly 24 mentioned invoices fit the cap: one subset, nothing was left out.
    let r = run(&identical(24), &[pay("p1", 2_400, "INV-7")], &config);
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].status, MatchStatus::Auto);
}

// Ambiguity and the suggest threshold. The ambiguity of an anchor looks at every candidate of
// its pass, including those below the suggest threshold. The tests below use weights with
// only `amount`, so a candidate scores exactly its amount signal (1 minus the difference
// over a tolerance of 100), and a margin of 0.3 against a threshold of 0.9.

fn amount_only_config() -> MatchConfig {
    MatchConfig {
        amount_tolerance_minor: 100,
        weights: weights(&[("amount", 1.0)]),
        suggest_threshold: 0.9,
        auto_threshold: 0.95,
        ambiguity_margin: 0.3,
        ..MatchConfig::default()
    }
}

/// Matches a payment of 10,000 against an exact invoice and one that is off by `off`.
fn one_to_one_with_a_rival(off: i64) -> ReconciliationResult {
    run(
        &[inv("i1", "INV-1", 10_000), inv("i2", "INV-2", 10_000 + off)],
        &[pay("p1", 10_000, "thanks")],
        &amount_only_config(),
    )
}

#[test]
fn a_one_to_one_rival_below_the_threshold_but_inside_the_margin_makes_the_match_ambiguous() {
    // The rival scores 0.8: below the threshold of 0.9, 0.2 under the accepted 1.0.
    let r = one_to_one_with_a_rival(20);
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].invoice_ids, vec!["i1"]);
    assert_eq!(r.matches[0].status, MatchStatus::Ambiguous);
    assert_eq!(r.unmatched_invoices, vec!["i2"]);
}

#[test]
fn a_one_to_one_rival_below_the_threshold_and_outside_the_margin_is_ignored() {
    // The rival scores 0.5, 0.5 under the accepted 1.0.
    let r = one_to_one_with_a_rival(50);
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].status, MatchStatus::Auto);
}

/// Matches a payment of 10,000 against groups of invoices: one exact, one off by `off`.
fn one_to_many_with_a_rival(off: i64) -> ReconciliationResult {
    run(
        &[
            inv("i1", "INV-1", 6_000),
            inv("i2", "INV-2", 4_000),
            inv("i3", "INV-3", 5_000),
            inv("i4", "INV-4", 5_000 + off),
        ],
        &[pay("p1", 10_000, "thanks")],
        &amount_only_config(),
    )
}

#[test]
fn a_one_to_many_rival_group_below_the_threshold_inside_the_margin_is_ambiguous() {
    let r = one_to_many_with_a_rival(20);
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].kind, MatchKind::OneToMany);
    assert_eq!(r.matches[0].invoice_ids, vec!["i1", "i2"]);
    assert_eq!(r.matches[0].status, MatchStatus::Ambiguous);
}

#[test]
fn a_one_to_many_rival_group_below_the_threshold_outside_the_margin_is_still_ambiguous() {
    // More than one fitting group is ambiguous by itself, whatever the scores.
    let r = one_to_many_with_a_rival(80);
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].invoice_ids, vec!["i1", "i2"]);
    assert_eq!(r.matches[0].status, MatchStatus::Ambiguous);
}

/// Matches an invoice of 10,000 against groups of payments: one exact, one off by `off`.
fn many_to_one_with_a_rival(off: i64) -> ReconciliationResult {
    run(
        &[inv("i1", "INV-1", 10_000)],
        &[
            pay("p1", 6_000, "thanks"),
            pay("p2", 4_000, "thanks"),
            pay("p3", 5_000, "thanks"),
            pay("p4", 5_000 + off, "thanks"),
        ],
        &amount_only_config(),
    )
}

#[test]
fn a_many_to_one_rival_group_below_the_threshold_inside_the_margin_is_ambiguous() {
    let r = many_to_one_with_a_rival(20);
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].kind, MatchKind::ManyToOne);
    assert_eq!(r.matches[0].payment_ids, vec!["p1", "p2"]);
    assert_eq!(r.matches[0].status, MatchStatus::Ambiguous);
}

#[test]
fn a_many_to_one_rival_group_below_the_threshold_outside_the_margin_is_still_ambiguous() {
    let r = many_to_one_with_a_rival(80);
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].payment_ids, vec!["p1", "p2"]);
    assert_eq!(r.matches[0].status, MatchStatus::Ambiguous);
}

#[test]
fn a_single_group_below_the_threshold_leaves_the_accepted_group_unambiguous() {
    // The only fitting group scores 1.0; the rest of the pool does not add up.
    let r = run(
        &[inv("i1", "INV-1", 6_000), inv("i2", "INV-2", 4_000)],
        &[pay("p1", 10_000, "thanks")],
        &amount_only_config(),
    );
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].status, MatchStatus::Auto);
}

#[test]
fn a_rival_exactly_the_margin_below_the_accepted_score_is_not_ambiguous() {
    // Default weights, threshold 0.55: the accepted candidate has no reference signal and
    // scores 0.55 (the default weights without `reference`). The rival is 4 days off its due
    // date inside a window of 5 days, so its date signal is 0.5 and it scores 0.05 less:
    // exactly the margin of 0.05, which the epsilon of the comparison reads as not ambiguous.
    let config = MatchConfig {
        suggest_threshold: 0.55,
        date_window_days: 5,
        ..MatchConfig::default()
    };
    let rival = Invoice {
        due_date: day(14),
        ..inv("i2", "INV-2", 10_000)
    };
    let r = run(
        &[inv("i1", "INV-1", 10_000), rival],
        &[pay("p1", 10_000, "thanks")],
        &config,
    );
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].invoice_ids, vec!["i1"]);
    assert_eq!(r.matches[0].status, MatchStatus::Suggested);
}

#[test]
fn a_rival_just_inside_the_margin_below_the_accepted_score_is_ambiguous() {
    // As above with a date window of 6 days: the rival's date signal is 1 - 1/3, so it
    // scores about 0.033 less than the accepted candidate, inside the margin of 0.05.
    let config = MatchConfig {
        suggest_threshold: 0.55,
        date_window_days: 6,
        ..MatchConfig::default()
    };
    let rival = Invoice {
        due_date: day(14),
        ..inv("i2", "INV-2", 10_000)
    };
    let r = run(
        &[inv("i1", "INV-1", 10_000), rival],
        &[pay("p1", 10_000, "thanks")],
        &config,
    );
    assert_eq!(r.matches[0].status, MatchStatus::Ambiguous);
}

#[test]
fn a_candidate_without_reference_sits_at_the_pruning_bound_under_the_defaults() {
    // Under the default weights a candidate with no reference signal scores at most 0.55,
    // which is the suggest threshold (0.60) minus the margin (0.05): it cannot be accepted
    // and, for an anchor with an accepted candidate, it is not a close rival either. Here the
    // rival is a perfect candidate without a reference (0.55) next to a fuzzy-reference
    // candidate that scores well above it.
    let r = run(
        &[
            inv("i1", "INV-20240001", 10_000),
            inv("i2", "OTHER-9", 10_000),
        ],
        &[pay("p1", 10_000, "PAGO INV-20240011")],
        &MatchConfig::default(),
    );
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].invoice_ids, vec!["i1"]);
    assert_eq!(r.matches[0].status, MatchStatus::Auto);
    assert_eq!(r.unmatched_invoices, vec!["i2"]);
}

#[test]
fn a_payment_naming_an_invoice_that_does_not_fit_still_matches_an_unnamed_one() {
    // p1 names i1, whose amount is far off, so the payment is explained and its other
    // candidates get no fuzzy reference signal; with weights where the unnamed invoice can
    // still score, it is matched.
    let r = run(
        &[inv("i1", "INV-1", 99_999), inv("i2", "INV-2", 10_000)],
        &[pay("p1", 10_000, "INV-1")],
        &no_reference_config(),
    );
    assert_eq!(r.matches.len(), 1);
    assert_eq!(r.matches[0].invoice_ids, vec!["i2"]);
    assert_eq!(r.matches[0].status, MatchStatus::Auto);
    assert!(close(r.matches[0].breakdown.reference, 0.0));
    assert_eq!(r.unmatched_invoices, vec!["i1"]);
}
