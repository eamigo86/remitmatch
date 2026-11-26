//! The matching engine: validation, partitioning, the reference passes, the 1:1 candidate
//! pass, group search and greedy assignment.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use aho_corasick::{AhoCorasick, MatchKind as AcMatchKind};
use chrono::{Days, NaiveDate};
use rayon::prelude::*;

use crate::fasthash::{FastMap, FastSet};
use crate::normalize::{Reference, normalize_name, normalize_reference};
use crate::score::{
    EPS, LazyCounts, Signals, Weights, amount_signal, breakdown, date_signal,
    fuzzy_reference_signal_with, meets, name_signal, weighted_score,
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
type Mentions = FastMap<usize, BTreeSet<usize>>;

/// The row a candidate search starts from; ambiguity is judged per anchor.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Anchor {
    /// The payment of a 1:1 or 1:N candidate.
    Payment(usize),
    /// The invoice of an N:1 candidate.
    Invoice(usize),
}

/// The rows of one side of a candidate; a single row needs no allocation.
enum Members {
    One(usize),
    Many(Vec<usize>),
}

impl Members {
    /// Returns the input indices, ascending.
    fn as_slice(&self) -> &[usize] {
        match self {
            Self::One(row) => std::slice::from_ref(row),
            Self::Many(rows) => rows,
        }
    }
}

/// A scored set of payments and invoices that has not been assigned yet.
///
/// Both sides are non-empty and hold ascending input indices; one of them has one entry.
struct Candidate {
    anchor: Anchor,
    payments: Members,
    invoices: Members,
    score: f64,
    signals: Signals,
    /// The group was searched in a pool cut to [`GROUP_CANDIDATE_LIMIT`] rows.
    truncated: bool,
}

impl Candidate {
    /// Returns whether the candidate links more than one payment or more than one invoice.
    fn is_group(&self) -> bool {
        self.payments.as_slice().len() + self.invoices.as_slice().len() > 2
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
    invoice_name: Names,
    payment_reference: Vec<Reference>,
    payment_name: Names,
    /// Character counts of each normalized invoice number, built when the fuzzy reference
    /// prefilter first needs them.
    invoice_number_counts: Vec<LazyCounts>,
    /// Cache of the name similarity of each pair of distinct names.
    name_scores: NameScores,
    /// Whether a candidate with no reference signal can neither be accepted nor make another
    /// candidate of its anchor ambiguous; see [`cannot_matter`].
    skip_unnamed: bool,
}

/// Normalized counterparty names of one side, stored once per distinct text.
struct Names {
    /// For each input row, the position of its name in `distinct`.
    ids: Vec<usize>,
    /// The distinct normalized names, in the order of the raw texts they first come from.
    distinct: Vec<String>,
}

impl Names {
    /// Normalizes the distinct texts of `rows`; `text` reads the raw name of a row.
    fn new<T: Sync>(rows: &[T], parallel: bool, text: impl Fn(&T) -> &str + Sync + Send) -> Self {
        let mut raw: Vec<&str> = if parallel {
            // Default hasher, as for `position` below.
            rows.par_iter()
                .fold(HashSet::new, |mut seen, row| {
                    seen.insert(text(row));
                    seen
                })
                .reduce(HashSet::new, |mut all, seen| {
                    all.extend(seen);
                    all
                })
                .into_iter()
                .collect()
        } else {
            rows.iter()
                .map(&text)
                .collect::<HashSet<_>>()
                .into_iter()
                .collect()
        };
        // Sorted, so the layout does not depend on how the threads were scheduled.
        raw.sort_unstable();
        // Default hasher: the texts are counterparty names from the caller or a bank feed, and
        // a fixed-constant hasher would let crafted names collide.
        let position: HashMap<&str, usize> = raw.iter().enumerate().map(|(k, &t)| (t, k)).collect();
        let normalized = map_rows(&raw, parallel, |&t| normalize_name(t));
        // Different raw texts often normalize to the same name ("Acme S.L." and "ACME").
        let mut distinct: Vec<String> = Vec::new();
        // Default hasher: keyed by normalized names derived from caller text.
        let mut known: HashMap<&str, usize> = HashMap::new();
        let name_of_raw: Vec<usize> = normalized
            .iter()
            .map(|name| {
                *known.entry(name.as_str()).or_insert_with(|| {
                    distinct.push(name.clone());
                    distinct.len() - 1
                })
            })
            .collect();
        Self {
            ids: map_rows(rows, parallel, |row| name_of_raw[position[text(row)]]),
            distinct,
        }
    }

    /// Returns the normalized name of the row with input index `row`.
    fn get(&self, row: usize) -> &str {
        &self.distinct[self.ids[row]]
    }
}

/// Largest number of distinct name pairs whose similarity is cached, 32 MB of scores.
// ponytail: above this many distinct pairs the similarity is computed per candidate, as
// before. Upgrade: a bounded cache that evicts, if such data turns up.
const NAME_SCORE_LIMIT: usize = 1 << 22;

/// The bits that mark a cell of [`NameScores`] as not computed yet: a NaN, which no score is.
const UNSET: u64 = u64::MAX;

/// Name similarity of each pair of distinct names, computed on first use.
///
/// The cells are relaxed atomics: two threads may compute the same pair, and both store the
/// same bits, so no lock is needed and the result never depends on the schedule.
struct NameScores {
    /// Number of distinct invoice names, the width of a row of `cells`.
    width: usize,
    /// One cell per pair, or empty when there are too many pairs to cache.
    cells: Vec<AtomicU64>,
}

impl NameScores {
    /// Starts with nothing computed for `payments` times `invoices` distinct names; caches
    /// nothing when that is more than `limit` pairs.
    fn new(payments: usize, invoices: usize, limit: usize) -> Self {
        let pairs = payments.saturating_mul(invoices);
        Self {
            width: invoices,
            cells: if pairs <= limit {
                (0..pairs).map(|_| AtomicU64::new(UNSET)).collect()
            } else {
                Vec::new()
            },
        }
    }

