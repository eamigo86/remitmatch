//! The 1:1 matching engine: validation, partitioning, the reference pass, the candidate
//! pass and greedy assignment.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use aho_corasick::{AhoCorasick, MatchKind as AcMatchKind};

use crate::normalize::{Reference, normalize_name, normalize_reference};
use crate::score::{
    EPS, Signals, Weights, amount_signal, breakdown, date_signal, fuzzy_reference_signal, meets,
    name_signal, weighted_score,
};
use crate::{
    Error, Invoice, Match, MatchConfig, MatchKind, MatchStatus, Payment, ReconciliationResult,
};

/// Signal names accepted as keys of [`MatchConfig::weights`].
const SIGNALS: [&str; 4] = ["reference", "amount", "name", "date"];

/// Input indices of the invoices and payments of one partition.
type PartitionRows = (Vec<usize>, Vec<usize>);

/// A scored payment-invoice pair that has not been assigned yet.
struct Candidate {
    payment: usize,
    invoice: usize,
    score: f64,
    signals: Signals,
}

/// Normalized inputs shared by every partition.
struct Engine<'a> {
    invoices: &'a [Invoice],
    payments: &'a [Payment],
    config: &'a MatchConfig,
    weights: Weights,
    /// Amount tolerance as an unsigned value; validated to be non-negative.
    tolerance: u64,
    invoice_number: Vec<String>,
    invoice_name: Vec<String>,
    payment_reference: Vec<Reference>,
    payment_name: Vec<String>,
}

/// Which rows are already matched, plus the matches found so far.
struct Assigned {
    invoice_taken: Vec<bool>,
    payment_taken: Vec<bool>,
    /// Each match with the input index of its payment, used for ordering.
    found: Vec<(usize, Match)>,
}

/// Runs the 1:1 engine; see [`crate::reconcile`] for the contract.
pub(crate) fn reconcile(
    invoices: &[Invoice],
    payments: &[Payment],
    config: &MatchConfig,
) -> Result<ReconciliationResult, Error> {
    validate(invoices, payments, config)?;
    let engine = Engine {
        invoices,
        payments,
        config,
        weights: Weights::from_map(&config.weights),
        tolerance: u64::try_from(config.amount_tolerance_minor).unwrap_or(0),
        invoice_number: invoices
            .iter()
            .map(|i| normalize_reference(&i.number).text)
            .collect(),
        invoice_name: invoices
            .iter()
            .map(|i| normalize_name(&i.counterparty))
            .collect(),
        payment_reference: payments
            .iter()
            .map(|p| normalize_reference(&p.reference))
            .collect(),
        payment_name: payments
            .iter()
            .map(|p| normalize_name(&p.counterparty))
            .collect(),
    };
    let mut state = Assigned {
        invoice_taken: vec![false; invoices.len()],
        payment_taken: vec![false; payments.len()],
        found: Vec::new(),
    };
    for (invoice_rows, payment_rows) in engine.partitions().values() {
        engine.run_partition(invoice_rows, payment_rows, &mut state)?;
    }
    state
        .found
        .sort_by(|a, b| b.1.score.total_cmp(&a.1.score).then_with(|| a.0.cmp(&b.0)));
    Ok(ReconciliationResult {
        matches: state.found.into_iter().map(|(_, m)| m).collect(),
        unmatched_payments: unmatched(payments.iter().map(|p| &p.id), &state.payment_taken),
        unmatched_invoices: unmatched(invoices.iter().map(|i| &i.id), &state.invoice_taken),
    })
}

/// Returns the ids whose rows were not taken, in input order.
fn unmatched<'a>(ids: impl Iterator<Item = &'a String>, taken: &[bool]) -> Vec<String> {
    ids.zip(taken)
        .filter(|&(_, &t)| !t)
        .map(|(id, _)| id.clone())
        .collect()
}

/// Rejects duplicate ids and out-of-range configuration.
fn validate(invoices: &[Invoice], payments: &[Payment], config: &MatchConfig) -> Result<(), Error> {
    validate_config(config)?;
    let mut seen = HashSet::new();
    for invoice in invoices {
        if !seen.insert(invoice.id.as_str()) {
            return Err(Error::DuplicateInvoiceId(invoice.id.clone()));
        }
    }
    seen.clear();
    for payment in payments {
        if !seen.insert(payment.id.as_str()) {
            return Err(Error::DuplicatePaymentId(payment.id.clone()));
        }
    }
    Ok(())
}

