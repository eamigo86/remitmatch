//! The matching engine: validation, partitioning, the reference passes, the 1:1 candidate
//! pass, group search and greedy assignment.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use aho_corasick::{AhoCorasick, MatchKind as AcMatchKind};
use chrono::NaiveDate;

use crate::normalize::{Reference, normalize_name, normalize_reference};
use crate::score::{
    EPS, Signals, Weights, amount_signal, breakdown, date_signal, fuzzy_reference_signal, meets,
    name_signal, weighted_score,
};
use crate::subset::{GROUP_CANDIDATE_LIMIT, GROUP_SUBSET_LIMIT, find_subsets};
use crate::{
    Error, Invoice, Match, MatchConfig, MatchKind, MatchStatus, Payment, ReconciliationResult,
};

/// Signal names accepted as keys of [`MatchConfig::weights`].
const SIGNALS: [&str; 4] = ["reference", "amount", "name", "date"];

/// Input indices of the invoices and payments of one partition.
type PartitionRows = (Vec<usize>, Vec<usize>);

/// Minimum name signal for a row to enter the pool of a counterparty group search.
const GROUP_NAME_THRESHOLD: f64 = 0.85;

/// Invoices each payment of a partition mentions exactly, by input index.
type Mentions = HashMap<usize, BTreeSet<usize>>;

/// The row a candidate search starts from; ambiguity is judged per anchor.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Anchor {
    /// The payment of a 1:1 or 1:N candidate.
    Payment(usize),
    /// The invoice of an N:1 candidate.
    Invoice(usize),
}

/// A scored set of payments and invoices that has not been assigned yet.
///
/// Both lists are non-empty and hold ascending input indices; one of them has one entry.
struct Candidate {
    anchor: Anchor,
    payments: Vec<usize>,
    invoices: Vec<usize>,
    score: f64,
    signals: Signals,
}

impl Candidate {
    /// Returns whether the candidate links more than one payment or more than one invoice.
    fn is_group(&self) -> bool {
        self.payments.len() + self.invoices.len() > 2
    }
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
    /// Each match with the smallest input index of its payments, used for ordering.
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

    /// Runs the reference passes, the 1:1 candidate pass and the counterparty group passes
    /// on one partition.
    fn run_partition(
        &self,
        invoice_rows: &[usize],
        payment_rows: &[usize],
        state: &mut Assigned,
    ) -> Result<(), Error> {
        if invoice_rows.is_empty() || payment_rows.is_empty() {
            return Ok(());
        }
        let (candidates, mentions) = self.reference_pass(invoice_rows, payment_rows)?;
        self.assign(candidates, state);
        let candidates = self.named_payment_groups(payment_rows, &mentions, state);
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
            .flat_map(|&p| self.window_candidates(p, &open, mentions.contains_key(&p)))
            .collect();
        self.assign(candidates, state);

        let candidates = self.invoice_groups_by_name(invoice_rows, payment_rows, &mentions, state);
        self.assign(candidates, state);
        let candidates = self.payment_groups_by_name(invoice_rows, payment_rows, &mentions, state);
        self.assign(candidates, state);
        Ok(())
    }

    /// Returns whether two dates are at most `date_window_days` apart.
    fn within_window(&self, a: NaiveDate, b: NaiveDate) -> bool {
        a.signed_duration_since(b).num_days().unsigned_abs()
            <= u64::from(self.config.date_window_days)
    }

    /// Sums the amounts of the given payments.
    fn payment_total(&self, payments: &[usize]) -> i128 {
        payments
            .iter()
            .map(|&p| i128::from(self.payments[p].amount_minor))
            .sum()
    }

    /// Sums the amounts of the given invoices.
    fn invoice_total(&self, invoices: &[usize]) -> i128 {
        invoices
            .iter()
            .map(|&i| i128::from(self.invoices[i].amount_minor))
            .sum()
    }

    /// Scores a set of payments against a set of invoices with the given reference signal.
    ///
    /// The amount signal reads the difference of the two totals; the name signal is the
    /// minimum and the date signal the mean over every payment-invoice pair.
    fn candidate(
        &self,
        anchor: Anchor,
        payments: Vec<usize>,
        invoices: Vec<usize>,
        reference: f64,
    ) -> Candidate {
        let diff = self
            .payment_total(&payments)
            .abs_diff(self.invoice_total(&invoices));
        let (mut name, mut date) = (1.0_f64, 0.0_f64);
        for &p in &payments {
            for &i in &invoices {
                name = name.min(name_signal(&self.payment_name[p], &self.invoice_name[i]));
                date += date_signal(
                    self.payments[p].value_date,
                    self.invoices[i].due_date,
                    self.config.date_window_days,
                );
            }
        }
        let signals = Signals {
            reference,
            // Only sets whose difference fits the tolerance reach this point.
            amount: amount_signal(
                u64::try_from(diff).unwrap_or(u64::MAX),
                self.config.amount_tolerance_minor,
            ),
            name,
            date: date / (payments.len() * invoices.len()) as f64,
        };
        Candidate {
            anchor,
            payments,
            invoices,
            score: weighted_score(&signals, &self.weights),
            signals,
        }
    }