    /// Returns the score of the pair of distinct names `(payment, invoice)`, calling
    /// `compute` only when it is not cached yet.
    fn get(&self, payment: usize, invoice: usize, compute: impl FnOnce() -> f64) -> f64 {
        let Some(cell) = self.cells.get(payment * self.width + invoice) else {
            return compute();
        };
        let bits = cell.load(Ordering::Relaxed);
        if bits != UNSET {
            return f64::from_bits(bits);
        }
        let score = compute();
        cell.store(score.to_bits(), Ordering::Relaxed);
        score
    }
}

/// Most distinct number lengths for which numbers are looked up by hash instead of scanned
/// with an automaton.
// ponytail: building an automaton for tens of thousands of numbers costs about three times
// its scan, while each length adds one hash lookup per byte of the reference. Upgrade: pick
// per partition from measured costs if lengths vary widely.
const HASHED_LENGTHS_MAX: usize = 3;

/// Finds the invoice numbers that occur in a text, with the invoices that carry each number.
enum NumberIndex<'a> {
    /// Looks up every substring of one of the few lengths that numbers have.
    Lengths {
        lengths: Vec<usize>,
        /// Whether some number starts with this byte; other positions are not looked up.
        first_bytes: Box<[bool; 256]>,
        /// Keyed by caller text, so it uses the default hasher.
        groups: HashMap<&'a str, Vec<usize>>,
    },
    /// Scans the text once for all numbers.
    Automaton {
        automaton: AhoCorasick,
        groups: Vec<Vec<usize>>,
    },
}

impl<'a> NumberIndex<'a> {
    /// Indexes the distinct non-empty `numbers`, each with its invoices; at most
    /// `max_lengths` distinct byte lengths are looked up by hash, otherwise an automaton
    /// is built.
    fn new(numbers: HashMap<&'a str, Vec<usize>>, max_lengths: usize) -> Result<Self, Error> {
        let mut lengths: Vec<usize> = numbers.keys().map(|number| number.len()).collect();
        lengths.sort_unstable();
        lengths.dedup();
        if lengths.len() <= max_lengths {
            let mut first_bytes = Box::new([false; 256]);
            for number in numbers.keys() {
                if let Some(&byte) = number.as_bytes().first() {
                    first_bytes[usize::from(byte)] = true;
                }
            }
            return Ok(Self::Lengths {
                lengths,
                first_bytes,
                groups: numbers,
            });
        }
        let (keys, groups): (Vec<&str>, Vec<Vec<usize>>) = numbers.into_iter().unzip();
        let automaton = AhoCorasick::builder()
            .match_kind(AcMatchKind::Standard)
            .build(&keys)
            .map_err(|e| Error::ReferenceIndex(e.to_string()))?;
        Ok(Self::Automaton { automaton, groups })
    }

    /// Calls `visit` with the byte range and the invoices of every occurrence of a number in
    /// `text`, overlapping ones included.
    fn find(&self, text: &str, mut visit: impl FnMut(usize, usize, &[usize])) {
        match self {
            Self::Lengths {
                lengths,
                first_bytes,
                groups,
            } => {
                for (start, &byte) in text.as_bytes().iter().enumerate() {
                    if !first_bytes[usize::from(byte)] {
                        continue;
                    }
                    for &length in lengths {
                        // `get` is `None` past the end and off a character boundary.
                        let rows = text
                            .get(start..start + length)
                            .and_then(|piece| groups.get(piece));
                        if let Some(rows) = rows {
                            visit(start, start + length, rows);
                        }
                    }
                }
            }
            Self::Automaton { automaton, groups } => {
                for hit in automaton.find_overlapping_iter(text) {
                    visit(hit.start(), hit.end(), &groups[hit.pattern().as_usize()]);
                }
            }
        }
    }
}

/// Which rows are already matched in one partition, plus the matches found so far.
///
/// The flags are shared by all partitions and indexed by input row. A partition only reads
/// and writes the flags of its own rows and partitions share no row, so relaxed atomics are
/// enough and a check is a single load.
struct Assigned<'a> {
    invoice_flags: &'a [AtomicBool],
    payment_flags: &'a [AtomicBool],
    /// Each match with the smallest input index of its payments, used for ordering.
    found: Vec<(usize, Match)>,
}

impl<'a> Assigned<'a> {
    /// Starts a partition with the shared flags of every row.
    fn new(invoice_flags: &'a [AtomicBool], payment_flags: &'a [AtomicBool]) -> Self {
        Self {
            invoice_flags,
            payment_flags,
            found: Vec::new(),
        }
    }

    /// Returns whether the invoice with input index `row` is already matched.
    fn invoice_taken(&self, row: usize) -> bool {
        self.invoice_flags[row].load(Ordering::Relaxed)
    }

    /// Returns whether the payment with input index `row` is already matched.
    fn payment_taken(&self, row: usize) -> bool {
        self.payment_flags[row].load(Ordering::Relaxed)
    }

    /// Marks the invoice with input index `row` as matched.
    fn take_invoice(&mut self, row: usize) {
        self.invoice_flags[row].store(true, Ordering::Relaxed);
    }

    /// Marks the payment with input index `row` as matched.
    fn take_payment(&mut self, row: usize) {
        self.payment_flags[row].store(true, Ordering::Relaxed);
    }
}

/// Runs the 1:1 engine; see [`crate::reconcile`] for the contract.
pub(crate) fn reconcile(
    invoices: &[Invoice],
    payments: &[Payment],
    config: &MatchConfig,
) -> Result<ReconciliationResult, Error> {
    // The owner is taken first: the id checks may already use the thread pool.
    let owner = *POOL_OWNER.get_or_init(std::process::id);
    let parallel = runs_in_parallel(owner, std::process::id());
    validate(invoices, payments, config, parallel)?;
    reconcile_validated(invoices, payments, config, parallel)
}

/// Maps every row with `f`, keeping the input order; the rows run on the thread pool only
/// when `parallel` is set.
fn map_rows<T: Sync, U: Send>(
    rows: &[T],
    parallel: bool,
    f: impl Fn(&T) -> U + Sync + Send,
) -> Vec<U> {
    if parallel {
        rows.par_iter().map(f).collect()
    } else {
        rows.iter().map(f).collect()
    }
}

