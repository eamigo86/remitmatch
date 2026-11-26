//! Signal functions and the weighted score of a payment-invoice candidate.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use chrono::NaiveDate;

/// Tolerance for comparing floating-point scores against thresholds.
pub(crate) const EPS: f64 = 1e-9;

/// Days around the due date that still score a perfect date signal.
const DATE_GRACE_DAYS: i64 = 3;

/// Minimum fuzzy similarity that counts as a partial reference signal.
const FUZZY_FLOOR: f64 = 0.8;

/// Shortest invoice number (in characters) eligible for fuzzy matching.
const FUZZY_MIN_LEN: usize = 4;

/// The four raw signals of one candidate, each in the range 0 to 1.
pub(crate) struct Signals {
    pub(crate) reference: f64,
    pub(crate) amount: f64,
    pub(crate) name: f64,
    pub(crate) date: f64,
}

/// Non-negative weights of the four signals.
pub(crate) struct Weights {
    pub(crate) reference: f64,
    pub(crate) amount: f64,
    pub(crate) name: f64,
    pub(crate) date: f64,
}

impl Weights {
    /// Reads the weights from the configuration map; a missing key counts as 0.
    pub(crate) fn from_map(map: &BTreeMap<String, f64>) -> Self {
        let get = |key: &str| map.get(key).copied().unwrap_or(0.0);
        Self {
            reference: get("reference"),
            amount: get("amount"),
            name: get("name"),
            date: get("date"),
        }
    }
}

/// Scores the amount difference: 1 when exact, linear down to 0 at the tolerance.
pub(crate) fn amount_signal(diff: u64, tolerance: i64) -> f64 {
    if diff == 0 {
        return 1.0;
    }
    match u64::try_from(tolerance) {
        // u64 to f64 loses precision only above 2^53 minor units, far beyond real amounts.
        Ok(t) if t > 0 => (1.0 - diff as f64 / t as f64).max(0.0),
        _ => 0.0,
    }
}

/// Scores two already normalized names with Jaro-Winkler; 0 when either is empty.
pub(crate) fn name_signal(payment_name: &str, invoice_name: &str) -> f64 {
    if payment_name.is_empty() || invoice_name.is_empty() {
        return 0.0;
    }
    strsim::jaro_winkler(payment_name, invoice_name)
}

/// Scores the distance between value date and due date against the window.
pub(crate) fn date_signal(value_date: NaiveDate, due_date: NaiveDate, window_days: u32) -> f64 {
    let d = value_date.signed_duration_since(due_date).num_days().abs();
    let window = i64::from(window_days);
    if d <= DATE_GRACE_DAYS {
        1.0
    } else if d >= window {
        0.0
    } else {
        1.0 - (d - DATE_GRACE_DAYS) as f64 / (window - DATE_GRACE_DAYS) as f64
    }
}

/// Finds the best approximate occurrence of `number` inside `reference`, without the
/// prefilter; the reference the filtered search is tested against.
#[cfg(test)]
pub(crate) fn fuzzy_reference_signal(number: &str, reference: &str) -> f64 {
    fuzzy_reference_signal_with(number, reference, None)
}

/// Character counts of a text, sorted by character, for the fuzzy reference prefilter.
pub(crate) struct CharCounts(Vec<(char, u32)>);

impl CharCounts {
    /// Counts the characters of `text`.
    pub(crate) fn of(text: &str) -> Self {
        let mut chars: Vec<char> = text.chars().collect();
        chars.sort_unstable();
        let mut counts: Vec<(char, u32)> = Vec::new();
        for c in chars {
            match counts.last_mut() {
                Some((last, n)) if *last == c => *n += 1,
                _ => counts.push((c, 1)),
            }
        }
        Self(counts)
    }

    /// Returns how many characters of `self` have no counterpart in `other` (multiset
    /// difference): a lower bound of the edit distance from `self` to any part of `other`.
    pub(crate) fn missing_from(&self, other: &Self) -> usize {
        let mut theirs = other.0.iter().peekable();
        let mut missing = 0;
        for &(c, n) in &self.0 {
            while theirs.next_if(|&&(o, _)| o < c).is_some() {}
            let available = theirs
                .peek()
                .map_or(0, |&&(o, m)| if o == c { m } else { 0 });
            missing += n.saturating_sub(available) as usize;
        }
        missing
    }
}