    /// Returns the subsets of `rows` whose amounts add up to `target` within the tolerance.
    ///
    /// `amount` reads a row's amount. Each subset lists ascending input indices; at most
    /// `max_group_size` rows per subset, none when that is below 2. Only the first
    /// [`GROUP_CANDIDATE_LIMIT`] rows are searched, so the caller orders them by priority.
    fn fitting_groups(
        &self,
        rows: &[usize],
        amount: impl Fn(usize) -> i64,
        target: i64,
    ) -> Vec<Vec<usize>> {
        let max_size = self.config.max_group_size;
        if max_size < 2 {
            return Vec::new();
        }
        let rows = &rows[..rows.len().min(GROUP_CANDIDATE_LIMIT)];
        let amounts: Vec<i64> = rows.iter().map(|&r| amount(r)).collect();
        find_subsets(
            &amounts,
            target,
            self.tolerance,
            max_size,
            GROUP_SUBSET_LIMIT,
        )
        .into_iter()
        .map(|subset| {
            let mut group: Vec<usize> = subset.into_iter().map(|k| rows[k]).collect();
            group.sort_unstable();
            group
        })
        .collect()
    }

    /// Finds the invoices each payment mentions exactly, in one scan per payment.
    ///
    /// Returns the candidates anchored at the payment whose amount fits (the date window is
    /// ignored): each mentioned invoice alone and each subset of mentioned invoices (1:N).
    /// Also returns every mention, including those whose amount does not fit.
    fn reference_pass(
        &self,
        invoice_rows: &[usize],
        payment_rows: &[usize],
    ) -> Result<(Vec<Candidate>, Mentions), Error> {
        let mut by_number: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for &i in invoice_rows {
            let number = self.invoice_number[i].as_str();
            if !number.is_empty() {
                by_number.entry(number).or_default().push(i);
            }
        }
        let mut candidates = Vec::new();
        let mut all_mentions = Mentions::new();
        if by_number.is_empty() {
            return Ok((candidates, all_mentions));
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
            for &i in &mentioned {
                let diff = self.payments[p]
                    .amount_minor
                    .abs_diff(self.invoices[i].amount_minor);
                if diff <= self.tolerance {
                    candidates.push(self.candidate(Anchor::Payment(p), vec![p], vec![i], 1.0));
                }
            }
            let pool = oldest(mentioned.iter().copied().collect(), |i| {
                self.invoices[i].due_date
            });
            for group in self.fitting_groups(
                &pool,
                |i| self.invoices[i].amount_minor,
                self.payments[p].amount_minor,
            ) {
                candidates.push(self.candidate(Anchor::Payment(p), vec![p], group, 1.0));
            }
            all_mentions.insert(p, mentioned);
        }
        Ok((candidates, all_mentions))
    }

    /// Builds the N:1 candidates anchored at each open invoice that open payments name.
    ///
    /// Each candidate is a subset of at least two such payments whose sum fits the invoice.
    fn named_payment_groups(
        &self,
        payment_rows: &[usize],
        mentions: &Mentions,
        state: &Assigned,
    ) -> Vec<Candidate> {
        let mut naming: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for &p in payment_rows.iter().filter(|&&p| !state.payment_taken[p]) {
            for &i in mentions.get(&p).into_iter().flatten() {
                if !state.invoice_taken[i] {
                    naming.entry(i).or_default().push(p);
                }
            }
        }
        let mut candidates = Vec::new();
        for (i, payments) in naming {
            let pool = oldest(payments, |p| self.payments[p].value_date);
            for group in self.fitting_groups(
                &pool,
                |p| self.payments[p].amount_minor,
                self.invoices[i].amount_minor,
            ) {
                candidates.push(self.candidate(Anchor::Invoice(i), group, vec![i], 1.0));
            }
        }
        candidates
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
            .filter(|&&(_, i)| self.within_window(pay.value_date, self.invoices[i].due_date))
            .map(|&(_, i)| {
                let reference = if explained {
                    0.0
                } else {
                    fuzzy_reference_signal(
                        &self.invoice_number[i],
                        &self.payment_reference[payment].text,
                    )
                };
                self.candidate(Anchor::Payment(payment), vec![payment], vec![i], reference)
            })
            .collect()
    }