/// Runs the engine on validated input, with the per-row setup and the partitions on the
/// thread pool when `parallel` is set.
fn reconcile_validated(
    invoices: &[Invoice],
    payments: &[Payment],
    config: &MatchConfig,
    parallel: bool,
) -> Result<ReconciliationResult, Error> {
    let invoice_number = map_rows(invoices, parallel, |i| normalize_reference(&i.number).text);
    let payment_reference = map_rows(payments, parallel, |p| normalize_reference(&p.reference));
    let invoice_name = Names::new(invoices, parallel, |i| i.counterparty.as_str());
    let payment_name = Names::new(payments, parallel, |p| p.counterparty.as_str());
    let engine = Engine {
        name_scores: NameScores::new(
            payment_name.distinct.len(),
            invoice_name.distinct.len(),
            NAME_SCORE_LIMIT,
        ),
        invoices,
        payments,
        config,
        weights: Weights::from_map(&config.weights),
        tolerance: u64::try_from(config.amount_tolerance_minor).unwrap_or(0),
        invoice_number_counts: (0..invoices.len()).map(|_| LazyCounts::default()).collect(),
        invoice_number,
        invoice_name,
        payment_reference,
        payment_name,
        skip_unnamed: cannot_matter(&Weights::from_map(&config.weights), config, 0.0),
    };
    let partitions = partitions(invoices, payments);
    let partitions: Vec<&PartitionRows> = partitions.values().collect();
    // Partitions share no rows. Each touches only the flags of its own rows and
    // keeps its own matches; the outcomes are collected in partition order, so the result
    // does not depend on how the threads are scheduled.
    let invoice_taken: Vec<AtomicBool> = (0..invoices.len())
        .map(|_| AtomicBool::new(false))
        .collect();
    let payment_taken: Vec<AtomicBool> = (0..payments.len())
        .map(|_| AtomicBool::new(false))
        .collect();
    let outcomes: Vec<Result<Vec<(usize, Match)>, Error>> = if parallel {
        partitions
            .par_iter()
            .map(|(invoice_rows, payment_rows)| {
                engine.run_partition(&invoice_taken, &payment_taken, invoice_rows, payment_rows)
            })
            .collect()
    } else {
        partitions
            .iter()
            .map(|(invoice_rows, payment_rows)| {
                engine.run_partition(&invoice_taken, &payment_taken, invoice_rows, payment_rows)
            })
            .collect()
    };
    let mut found = Vec::new();
    for outcome in outcomes {
        found.extend(outcome?);
    }
    found.sort_by(|a, b| b.1.score.total_cmp(&a.1.score).then_with(|| a.0.cmp(&b.0)));
    Ok(ReconciliationResult {
        matches: found.into_iter().map(|(_, m)| m).collect(),
        unmatched_payments: unmatched(payments.iter().map(|p| &p.id), &payment_taken),
        unmatched_invoices: unmatched(invoices.iter().map(|i| &i.id), &invoice_taken),
    })
}

/// Returns whether a candidate whose reference signal is at most `reference` can be ignored.
///
/// Its score is at most the one with the other three signals at 1. When that bound is below
/// the suggest threshold by at least the ambiguity margin (and also fails the acceptance
/// test, which matters for a margin of 0), the candidate cannot be accepted. It cannot make
/// an accepted candidate of its anchor ambiguous either, because that needs a score within
/// the margin of the accepted one, which scores at least the threshold.
fn cannot_matter(weights: &Weights, config: &MatchConfig, reference: f64) -> bool {
    let best = Signals {
        reference,
        amount: 1.0,
        name: 1.0,
        date: 1.0,
    };
    let bound = weighted_score(&best, weights);
    bound <= config.suggest_threshold - config.ambiguity_margin
        && !meets(bound, config.suggest_threshold)
}

/// The process that first ran `reconcile`, which is the one that owns the thread pool.
static POOL_OWNER: OnceLock<u32> = OnceLock::new();

/// Returns whether the process `current` may use the thread pool that `owner` started.
///
/// A process forked after its parent used rayon inherits the pool's bookkeeping but none of
/// its worker threads, so waiting for them would never return.
// ponytail: a child forked after the parent's first call runs sequentially, setup included.
// Upgrade: a per-process pool rebuilt after the fork (for example with a pthread_atfork hook).
fn runs_in_parallel(owner: u32, current: u32) -> bool {
    owner == current
}

/// Returns the ids whose rows were not taken, in input order.
fn unmatched<'a>(ids: impl Iterator<Item = &'a String>, taken: &[AtomicBool]) -> Vec<String> {
    ids.zip(taken)
        .filter(|&(_, t)| !t.load(Ordering::Relaxed))
        .map(|(id, _)| id.clone())
        .collect()
}

/// Rejects duplicate ids and out-of-range configuration.
///
/// The two id checks are independent and run side by side on the thread pool when `parallel`
/// is set; the error of the invoices still comes before the one of the payments.
fn validate(
    invoices: &[Invoice],
    payments: &[Payment],
    config: &MatchConfig,
    parallel: bool,
) -> Result<(), Error> {
    validate_config(config)?;
    let first_invoice = || first_repeat(invoices.iter().map(|i| i.id.as_str()));
    let first_payment = || first_repeat(payments.iter().map(|p| p.id.as_str()));
    let (invoice, payment) = if parallel {
        rayon::join(first_invoice, first_payment)
    } else {
        (first_invoice(), first_payment())
    };
    if let Some(id) = invoice {
        return Err(Error::DuplicateInvoiceId(id.to_owned()));
    }
    if let Some(id) = payment {
        return Err(Error::DuplicatePaymentId(id.to_owned()));
    }
    Ok(())
}

/// Returns the first id that repeats an earlier one, in iteration order.
fn first_repeat<'a>(ids: impl ExactSizeIterator<Item = &'a str>) -> Option<&'a str> {
    // Default hasher: ids are caller text, and crafted ids must not make the check quadratic.
    let mut seen = HashSet::with_capacity(ids.len());
    ids.into_iter().find(|&id| !seen.insert(id))
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

/// Groups the rows with a positive amount by scope and upper-cased currency.
fn partitions<'a>(
    invoices: &'a [Invoice],
    payments: &'a [Payment],
) -> BTreeMap<(&'a str, String), PartitionRows> {
    // Rows are first grouped by the raw pair, which costs no allocation per row.
    // Default hasher: scope and currency are caller text.
    let mut raw: HashMap<(&'a str, &'a str), PartitionRows> = HashMap::new();
    for (i, invoice) in invoices.iter().enumerate() {
        if invoice.amount_minor > 0 {
            let key = (invoice.scope.as_str(), invoice.currency.as_str());
            raw.entry(key).or_default().0.push(i);
        }
    }
    for (p, payment) in payments.iter().enumerate() {
        if payment.amount_minor > 0 {
            let key = (payment.scope.as_str(), payment.currency.as_str());
            raw.entry(key).or_default().1.push(p);
        }
    }
    let mut partitions: BTreeMap<(&str, String), PartitionRows> = BTreeMap::new();
    for ((scope, currency), (invoice_rows, payment_rows)) in raw {
        let entry = partitions
            .entry((scope, currency.to_ascii_uppercase()))
            .or_default();
        let merged = !entry.0.is_empty() || !entry.1.is_empty();
        entry.0.extend(invoice_rows);
        entry.1.extend(payment_rows);
        if merged {
            // Currencies that differ only in case share a partition; keep input order.
            entry.0.sort_unstable();
            entry.1.sort_unstable();
        }
    }
    partitions
}