/// Character counts that are built on first use and kept, so a text that never reaches the
/// prefilter is never counted.
#[derive(Default)]
pub(crate) struct LazyCounts(OnceLock<CharCounts>);

impl LazyCounts {
    /// Returns the counts of `text`, building them on the first call; later calls ignore
    /// their argument, so every call must pass the same text.
    pub(crate) fn get(&self, text: &str) -> &CharCounts {
        self.0.get_or_init(|| CharCounts::of(text))
    }

    /// Returns whether the counts have been built.
    #[cfg(test)]
    fn is_built(&self) -> bool {
        self.0.get().is_some()
    }
}

/// Finds the best approximate occurrence of `number` inside `reference`.
///
/// Both arguments are normalized. Returns 0 when `number` occurs exactly: the caller only
/// gets here for payments with no accepted mention, so an exact occurrence was rejected by
/// the digit-boundary rule and names a different, longer number. Otherwise returns the best
/// similarity when it reaches the floor, else 0.
// ponytail: naive sliding window, O(len(reference) * len(number)^2) per candidate with an
// allocation per window (about 66 microseconds per pair in a release build; 400 x 400
// same-amount rows took 10.6 s). Upgrade: one approximate-substring DP (Sellers) per pair,
// and do not materialize candidates below the suggest threshold.
///
/// With `counts`, a prefilter on character counts runs first.
/// `counts` holds the lazily built counts of `number` and of `reference`. A similarity of at least the
/// floor allows at most `(len + 1) / 5` edits for a number of `len` characters (the windows
/// are at most `len + 1` long), and every edit repairs at most one character missing from the
/// reference, so a number with more missing characters scores 0 without the window search.
pub(crate) fn fuzzy_reference_signal_with(
    number: &str,
    reference: &str,
    counts: Option<(&LazyCounts, &LazyCounts)>,
) -> f64 {
    let n = number.chars().count();
    if n < FUZZY_MIN_LEN || reference.contains(number) {
        return 0.0;
    }
    if let Some((number_counts, reference_counts)) = counts
        && number_counts
            .get(number)
            .missing_from(reference_counts.get(reference))
            > (n + 1) / 5
    {
        return 0.0;
    }
    let best = if number.is_ascii() && reference.is_ascii() {
        best_ascii_window(number, reference, n)
    } else {
        best_char_window(number, reference, n)
    };
    if best >= FUZZY_FLOOR { best } else { 0.0 }
}

/// Returns the similarity of two texts of `len1` and `len2` characters at edit distance `dist`.
///
/// The same expression as [`strsim::normalized_damerau_levenshtein`], so the bits agree.
fn similarity(dist: usize, len1: usize, len2: usize) -> f64 {
    1.0 - (dist as f64) / (len1.max(len2) as f64)
}

/// Returns the largest edit distance whose similarity still reaches [`FUZZY_FLOOR`] for texts
/// of `len1` and `len2` characters.
fn max_edits(len1: usize, len2: usize) -> usize {
    let mut dist = 0;
    while similarity(dist + 1, len1, len2) >= FUZZY_FLOOR {
        dist += 1;
    }
    dist
}