    /// Returns the share of payment-invoice pairs where the payment names the invoice exactly.
    fn mention_share(&self, mentions: &Mentions, payments: &[usize], invoices: &[usize]) -> f64 {
        let named = payments
            .iter()
            .flat_map(|&p| invoices.iter().map(move |&i| (p, i)))
            .filter(|&(p, i)| mentions.get(&p).is_some_and(|set| set.contains(&i)))
            .count();
        named as f64 / (payments.len() * invoices.len()) as f64
    }

    /// Builds the 1:N candidates of each open payment from its counterparty's open invoices.
    ///
    /// The pool holds the open invoices whose name reaches [`GROUP_NAME_THRESHOLD`], that fall
    /// inside the date window and that do not exceed the payment plus the tolerance. The
    /// reference signal is the share of pairs the payment names exactly.
    fn invoice_groups_by_name(
        &self,
        invoice_rows: &[usize],
        payment_rows: &[usize],
        mentions: &Mentions,
        state: &Assigned,
    ) -> Vec<Candidate> {
        let invoices = by_name(invoice_rows, &state.invoice_taken, &self.invoice_name);
        let payments: Vec<usize> = payment_rows
            .iter()
            .copied()
            .filter(|&p| !state.payment_taken[p])
            .collect();
        let names = payments.iter().map(|&p| self.payment_name[p].as_str());
        let similar = similar_buckets(names, &invoices);
        let mut candidates = Vec::new();
        for p in payments {
            let pay = &self.payments[p];
            let limit = i128::from(pay.amount_minor) + i128::from(self.tolerance);
            let pool = similar
                .get(self.payment_name[p].as_str())
                .into_iter()
                .flatten()
                .flat_map(|rows| rows.iter().copied())
                .filter(|&i| {
                    let inv = &self.invoices[i];
                    i128::from(inv.amount_minor) <= limit
                        && self.within_window(pay.value_date, inv.due_date)
                })
                .collect();
            let pool = oldest(pool, |i| self.invoices[i].due_date);
            for group in
                self.fitting_groups(&pool, |i| self.invoices[i].amount_minor, pay.amount_minor)
            {
                let reference = self.mention_share(mentions, &[p], &group);
                candidates.push(self.candidate(Anchor::Payment(p), vec![p], group, reference));
            }
        }
        candidates
    }

    /// Builds the N:1 candidates of each open invoice from its counterparty's open payments.
    ///
    /// Mirror of [`Self::invoice_groups_by_name`]: the pool holds the open payments whose
    /// name reaches [`GROUP_NAME_THRESHOLD`], inside the date window and not above the
    /// invoice plus the tolerance.
    fn payment_groups_by_name(
        &self,
        invoice_rows: &[usize],
        payment_rows: &[usize],
        mentions: &Mentions,
        state: &Assigned,
    ) -> Vec<Candidate> {
        let payments = by_name(payment_rows, &state.payment_taken, &self.payment_name);
        let invoices: Vec<usize> = invoice_rows
            .iter()
            .copied()
            .filter(|&i| !state.invoice_taken[i])
            .collect();
        let names = invoices.iter().map(|&i| self.invoice_name[i].as_str());
        let similar = similar_buckets(names, &payments);
        let mut candidates = Vec::new();
        for i in invoices {
            let inv = &self.invoices[i];
            let limit = i128::from(inv.amount_minor) + i128::from(self.tolerance);
            let pool = similar
                .get(self.invoice_name[i].as_str())
                .into_iter()
                .flatten()
                .flat_map(|rows| rows.iter().copied())
                .filter(|&p| {
                    let pay = &self.payments[p];
                    i128::from(pay.amount_minor) <= limit
                        && self.within_window(pay.value_date, inv.due_date)
                })
                .collect();
            let pool = oldest(pool, |p| self.payments[p].value_date);
            for group in
                self.fitting_groups(&pool, |p| self.payments[p].amount_minor, inv.amount_minor)
            {
                let reference = self.mention_share(mentions, &group, &[i]);
                candidates.push(self.candidate(Anchor::Invoice(i), group, vec![i], reference));
            }
        }
        candidates
    }