/// Checks every configuration field the 1:1 engine uses.
fn validate_config(config: &MatchConfig) -> Result<(), Error> {
    let invalid = |message: String| Err(Error::InvalidConfig(message));
    for (field, value) in [
        ("auto_threshold", config.auto_threshold),
        ("suggest_threshold", config.suggest_threshold),
    ] {
        if !(value.is_finite() && (0.0..=1.0).contains(&value)) {
            return invalid(format!("{field} must be between 0 and 1, got {value}"));
        }
    }
    if config.suggest_threshold > config.auto_threshold {
        return invalid("suggest_threshold must not exceed auto_threshold".to_owned());
    }
    if !(config.ambiguity_margin.is_finite() && config.ambiguity_margin >= 0.0) {
        return invalid(format!(
            "ambiguity_margin must be finite and not negative, got {}",
            config.ambiguity_margin
        ));
    }
    if config.amount_tolerance_minor < 0 {
        return invalid(format!(
            "amount_tolerance_minor must not be negative, got {}",
            config.amount_tolerance_minor
        ));
    }
    for (key, weight) in &config.weights {
        if !SIGNALS.contains(&key.as_str()) {
            return invalid(format!("weights has unknown signal {key:?}"));
        }
        if !(weight.is_finite() && *weight >= 0.0) {
            return invalid(format!(
                "weights[{key}] must be finite and not negative, got {weight}"
            ));
        }
    }
    let sum: f64 = config.weights.values().sum();
    if !(sum.is_finite() && sum > 0.0) {
        return invalid("weights must sum to a positive number".to_owned());
    }
    Ok(())
}

/// Returns whether a hit at bytes `[start, end)` of the normalized reference is a whole
/// mention: neither edge may split a digit run unless whitespace was removed there.
fn is_mention(reference: &Reference, start: usize, end: usize) -> bool {
    let bytes = reference.text.as_bytes();
    let splits_digits = |edge: usize| {
        edge > 0
            && edge < bytes.len()
            && !reference.gap_before[edge]
            && bytes[edge - 1].is_ascii_digit()
            && bytes[edge].is_ascii_digit()
    };
    !splits_digits(start) && !splits_digits(end)
}