/// Returns the best similarity of `number`, of `n` characters, to any window of `reference`
/// with `n - 1`, `n` or `n + 1` characters; both texts are ASCII.
///
/// A window that lacks more characters of `number` than the edit budget allows cannot be
/// close enough (every edit repairs at most one missing character), so the window's counts
/// are kept while it slides and only the windows within budget get the full distance.
fn best_ascii_window(number: &str, reference: &str, n: usize) -> f64 {
    let (wanted_bytes, text) = (number.as_bytes(), reference.as_bytes());
    let mut wanted = [0_u32; 128];
    for &b in wanted_bytes {
        wanted[usize::from(b)] += 1;
    }
    let mut best = 0.0_f64;
    for len in (n - 1)..=(n + 1) {
        let budget = max_edits(n, len);
        let mut have = [0_u32; 128];
        let mut missing = n;
        for (end, &added) in text.iter().enumerate() {
            let added = usize::from(added);
            if have[added] < wanted[added] {
                missing -= 1;
            }
            have[added] += 1;
            if end >= len {
                let dropped = usize::from(text[end - len]);
                have[dropped] -= 1;
                if have[dropped] < wanted[dropped] {
                    missing += 1;
                }
            }
            if end + 1 >= len && missing <= budget {
                let window = &reference[end + 1 - len..=end];
                let dist = strsim::damerau_levenshtein(number, window);
                best = best.max(similarity(dist, n, len));
            }
        }
    }
    best
}

/// Same as [`best_ascii_window`] for any text, one window at a time.
fn best_char_window(number: &str, reference: &str, n: usize) -> f64 {
    let chars: Vec<char> = reference.chars().collect();
    let mut best = 0.0_f64;
    for len in (n - 1)..=(n + 1) {
        for window in chars.windows(len) {
            let candidate: String = window.iter().collect();
            best = best.max(strsim::normalized_damerau_levenshtein(number, &candidate));
        }
    }
    best
}

/// Combines the signals into `sum(w * s) / sum(w)`.
///
/// The caller guarantees a positive weight sum (validated in the configuration).
pub(crate) fn weighted_score(signals: &Signals, weights: &Weights) -> f64 {
    let total = weights.reference + weights.amount + weights.name + weights.date;
    (weights.reference * signals.reference
        + weights.amount * signals.amount
        + weights.name * signals.name
        + weights.date * signals.date)
        / total
}