    /// Greedily accepts the best candidates whose payments and invoices are all free.
    ///
    /// A match is ambiguous when its anchor's two best candidates in this pass (any score,
    /// taken or not) are closer than the ambiguity margin, or when it is a group and its
    /// anchor had more than one fitting group in this pass. A lone candidate is never
    /// ambiguous by the margin rule.
    fn assign(&self, mut candidates: Vec<Candidate>, state: &mut Assigned) {
        let config = self.config;
        let mut top_two: HashMap<Anchor, (f64, f64)> = HashMap::new();
        let mut groups_per_anchor: HashMap<Anchor, usize> = HashMap::new();
        for c in &candidates {
            let (first, second) = top_two
                .entry(c.anchor)
                .or_insert((f64::NEG_INFINITY, f64::NEG_INFINITY));
            if c.score > *first {
                *second = *first;
                *first = c.score;
            } else if c.score > *second {
                *second = c.score;
            }
            if c.is_group() {
                *groups_per_anchor.entry(c.anchor).or_default() += 1;
            }
        }
        candidates.retain(|c| meets(c.score, config.suggest_threshold));
        candidates.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.payments.first().cmp(&b.payments.first()))
                .then_with(|| a.invoices.first().cmp(&b.invoices.first()))
                .then_with(|| a.payments.cmp(&b.payments))
                .then_with(|| a.invoices.cmp(&b.invoices))
        });
        for c in candidates {
            if c.payments.iter().any(|&p| state.payment_taken[p])
                || c.invoices.iter().any(|&i| state.invoice_taken[i])
            {
                continue;
            }
            for &p in &c.payments {
                state.payment_taken[p] = true;
            }
            for &i in &c.invoices {
                state.invoice_taken[i] = true;
            }
            let (first, second) = top_two[&c.anchor];
            let several_groups = c.is_group() && groups_per_anchor.get(&c.anchor) > Some(&1);
            let status = if first - second + EPS < config.ambiguity_margin || several_groups {
                MatchStatus::Ambiguous
            } else if meets(c.score, config.auto_threshold) {
                MatchStatus::Auto
            } else {
                MatchStatus::Suggested
            };
            let kind = match (c.payments.len(), c.invoices.len()) {
                (1, 1) => MatchKind::OneToOne,
                (1, _) => MatchKind::OneToMany,
                _ => MatchKind::ManyToOne,
            };
            // The difference is within the tolerance, which is a non-negative i64.
            let residual =
                i64::try_from(self.payment_total(&c.payments) - self.invoice_total(&c.invoices))
                    .unwrap_or_default();
            state.found.push((
                c.payments.first().copied().unwrap_or_default(),
                Match {
                    kind,
                    status,
                    payment_ids: c
                        .payments
                        .iter()
                        .map(|&p| self.payments[p].id.clone())
                        .collect(),
                    invoice_ids: c
                        .invoices
                        .iter()
                        .map(|&i| self.invoices[i].id.clone())
                        .collect(),
                    score: c.score,
                    breakdown: breakdown(&c.signals),
                    residual_minor: residual,
                },
            ));
        }
    }
}

/// Buckets the free rows by normalized name, keeping input order inside each bucket.
fn by_name<'a>(
    rows: &[usize],
    taken: &[bool],
    names: &'a [String],
) -> BTreeMap<&'a str, Vec<usize>> {
    let mut buckets: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for &row in rows.iter().filter(|&&r| !taken[r]) {
        buckets.entry(names[row].as_str()).or_default().push(row);
    }
    buckets
}

/// Finds, for each distinct anchor name, the buckets whose name reaches the threshold.
///
/// Names are compared once per distinct pair, not once per row pair.
// ponytail: distinct anchor names x distinct bucket names Jaro-Winkler comparisons per
// partition. Upgrade: block the names (first letters or n-grams) before comparing if a
// partition holds tens of thousands of distinct names.
fn similar_buckets<'a, 'b>(
    anchor_names: impl Iterator<Item = &'a str>,
    buckets: &'b BTreeMap<&str, Vec<usize>>,
) -> HashMap<&'a str, Vec<&'b Vec<usize>>> {
    let mut similar: HashMap<&str, Vec<&Vec<usize>>> = HashMap::new();
    for name in anchor_names {
        similar.entry(name).or_insert_with(|| {
            buckets
                .iter()
                .filter(|&(&other, _)| meets(name_signal(name, other), GROUP_NAME_THRESHOLD))
                .map(|(_, rows)| rows)
                .collect()
        });
    }
    similar
}

/// Orders rows by date and then input index, so a truncated pool keeps the oldest rows.
fn oldest(mut rows: Vec<usize>, date: impl Fn(usize) -> NaiveDate) -> Vec<usize> {
    rows.sort_by_key(|&r| (date(r), r));
    rows
}