impl Engine<'_> {
    /// Groups the rows with a positive amount by scope and upper-cased currency.
    fn partitions(&self) -> BTreeMap<(&str, String), PartitionRows> {
        let mut partitions: BTreeMap<(&str, String), PartitionRows> = BTreeMap::new();
        for (i, invoice) in self.invoices.iter().enumerate() {
            if invoice.amount_minor > 0 {
                let key = (
                    invoice.scope.as_str(),
                    invoice.currency.to_ascii_uppercase(),
                );
                partitions.entry(key).or_default().0.push(i);
            }
        }
        for (p, payment) in self.payments.iter().enumerate() {
            if payment.amount_minor > 0 {
                let key = (
                    payment.scope.as_str(),
                    payment.currency.to_ascii_uppercase(),
                );
                partitions.entry(key).or_default().1.push(p);
            }
        }
        partitions
    }

    /// Runs the reference pass and then the candidate pass on one partition.
    fn run_partition(
        &self,
        invoice_rows: &[usize],
        payment_rows: &[usize],
        state: &mut Assigned,
    ) -> Result<(), Error> {
        if invoice_rows.is_empty() || payment_rows.is_empty() {
            return Ok(());
        }
        let (candidates, explained) = self.reference_pass(invoice_rows, payment_rows)?;
        self.assign(candidates, state);

        let mut open: Vec<(i64, usize)> = invoice_rows
            .iter()
            .filter(|&&i| !state.invoice_taken[i])
            .map(|&i| (self.invoices[i].amount_minor, i))
            .collect();
        open.sort_unstable();
        let candidates = payment_rows
            .iter()
            .filter(|&&p| !state.payment_taken[p])
            .flat_map(|&p| self.window_candidates(p, &open, explained.contains(&p)))
            .collect();
        self.assign(candidates, state);
        Ok(())
    }

    /// Scores one pair with the given reference signal.
    fn candidate(&self, payment: usize, invoice: usize, reference: f64) -> Candidate {
        let (pay, inv) = (&self.payments[payment], &self.invoices[invoice]);
        let signals = Signals {
            reference,
            amount: amount_signal(
                pay.amount_minor.abs_diff(inv.amount_minor),
                self.config.amount_tolerance_minor,
            ),
            name: name_signal(&self.payment_name[payment], &self.invoice_name[invoice]),
            date: date_signal(pay.value_date, inv.due_date, self.config.date_window_days),
        };
        Candidate {
            payment,
            invoice,
            score: weighted_score(&signals, &self.weights),
            signals,
        }
    }

    /// Finds the invoices each payment mentions exactly, in one scan per payment.
    ///
    /// Returns the candidates whose amount fits (the date window is ignored) and the
    /// payments that mention at least one invoice, whether or not its amount fits.
    fn reference_pass(
        &self,
        invoice_rows: &[usize],
        payment_rows: &[usize],
    ) -> Result<(Vec<Candidate>, HashSet<usize>), Error> {
        let mut by_number: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for &i in invoice_rows {
            let number = self.invoice_number[i].as_str();
            if !number.is_empty() {
                by_number.entry(number).or_default().push(i);
            }
        }
        let mut candidates = Vec::new();
        let mut explained = HashSet::new();
        if by_number.is_empty() {
            return Ok((candidates, explained));
        }
        let automaton = AhoCorasick::builder()
            .match_kind(AcMatchKind::Standard)
            .build(by_number.keys())
            .map_err(|e| Error::ReferenceIndex(e.to_string()))?;
        let groups: Vec<&Vec<usize>> = by_number.values().collect();
        for &p in payment_rows {
            let reference = &self.payment_reference[p];
            let mut mentioned = BTreeSet::new();
            for hit in automaton.find_overlapping_iter(&reference.text) {
                if is_mention(reference, hit.start(), hit.end()) {
                    mentioned.extend(groups[hit.pattern().as_usize()].iter().copied());
                }
            }
            if mentioned.is_empty() {
                continue;
            }
            explained.insert(p);
            for i in mentioned {
                let diff = self.payments[p]
                    .amount_minor
                    .abs_diff(self.invoices[i].amount_minor);
                if diff <= self.tolerance {
                    candidates.push(self.candidate(p, i, 1.0));
                }
            }
        }
        Ok((candidates, explained))
    }

    /// Scores the open invoices within the amount tolerance and the date window.
    ///
    /// `open` is sorted by amount, so a binary search bounds the invoices to look at.
    /// A payment that already mentions an invoice exactly gets no fuzzy reference signal.
    fn window_candidates(
        &self,
        payment: usize,
        open: &[(i64, usize)],
        explained: bool,
    ) -> Vec<Candidate> {
        let pay = &self.payments[payment];
        let tolerance = self.config.amount_tolerance_minor;
        let low = pay.amount_minor.saturating_sub(tolerance);
        let high = pay.amount_minor.saturating_add(tolerance);
        let start = open.partition_point(|&(amount, _)| amount < low);
        let end = open.partition_point(|&(amount, _)| amount <= high);
        open[start..end]
            .iter()
            .filter(|&&(_, i)| {
                let days = pay
                    .value_date
                    .signed_duration_since(self.invoices[i].due_date)
                    .num_days()
                    .unsigned_abs();
                days <= u64::from(self.config.date_window_days)
            })
            .map(|&(_, i)| {
                let reference = if explained {
                    0.0
                } else {
                    fuzzy_reference_signal(
                        &self.invoice_number[i],
                        &self.payment_reference[payment].text,
                    )
                };
                self.candidate(payment, i, reference)
            })
            .collect()
    }

    /// Greedily accepts the best candidates whose payment and invoice are both free.
    ///
    /// A payment's match is ambiguous when its two best candidates in this pass (any score,
    /// taken or not) are closer than the ambiguity margin; the flag applies to whichever of
    /// its candidates is accepted. A payment with a single candidate is never ambiguous.
    fn assign(&self, mut candidates: Vec<Candidate>, state: &mut Assigned) {
        let config = self.config;
        let mut top_two: HashMap<usize, (f64, f64)> = HashMap::new();
        for c in &candidates {
            let (first, second) = top_two
                .entry(c.payment)
                .or_insert((f64::NEG_INFINITY, f64::NEG_INFINITY));
            if c.score > *first {
                *second = *first;
                *first = c.score;
            } else if c.score > *second {
                *second = c.score;
            }
        }
        candidates.retain(|c| meets(c.score, config.suggest_threshold));
        candidates.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.payment.cmp(&b.payment))
                .then_with(|| a.invoice.cmp(&b.invoice))
        });
        for c in candidates {
            if state.payment_taken[c.payment] || state.invoice_taken[c.invoice] {
                continue;
            }
            state.payment_taken[c.payment] = true;
            state.invoice_taken[c.invoice] = true;
            let (first, second) = top_two[&c.payment];
            let status = if first - second + EPS < config.ambiguity_margin {
                MatchStatus::Ambiguous
            } else if meets(c.score, config.auto_threshold) {
                MatchStatus::Auto
            } else {
                MatchStatus::Suggested
            };
            let (pay, inv) = (&self.payments[c.payment], &self.invoices[c.invoice]);
            state.found.push((
                c.payment,
                Match {
                    kind: MatchKind::OneToOne,
                    status,
                    payment_ids: vec![pay.id.clone()],
                    invoice_ids: vec![inv.id.clone()],
                    score: c.score,
                    breakdown: breakdown(&c.signals),
                    residual_minor: pay.amount_minor - inv.amount_minor,
                },
            ));
        }
    }
}