/// Returns the four raw signals keyed by `reference`, `amount`, `name` and `date`.
pub(crate) fn breakdown(signals: &Signals) -> BTreeMap<String, f64> {
    [
        ("reference", signals.reference),
        ("amount", signals.amount),
        ("name", signals.name),
        ("date", signals.date),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect()
}

/// Returns whether `score` reaches `threshold`, tolerating floating-point noise.
pub(crate) fn meets(score: f64, threshold: f64) -> bool {
    score + EPS >= threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 3, d).unwrap()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn amount_is_one_when_exact_and_linear_to_the_tolerance() {
        assert!(close(amount_signal(0, 0), 1.0));
        assert!(close(amount_signal(0, 100), 1.0));
        assert!(close(amount_signal(25, 100), 0.75));
        assert!(close(amount_signal(100, 100), 0.0));
    }

    #[test]
    fn amount_outside_tolerance_or_without_tolerance_is_zero() {
        assert!(close(amount_signal(150, 100), 0.0));
        assert!(close(amount_signal(1, 0), 0.0));
    }

    #[test]
    fn name_uses_jaro_winkler_and_is_zero_when_empty() {
        assert!(close(name_signal("acme", "acme"), 1.0));
        assert!(name_signal("acme", "acne") > 0.8);
        assert!(close(name_signal("", "acme"), 0.0));
        assert!(close(name_signal("acme", ""), 0.0));
    }

    #[test]
    fn date_is_one_within_the_grace_days() {
        assert!(close(date_signal(day(10), day(10), 30), 1.0));
        assert!(close(date_signal(day(13), day(10), 30), 1.0));
        assert!(close(date_signal(day(7), day(10), 30), 1.0));
    }

    #[test]
    fn date_decreases_linearly_and_is_zero_at_the_window() {
        // Six days apart: (6 - 3) of the (30 - 3) days of decay are used.
        assert!(close(date_signal(day(16), day(10), 30), 1.0 - 3.0 / 27.0));
        assert!(close(date_signal(day(10), day(1), 9), 0.0));
        assert!(close(date_signal(day(10), day(1), 5), 0.0));
    }

    #[test]
    fn fuzzy_reference_scores_a_misspelling_partially() {
        let s = fuzzy_reference_signal("INV2024001", "PAGO INV2024010 GRACIAS");
        assert!((0.8..1.0).contains(&s), "got {s}");
    }

    #[test]
    fn fuzzy_reference_is_zero_below_the_similarity_floor() {
        assert!(close(
            fuzzy_reference_signal("INV2024001", "TRANSFER 55 RENT"),
            0.0
        ));
    }

    #[test]
    fn fuzzy_reference_is_zero_when_the_number_occurs_exactly() {
        // An exact occurrence that reaches the fuzzy path was rejected as a mention
        // (it is glued to other digits), so it names a different, longer number.
        assert!(close(fuzzy_reference_signal("INV1", "INV10"), 0.0));
        assert!(close(fuzzy_reference_signal("1001", "PAGOFRA10010"), 0.0));
    }

    #[test]
    fn fuzzy_reference_is_zero_for_short_numbers() {
        assert!(close(fuzzy_reference_signal("A12", "A12"), 0.0));
    }

    #[test]
    fn weights_missing_keys_count_as_zero() {
        let map = BTreeMap::from([("amount".to_owned(), 2.0)]);
        let w = Weights::from_map(&map);
        assert!(close(w.amount, 2.0));
        assert!(close(w.reference + w.name + w.date, 0.0));
    }

    #[test]
    fn weighted_score_is_the_normalized_weighted_sum() {
        let w = Weights {
            reference: 1.0,
            amount: 1.0,
            name: 2.0,
            date: 0.0,
        };
        let s = Signals {
            reference: 1.0,
            amount: 0.0,
            name: 0.5,
            date: 1.0,
        };
        assert!(close(weighted_score(&s, &w), 0.5));
    }

    #[test]
    fn breakdown_holds_the_four_raw_signals() {
        let s = Signals {
            reference: 1.0,
            amount: 0.5,
            name: 0.25,
            date: 0.0,
        };
        let b = breakdown(&s);
        let keys: Vec<&str> = b.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["amount", "date", "name", "reference"]);
        assert!(close(b["amount"], 0.5));
        assert!(close(b["reference"], 1.0));
    }

    #[test]
    fn threshold_helper_tolerates_float_noise() {
        assert!(meets(0.90 - 1e-12, 0.90));
        assert!(meets(0.90, 0.90));
        assert!(!meets(0.89, 0.90));
    }

    /// Scores with the prefilter on the character counts of both strings.
    fn counted(number: &str, reference: &str) -> f64 {
        fuzzy_reference_signal_with(
            number,
            reference,
            Some((&LazyCounts::default(), &LazyCounts::default())),
        )
    }

    #[test]
    fn counts_are_built_only_when_the_prefilter_needs_them() {
        // A number below the minimum length and a number found verbatim never reach it.
        for (number, reference) in [("A12", "PAGO A13"), ("INV2024001", "PAGO INV2024001")] {
            let (number_counts, reference_counts) = (LazyCounts::default(), LazyCounts::default());
            fuzzy_reference_signal_with(
                number,
                reference,
                Some((&number_counts, &reference_counts)),
            );
            assert!(
                !number_counts.is_built() && !reference_counts.is_built(),
                "{number}"
            );
        }
        // A partial occurrence does, and each side is counted from its own text.
        let (number_counts, reference_counts) = (LazyCounts::default(), LazyCounts::default());
        let score = fuzzy_reference_signal_with(
            "INV2024001",
            "PAGO INV2024010 GRACIAS",
            Some((&number_counts, &reference_counts)),
        );
        assert!(score >= FUZZY_FLOOR);
        assert!(number_counts.is_built() && reference_counts.is_built());
        assert_eq!(
            number_counts
                .get("ignored")
                .missing_from(reference_counts.get("ignored")),
            0
        );
    }

    #[test]
    fn counts_report_the_characters_missing_from_a_text() {
        let missing = |a: &str, b: &str| CharCounts::of(a).missing_from(&CharCounts::of(b));
        assert_eq!(missing("INV001", "PAGO INV001"), 0);
        assert_eq!(missing("INV001", "INV01"), 1);
        assert_eq!(missing("AAB", "A"), 2);
        assert_eq!(missing("", "ABC"), 0);
        assert_eq!(missing("ABC", ""), 3);
        assert_eq!(missing("ÁÉ", "É"), 1);
    }

    #[test]
    fn the_prefilter_keeps_pairs_at_the_edit_limit() {
        // Ten characters allow two edits (similarity 0.8).
        let number = "INV2024001";
        for reference in [
            "PAGO INV2024010 GRACIAS", // transposition
            "PAGO INV202X001",         // one substitution
            "PAGO INV2024XX1",         // two substitutions: exactly 0.8
            "PAGO INV20X24001",        // insertion
            "PAGO INV202401",          // deletion
        ] {
            let full = fuzzy_reference_signal(number, reference);
            assert!(full >= FUZZY_FLOOR, "{reference}: {full}");
            assert_eq!(counted(number, reference), full, "{reference}");
        }
    }

    #[test]
    fn the_prefilter_rejects_pairs_beyond_the_edit_limit() {
        // Three substitutions are 0.7: the full search scores 0 and so does the filter.
        let (number, reference) = ("INV2024001", "PAGO INV2XXX001");
        assert!(close(fuzzy_reference_signal(number, reference), 0.0));
        let missing = CharCounts::of(number).missing_from(&CharCounts::of(reference));
        assert!(missing > (number.chars().count() + 1) / 5);
        assert!(close(counted(number, reference), 0.0));
    }

    /// The search as it was before the window bound: every window, one `String` each.
    fn naive_fuzzy_signal(number: &str, reference: &str) -> f64 {
        let n = number.chars().count();
        if n < FUZZY_MIN_LEN || reference.contains(number) {
            return 0.0;
        }
        let chars: Vec<char> = reference.chars().collect();
        let mut best = 0.0_f64;
        for len in (n - 1)..=(n + 1) {
            for window in chars.windows(len) {
                let candidate: String = window.iter().collect();
                best = best.max(strsim::normalized_damerau_levenshtein(number, &candidate));
            }
        }
        if best >= FUZZY_FLOOR { best } else { 0.0 }
    }

    #[test]
    fn the_edit_budget_is_the_most_that_still_reaches_the_floor() {
        assert_eq!(max_edits(4, 4), 0);
        assert_eq!(max_edits(4, 5), 1);
        assert_eq!(max_edits(10, 10), 2);
        assert_eq!(max_edits(10, 11), 2);
        assert_eq!(max_edits(10, 9), 2);
        assert_eq!(max_edits(15, 16), 3);
    }

    proptest::proptest! {
        #[test]
        fn the_window_bound_never_changes_the_signal(
            number in "[ABC1]{4,9}",
            reference in "[ABC1 ]{0,30}",
        ) {
            proptest::prop_assert_eq!(
                fuzzy_reference_signal(&number, &reference).to_bits(),
                naive_fuzzy_signal(&number, &reference).to_bits()
            );
        }

        #[test]
        fn the_window_bound_never_changes_the_signal_for_non_ascii_text(
            number in "[AB\u{c9}1]{4,8}",
            reference in "[AB\u{c9}1]{0,24}",
        ) {
            proptest::prop_assert_eq!(
                fuzzy_reference_signal(&number, &reference).to_bits(),
                naive_fuzzy_signal(&number, &reference).to_bits()
            );
        }

        #[test]
        fn the_window_bound_finds_every_close_window_in_longer_digit_text(
            number in "INV[0-9]{4,7}",
            reference in "[IN V0-9]{0,60}",
        ) {
            proptest::prop_assert_eq!(
                fuzzy_reference_signal(&number, &reference).to_bits(),
                naive_fuzzy_signal(&number, &reference).to_bits()
            );
        }
    }

    proptest::proptest! {
        #[test]
        fn the_prefilter_never_changes_the_signal(
            number in "[ABC1]{4,10}",
            reference in "[ABC1 ]{0,24}",
        ) {
            let plain = fuzzy_reference_signal_with(&number, &reference, None);
            proptest::prop_assert_eq!(counted(&number, &reference), plain);
        }
    }
}