impl Engine<'_> {
    /// Runs the reference passes, the 1:1 candidate pass and the counterparty group passes
    /// on one partition.
    fn run_partition(
        &self,
        invoice_taken: &[AtomicBool],
        payment_taken: &[AtomicBool],
        invoice_rows: &[usize],
        payment_rows: &[usize],
    ) -> Result<Vec<(usize, Match)>, Error> {
        let mut state = Assigned::new(invoice_taken, payment_taken);
        if !invoice_rows.is_empty() && !payment_rows.is_empty() {
            self.search_partition(invoice_rows, payment_rows, &mut state)?;
        }
        Ok(state.found)
    }

    /// Runs the passes on a partition that has rows on both sides.
    fn search_partition(
        &self,
        invoice_rows: &[usize],
        payment_rows: &[usize],
        state: &mut Assigned<'_>,
    ) -> Result<(), Error> {
        let (candidates, mentions) = self.reference_pass(invoice_rows, payment_rows)?;
        self.assign(candidates, state);
        let candidates = self.named_payment_groups(payment_rows, &mentions, state);
        self.assign(candidates, state);

        let mut open: Vec<(i64, usize)> = invoice_rows
            .iter()
            .filter(|&&i| !state.invoice_taken(i))
            .map(|&i| (self.invoices[i].amount_minor, i))
            .collect();
        open.sort_unstable();
        let candidates = payment_rows
            .iter()
            .filter(|&&p| !state.payment_taken(p))
            .flat_map(|&p| self.window_candidates(p, &open, mentions.contains_key(&p)))
            .collect();
        self.assign(candidates, state);

        self.match_invoice_groups_by_name(invoice_rows, payment_rows, &mentions, state);
        self.match_payment_groups_by_name(invoice_rows, payment_rows, &mentions, state);
        Ok(())
    }

    /// Returns the name signal of payment `p` and invoice `i`, cached per pair of distinct names.
    fn name_score(&self, p: usize, i: usize) -> f64 {
        let (payment, invoice) = (self.payment_name.ids[p], self.invoice_name.ids[i]);
        self.name_scores.get(payment, invoice, || {
            name_signal(
                &self.payment_name.distinct[payment],
                &self.invoice_name.distinct[invoice],
            )
        })
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
    /// minimum and the date signal the mean over every payment-invoice pair. `truncated`
    /// marks a group found in a pool that had to be cut.
    fn candidate(
        &self,
        anchor: Anchor,
        payments: Members,
        invoices: Members,
        reference: f64,
        truncated: bool,
    ) -> Candidate {
        let (pay_rows, inv_rows) = (payments.as_slice(), invoices.as_slice());
        let diff = self
            .payment_total(pay_rows)
            .abs_diff(self.invoice_total(inv_rows));
        let (mut name, mut date) = (1.0_f64, 0.0_f64);
        for &p in pay_rows {
            for &i in inv_rows {
                name = name.min(self.name_score(p, i));
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
            date: date / (pay_rows.len() * inv_rows.len()) as f64,
        };
        Candidate {
            anchor,
            payments,
            invoices,
            score: weighted_score(&signals, &self.weights),
            signals,
            truncated,
        }
    }

    /// Returns the subsets of `rows` whose amounts add up to `target` within the tolerance.
    ///
    /// `amount` reads a row's amount. Each subset lists ascending input indices; at most
    /// `max_group_size` rows per subset, none when that is below 2. Only the first
    /// [`GROUP_CANDIDATE_LIMIT`] rows are searched, so the caller orders them by priority;
    /// the flag says whether rows were left out, which makes the search incomplete.
    fn fitting_groups(
        &self,
        rows: &[usize],
        amount: impl Fn(usize) -> i64,
        target: i64,
    ) -> (Vec<Vec<usize>>, bool) {
        let max_size = self.config.max_group_size;
        if max_size < 2 {
            return (Vec::new(), false);
        }
        let truncated = rows.len() > GROUP_CANDIDATE_LIMIT;
        let rows = &rows[..rows.len().min(GROUP_CANDIDATE_LIMIT)];
        let amounts: Vec<i64> = rows.iter().map(|&r| amount(r)).collect();
        let groups = find_subsets(
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
        .collect();
        (groups, truncated)
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
        // Default hasher: invoice numbers are caller text.
        let mut by_number: HashMap<&str, Vec<usize>> = HashMap::new();
        for &i in invoice_rows {
            let number = self.invoice_number[i].as_str();
            if !number.is_empty() {
                by_number.entry(number).or_default().push(i);
            }
        }
        let mut candidates = Vec::new();
        let mut all_mentions = Mentions::default();
        if by_number.is_empty() {
            return Ok((candidates, all_mentions));
        }
        let index = NumberIndex::new(by_number, HASHED_LENGTHS_MAX)?;
        for &p in payment_rows {
            let reference = &self.payment_reference[p];
            let mut mentioned = BTreeSet::new();
            index.find(&reference.text, |start, end, rows| {
                if is_mention(reference, start, end) {
                    mentioned.extend(rows.iter().copied());
                }
            });
            if mentioned.is_empty() {
                continue;
            }
            for &i in &mentioned {
                let diff = self.payments[p]
                    .amount_minor
                    .abs_diff(self.invoices[i].amount_minor);
                if diff <= self.tolerance {
                    candidates.push(self.candidate(
                        Anchor::Payment(p),
                        Members::One(p),
                        Members::One(i),
                        1.0,
                        false,
                    ));
                }
            }
            let pool = oldest(mentioned.iter().copied().collect(), |i| {
                self.invoices[i].due_date
            });
            let (groups, truncated) = self.fitting_groups(
                &pool,
                |i| self.invoices[i].amount_minor,
                self.payments[p].amount_minor,
            );
            for group in groups {
                let c = self.candidate(
                    Anchor::Payment(p),
                    Members::One(p),
                    Members::Many(group),
                    1.0,
                    truncated,
                );
                candidates.push(c);
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
        state: &Assigned<'_>,
    ) -> Vec<Candidate> {
        let mut naming: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for &p in payment_rows.iter().filter(|&&p| !state.payment_taken(p)) {
            for &i in mentions.get(&p).into_iter().flatten() {
                if !state.invoice_taken(i) {
                    naming.entry(i).or_default().push(p);
                }
            }
        }
        let mut candidates = Vec::new();
        for (i, payments) in naming {
            let pool = oldest(payments, |p| self.payments[p].value_date);
            let (groups, truncated) = self.fitting_groups(
                &pool,
                |p| self.payments[p].amount_minor,
                self.invoices[i].amount_minor,
            );
            for group in groups {
                let c = self.candidate(
                    Anchor::Invoice(i),
                    Members::Many(group),
                    Members::One(i),
                    1.0,
                    truncated,
                );
                candidates.push(c);
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
        if explained && self.skip_unnamed {
            // Every candidate of an explained payment has reference 0: none can matter.
            return Vec::new();
        }
        let pay = &self.payments[payment];
        // Shared by the invoices this payment is compared with; counted at most once.
        let reference_counts = LazyCounts::default();
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
                    fuzzy_reference_signal_with(
                        &self.invoice_number[i],
                        &self.payment_reference[payment].text,
                        Some((&self.invoice_number_counts[i], &reference_counts)),
                    )
                };
                self.candidate(
                    Anchor::Payment(payment),
                    Members::One(payment),
                    Members::One(i),
                    reference,
                    false,
                )
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

    /// Matches each open payment, oldest first, to a group of its counterparty's invoices.
    ///
    /// Anchors are taken one at a time and each assigns before the next builds its pool, so
    /// a pool only holds rows that are still free. The pool holds the open invoices whose
    /// name reaches [`GROUP_NAME_THRESHOLD`], that fall inside the date window and that do
    /// not exceed the payment plus the tolerance. The reference signal is the share of pairs
    /// the payment names exactly.
    fn match_invoice_groups_by_name(
        &self,
        invoice_rows: &[usize],
        payment_rows: &[usize],
        mentions: &Mentions,
        state: &mut Assigned<'_>,
    ) {
        let invoices = by_name(
            invoice_rows,
            |r| state.invoice_taken(r),
            &self.invoice_name,
            |i| self.invoices[i].due_date,
        );
        let payments = oldest(
            payment_rows
                .iter()
                .copied()
                .filter(|&p| !state.payment_taken(p))
                .collect(),
            |p| self.payments[p].value_date,
        );
        let mut similar = SimilarBuckets::new(&invoices);
        for p in payments {
            // Without a free mentioned invoice every group has reference share 0.
            let names_one = mentions
                .get(&p)
                .is_some_and(|set| set.iter().any(|&i| !state.invoice_taken(i)));
            if self.skip_unnamed && !names_one {
                continue;
            }
            let mut candidates = Vec::new();
            let pay = &self.payments[p];
            let limit = i128::from(pay.amount_minor) + i128::from(self.tolerance);
            let pool = similar
                .get(self.payment_name.get(p))
                .iter()
                .flat_map(|rows| {
                    let near = within_days(
                        rows,
                        |i| self.invoices[i].due_date,
                        pay.value_date,
                        self.config.date_window_days,
                    );
                    near.iter().copied()
                })
                .filter(|&i| {
                    !state.invoice_taken(i) && i128::from(self.invoices[i].amount_minor) <= limit
                })
                .collect();
            let pool = oldest(pool, |i| self.invoices[i].due_date);
            let (groups, truncated) =
                self.fitting_groups(&pool, |i| self.invoices[i].amount_minor, pay.amount_minor);
            for group in groups {
                let reference = self.mention_share(mentions, &[p], &group);
                let c = self.candidate(
                    Anchor::Payment(p),
                    Members::One(p),
                    Members::Many(group),
                    reference,
                    truncated,
                );
                candidates.push(c);
            }
            self.assign(candidates, state);
        }
    }

    /// Matches each open invoice, oldest first, to a group of its counterparty's payments.
    ///
    /// Mirror of [`Self::match_invoice_groups_by_name`]: anchors assign one at a time, and
    /// the pool holds the free payments whose name reaches [`GROUP_NAME_THRESHOLD`], inside
    /// the date window and not above the invoice plus the tolerance.
    fn match_payment_groups_by_name(
        &self,
        invoice_rows: &[usize],
        payment_rows: &[usize],
        mentions: &Mentions,
        state: &mut Assigned<'_>,
    ) {
        let payments = by_name(
            payment_rows,
            |r| state.payment_taken(r),
            &self.payment_name,
            |p| self.payments[p].value_date,
        );
        let invoices = oldest(
            invoice_rows
                .iter()
                .copied()
                .filter(|&i| !state.invoice_taken(i))
                .collect(),
            |i| self.invoices[i].due_date,
        );
        let mut similar = SimilarBuckets::new(&payments);
        // Invoices that some payment of the partition names; any other has share 0.
        let mut named: FastSet<usize> = FastSet::default();
        named.extend(mentions.values().flatten().copied());
        for i in invoices {
            if self.skip_unnamed && !named.contains(&i) {
                continue;
            }
            let mut candidates = Vec::new();
            let inv = &self.invoices[i];
            let limit = i128::from(inv.amount_minor) + i128::from(self.tolerance);
            let pool = similar
                .get(self.invoice_name.get(i))
                .iter()
                .flat_map(|rows| {
                    let near = within_days(
                        rows,
                        |p| self.payments[p].value_date,
                        inv.due_date,
                        self.config.date_window_days,
                    );
                    near.iter().copied()
                })
                .filter(|&p| {
                    !state.payment_taken(p) && i128::from(self.payments[p].amount_minor) <= limit
                })
                .collect();
            let pool = oldest(pool, |p| self.payments[p].value_date);
            let (groups, truncated) =
                self.fitting_groups(&pool, |p| self.payments[p].amount_minor, inv.amount_minor);
            for group in groups {
                let reference = self.mention_share(mentions, &group, &[i]);
                let c = self.candidate(
                    Anchor::Invoice(i),
                    Members::Many(group),
                    Members::One(i),
                    reference,
                    truncated,
                );
                candidates.push(c);
            }
            self.assign(candidates, state);
        }
    }

    /// Greedily accepts the best candidates whose payments and invoices are all free.
    ///
    /// A match is ambiguous when its anchor's two best candidates in this pass (any score,
    /// taken or not) are closer than the ambiguity margin, or when it is a group and its
    /// anchor had more than one fitting group in this pass, or when its group came from a
    /// pool cut to [`GROUP_CANDIDATE_LIMIT`] rows (the search was incomplete). A lone
    /// candidate is never ambiguous by the margin rule.
    fn assign(&self, mut candidates: Vec<Candidate>, state: &mut Assigned<'_>) {
        let config = self.config;
        let mut top_two: FastMap<Anchor, (f64, f64)> = FastMap::default();
        let mut groups_per_anchor: FastMap<Anchor, usize> = FastMap::default();
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
                .then_with(|| {
                    a.payments
                        .as_slice()
                        .first()
                        .cmp(&b.payments.as_slice().first())
                })
                .then_with(|| {
                    a.invoices
                        .as_slice()
                        .first()
                        .cmp(&b.invoices.as_slice().first())
                })
                .then_with(|| a.payments.as_slice().cmp(b.payments.as_slice()))
                .then_with(|| a.invoices.as_slice().cmp(b.invoices.as_slice()))
        });
        for c in candidates {
            let (pay_rows, inv_rows) = (c.payments.as_slice(), c.invoices.as_slice());
            if pay_rows.iter().any(|&p| state.payment_taken(p))
                || inv_rows.iter().any(|&i| state.invoice_taken(i))
            {
                continue;
            }
            for &p in pay_rows {
                state.take_payment(p);
            }
            for &i in inv_rows {
                state.take_invoice(i);
            }
            let (first, second) = top_two[&c.anchor];
            let several_groups = c.is_group() && groups_per_anchor.get(&c.anchor) > Some(&1);
            let unsure = several_groups || c.truncated;
            let status = if first - second + EPS < config.ambiguity_margin || unsure {
                MatchStatus::Ambiguous
            } else if meets(c.score, config.auto_threshold) {
                MatchStatus::Auto
            } else {
                MatchStatus::Suggested
            };
            let kind = match (pay_rows.len(), inv_rows.len()) {
                (1, 1) => MatchKind::OneToOne,
                (1, _) => MatchKind::OneToMany,
                _ => MatchKind::ManyToOne,
            };
            // The difference is within the tolerance, which is a non-negative i64.
            let residual =
                i64::try_from(self.payment_total(pay_rows) - self.invoice_total(inv_rows))
                    .unwrap_or_default();
            state.found.push((
                pay_rows.first().copied().unwrap_or_default(),
                Match {
                    kind,
                    status,
                    payment_ids: pay_rows
                        .iter()
                        .map(|&p| self.payments[p].id.clone())
                        .collect(),
                    invoice_ids: inv_rows
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

/// Buckets the free rows by normalized name, each bucket sorted by date and then input index,
/// so [`within_days`] can cut the date window out of it.
fn by_name<'a>(
    rows: &[usize],
    taken: impl Fn(usize) -> bool,
    names: &'a Names,
    date: impl Fn(usize) -> NaiveDate,
) -> BTreeMap<&'a str, Vec<usize>> {
    let mut buckets: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for &row in rows.iter().filter(|&&r| !taken(r)) {
        buckets.entry(names.get(row)).or_default().push(row);
    }
    for bucket in buckets.values_mut() {
        bucket.sort_unstable_by_key(|&r| (date(r), r));
    }
    buckets
}

/// Returns the rows whose date is at most `days` days from `center`, as a slice of `rows`,
/// which must be sorted by date.
fn within_days(
    rows: &[usize],
    date: impl Fn(usize) -> NaiveDate,
    center: NaiveDate,
    days: u32,
) -> &[usize] {
    let low = center
        .checked_sub_days(Days::new(days.into()))
        .unwrap_or(NaiveDate::MIN);
    let high = center
        .checked_add_days(Days::new(days.into()))
        .unwrap_or(NaiveDate::MAX);
    let start = rows.partition_point(|&r| date(r) < low);
    let end = rows.partition_point(|&r| date(r) <= high);
    &rows[start..end]
}

/// Finds, for each distinct anchor name, the buckets whose name reaches the threshold.
///
/// Names are compared once per distinct pair, not once per row pair, and only for the anchor
/// names that are asked for: an anchor that is skipped costs nothing.
// ponytail: distinct anchor names x distinct bucket names Jaro-Winkler comparisons per
// partition. Upgrade: block the names (first letters or n-grams) before comparing if a
// partition holds tens of thousands of distinct names.
struct SimilarBuckets<'a, 'b> {
    buckets: &'b BTreeMap<&'a str, Vec<usize>>,
    /// Keyed by names from caller text, so it uses the default hasher.
    cache: HashMap<&'b str, Vec<&'b Vec<usize>>>,
}

impl<'a, 'b> SimilarBuckets<'a, 'b> {
    /// Starts with no name compared.
    fn new(buckets: &'b BTreeMap<&'a str, Vec<usize>>) -> Self {
        Self {
            buckets,
            cache: HashMap::new(),
        }
    }

    /// Returns the buckets similar to `name`, comparing on the first request for that name.
    fn get(&mut self, name: &'b str) -> &[&'b Vec<usize>] {
        let buckets = self.buckets;
        self.cache.entry(name).or_insert_with(|| {
            buckets
                .iter()
                .filter(|&(&other, _)| meets(name_signal(name, other), GROUP_NAME_THRESHOLD))
                .map(|(_, rows)| rows)
                .collect()
        })
    }
}

/// Orders rows by date and then input index, so a truncated pool keeps the oldest rows.
fn oldest(mut rows: Vec<usize>, date: impl Fn(usize) -> NaiveDate) -> Vec<usize> {
    rows.sort_by_key(|&r| (date(r), r));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds `scopes` partitions of a few rows each: named, unnamed and grouped payments.
    fn many_partitions(scopes: usize) -> (Vec<Invoice>, Vec<Payment>) {
        let day = |d: u64| NaiveDate::from_ymd_opt(2024, 1, 1).unwrap() + chrono::Days::new(d);
        let (mut invoices, mut payments) = (Vec::new(), Vec::new());
        for s in 0..scopes {
            for k in 0..6_i64 {
                invoices.push(Invoice {
                    id: format!("i{s}-{k}"),
                    number: format!("INV-{s}-{k}"),
                    amount_minor: 1_000 + 100 * (k % 3),
                    currency: "EUR".to_owned(),
                    issue_date: day(0),
                    due_date: day(10 + k as u64),
                    counterparty: "Acme S.L.".to_owned(),
                    scope: format!("scope-{s}"),
                });
            }
            for k in 0..5_i64 {
                payments.push(Payment {
                    id: format!("p{s}-{k}"),
                    amount_minor: 1_000 + 100 * (k % 3) + if k == 4 { 1_000 } else { 0 },
                    currency: "EUR".to_owned(),
                    value_date: day(12),
                    reference: if k % 2 == 0 {
                        format!("pago INV-{s}-{k}")
                    } else {
                        "thanks".to_owned()
                    },
                    counterparty: "ACME".to_owned(),
                    scope: format!("scope-{s}"),
                });
            }
        }
        (invoices, payments)
    }

    #[test]
    fn map_rows_keeps_the_input_order_in_both_modes() {
        let rows: Vec<u32> = (0..1_000).collect();
        let expected: Vec<u32> = rows.iter().map(|n| n * 2).collect();
        assert_eq!(map_rows(&rows, true, |n| n * 2), expected);
        assert_eq!(map_rows(&rows, false, |n| n * 2), expected);
    }

    #[test]
    fn map_rows_leaves_the_thread_pool_alone_unless_asked() {
        // The test thread is not a pool worker, so a parallel call hands every row to a
        // worker and a sequential call keeps every row here.
        let rows: Vec<u32> = (0..1_000).collect();
        let on_worker =
            |parallel: bool| map_rows(&rows, parallel, |_| rayon::current_thread_index());
        assert!(on_worker(false).iter().all(Option::is_none));
        assert!(on_worker(true).iter().all(Option::is_some));
    }

    #[test]
    fn a_sequential_run_gives_the_same_result_as_a_parallel_one() {
        let (invoices, payments) = many_partitions(30);
        let config = MatchConfig {
            suggest_threshold: 0.3,
            ..MatchConfig::default()
        };
        let parallel = reconcile_validated(&invoices, &payments, &config, true).unwrap();
        let sequential = reconcile_validated(&invoices, &payments, &config, false).unwrap();
        assert!(parallel.matches.len() > 30, "the data must produce matches");
        assert_eq!(parallel, sequential);
    }

    #[test]
    fn names_are_normalized_once_per_distinct_text_and_stored_once_per_distinct_name() {
        let raw = ["Acme S.L.", "ACME", "Acme S.L.", "Beta Inc.", "ACME", ""];
        let build = |parallel: bool| Names::new(&raw, parallel, |text| text);
        for parallel in [false, true] {
            let names = build(parallel);
            assert_eq!(
                names.distinct.len(),
                3,
                "one entry per distinct normalized name"
            );
            for (row, text) in raw.iter().enumerate() {
                assert_eq!(names.get(row), normalize_name(text), "row {row}");
            }
        }
        let (sequential, parallel) = (build(false), build(true));
        assert_eq!(sequential.ids, parallel.ids);
        assert_eq!(sequential.distinct, parallel.distinct);
    }

    #[test]
    fn currencies_that_differ_only_in_case_share_a_partition_in_input_order() {
        let (invoices, payments) = many_partitions(2);
        let recase =
            |k: usize, currency: &str| k.is_multiple_of(2).then(|| currency.to_ascii_lowercase());
        let invoices: Vec<Invoice> = invoices
            .into_iter()
            .enumerate()
            .map(|(k, i)| Invoice {
                currency: recase(k, &i.currency).unwrap_or(i.currency),
                ..i
            })
            .collect();
        let payments: Vec<Payment> = payments
            .into_iter()
            .enumerate()
            .map(|(k, p)| Payment {
                currency: recase(k + 1, &p.currency).unwrap_or(p.currency),
                ..p
            })
            .collect();
        let groups = partitions(&invoices, &payments);
        assert_eq!(groups.len(), 2, "one partition per scope");
        for (_, (invoice_rows, payment_rows)) in groups {
            assert!(invoice_rows.is_sorted() && payment_rows.is_sorted());
            assert_eq!((invoice_rows.len(), payment_rows.len()), (6, 5));
        }
    }

    #[test]
    fn name_scores_are_computed_once_per_distinct_pair() {
        let scores = NameScores::new(3, 2, 6);
        let calls = std::cell::Cell::new(0);
        let get = |p: usize, i: usize| {
            scores.get(p, i, || {
                calls.set(calls.get() + 1);
                (p * 10 + i) as f64 / 100.0
            })
        };
        for _ in 0..3 {
            for (p, i) in [(0, 0), (2, 1), (1, 0), (2, 1)] {
                assert_eq!(get(p, i), (p * 10 + i) as f64 / 100.0);
            }
        }
        assert_eq!(calls.get(), 3, "three distinct pairs were asked for");
    }

    #[test]
    fn name_scores_keep_a_zero_and_skip_the_cache_over_the_limit() {
        let scores = NameScores::new(2, 2, 3);
        let calls = std::cell::Cell::new(0);
        for _ in 0..2 {
            assert_eq!(
                scores.get(1, 1, || {
                    calls.set(calls.get() + 1);
                    0.0
                }),
                0.0
            );
        }
        assert_eq!(calls.get(), 2, "four pairs exceed the limit of three");
        let cached = NameScores::new(2, 2, 4);
        for _ in 0..2 {
            assert_eq!(cached.get(1, 1, || 0.0), 0.0);
        }
    }

    #[test]
    fn within_days_returns_exactly_the_rows_inside_the_window() {
        let day =
            |d: i64| NaiveDate::from_ymd_opt(2024, 3, 1).unwrap() + chrono::Days::new(d as u64);
        let dates: Vec<NaiveDate> = [0, 1, 1, 5, 9, 10, 10, 11, 30].map(day).to_vec();
        let rows: Vec<usize> = (0..dates.len()).collect();
        for center in [day(0), day(10), day(30), day(100)] {
            for window in [0, 1, 5, 10, 365] {
                let expected: Vec<usize> = rows
                    .iter()
                    .copied()
                    .filter(|&r| {
                        center
                            .signed_duration_since(dates[r])
                            .num_days()
                            .unsigned_abs()
                            <= u64::from(window)
                    })
                    .collect();
                assert_eq!(
                    within_days(&rows, |r| dates[r], center, window),
                    expected,
                    "center {center} window {window}"
                );
            }
        }
        let first = [NaiveDate::MIN];
        assert_eq!(
            within_days(&[0], |r| first[r], NaiveDate::MIN, u32::MAX),
            [0]
        );
        let last = [NaiveDate::MAX];
        assert_eq!(
            within_days(&[0], |r| last[r], NaiveDate::MAX, u32::MAX),
            [0]
        );
    }

    /// Every occurrence the index reports in `text`, sorted.
    fn occurrences(index: &NumberIndex<'_>, text: &str) -> Vec<(usize, usize, Vec<usize>)> {
        let mut found = Vec::new();
        index.find(text, |start, end, rows| {
            found.push((start, end, rows.to_vec()))
        });
        found.sort();
        found
    }

    #[test]
    fn numbers_are_found_by_lookup_and_by_automaton_alike() {
        let numbers = || {
            vec![
                ("INV1", vec![0, 4]),
                ("INV12", vec![1]),
                ("12", vec![2]),
                ("É1", vec![3]),
            ]
        };
        let by_lookup = NumberIndex::new(numbers().into_iter().collect(), 4).unwrap();
        let by_automaton = NumberIndex::new(numbers().into_iter().collect(), 0).unwrap();
        assert!(matches!(by_lookup, NumberIndex::Lengths { .. }));
        assert!(matches!(by_automaton, NumberIndex::Automaton { .. }));
        for text in ["", "INV12", "PAGO INV1 INV12 12INV1", "É1É1 INV121", "INV"] {
            assert_eq!(
                occurrences(&by_lookup, text),
                occurrences(&by_automaton, text),
                "{text}"
            );
        }
        let overlapping = occurrences(&by_lookup, "INV12");
        assert_eq!(
            overlapping,
            vec![(0, 4, vec![0, 4]), (0, 5, vec![1]), (3, 5, vec![2])]
        );
    }

    #[test]
    fn more_lengths_than_the_limit_use_the_automaton() {
        let numbers = vec![("AB", vec![0]), ("ABC", vec![1]), ("ABCD", vec![2])];
        assert!(matches!(
            NumberIndex::new(numbers.iter().cloned().collect(), 3).unwrap(),
            NumberIndex::Lengths { .. }
        ));
        assert!(matches!(
            NumberIndex::new(numbers.into_iter().collect(), 2).unwrap(),
            NumberIndex::Automaton { .. }
        ));
    }

    proptest::proptest! {
        #[test]
        fn lookup_and_automaton_find_the_same_occurrences(
            patterns in proptest::collection::btree_set("[AB1\u{c9}]{1,4}", 1..12),
            text in "[AB1\u{c9} ]{0,30}",
        ) {
            let numbers: Vec<(&str, Vec<usize>)> = patterns
                .iter()
                .enumerate()
                .map(|(k, p)| (p.as_str(), vec![k, k + 100]))
                .collect();
            let by_lookup = NumberIndex::new(numbers.iter().cloned().collect(), usize::MAX).unwrap();
            let by_automaton = NumberIndex::new(numbers.into_iter().collect(), 0).unwrap();
            proptest::prop_assert_eq!(
                occurrences(&by_lookup, &text),
                occurrences(&by_automaton, &text)
            );
        }
    }

    #[test]
    fn duplicate_ids_are_reported_in_the_same_order_in_both_modes() {
        let (mut invoices, mut payments) = many_partitions(2);
        let config = MatchConfig::default();
        for parallel in [false, true] {
            assert!(validate(&invoices, &payments, &config, parallel).is_ok());
        }
        payments[3].id = payments[1].id.clone();
        for parallel in [false, true] {
            let error = validate(&invoices, &payments, &config, parallel).unwrap_err();
            assert!(matches!(error, Error::DuplicatePaymentId(ref id) if *id == payments[1].id));
        }
        invoices[5].id = invoices[2].id.clone();
        for parallel in [false, true] {
            // Both sides repeat an id; the invoice error comes first, as in a sequential check.
            let error = validate(&invoices, &payments, &config, parallel).unwrap_err();
            assert!(matches!(error, Error::DuplicateInvoiceId(ref id) if *id == invoices[2].id));
        }
        let bad = MatchConfig {
            auto_threshold: 2.0,
            ..MatchConfig::default()
        };
        assert!(matches!(
            validate(&invoices, &payments, &bad, true),
            Err(Error::InvalidConfig(_))
        ));
    }

    #[test]
    fn partitions_run_in_parallel_only_in_the_process_that_started_the_pool() {
        assert!(runs_in_parallel(7, 7));
        assert!(!runs_in_parallel(7, 8), "a forked child has no workers");
    }

    #[test]
    fn the_default_weights_prune_candidates_without_a_reference_signal() {
        let config = MatchConfig::default();
        let weights = Weights::from_map(&config.weights);
        // 0.55 against 0.60 - 0.05: the bound holds, so the skip really applies.
        assert!(cannot_matter(&weights, &config, 0.0));
        // With any reference signal the bound rises above the threshold minus the margin.
        assert!(!cannot_matter(&weights, &config, 0.2));
        assert!(!cannot_matter(&weights, &config, 1.0));
    }

    #[test]
    fn a_zero_margin_never_prunes_a_candidate_that_can_be_accepted() {
        // The bound 0.55 equals the threshold, so a perfect candidate would be accepted.
        let config = MatchConfig {
            suggest_threshold: 0.55,
            ambiguity_margin: 0.0,
            ..MatchConfig::default()
        };
        let weights = Weights::from_map(&config.weights);
        assert!(!cannot_matter(&weights, &config, 0.0));
    }

    #[test]
    fn the_result_does_not_depend_on_the_number_of_threads() {
        let (invoices, payments) = many_partitions(60);
        let config = MatchConfig {
            suggest_threshold: 0.3,
            ..MatchConfig::default()
        };
        let run = |threads: usize| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| reconcile(&invoices, &payments, &config).unwrap())
        };
        let single = run(1);
        assert!(single.matches.len() > 60, "the data must produce matches");
        assert_eq!(single, run(8));
        assert_eq!(single, run(32));
    }
}
