//! Text normalization for payment references, invoice numbers and counterparty names.

use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

/// Trailing legal-form tokens ignored when comparing names.
const COMPANY_SUFFIXES: &[&str] = &[
    "sl",
    "slu",
    "sa",
    "sau",
    "sas",
    "sarl",
    "srl",
    "spa",
    "lda",
    "llc",
    "inc",
    "ltd",
    "limited",
    "gmbh",
    "ag",
    "kg",
    "corp",
    "corporation",
    "co",
    "company",
    "bv",
    "nv",
    "plc",
    "pty",
    "oy",
    "ab",
];

/// Normalized free text plus the positions where whitespace was removed.
pub(crate) struct Reference {
    /// Uppercased text without whitespace or dashes.
    pub(crate) text: String,
    /// The byte offsets of `text`, ascending, right before which whitespace was removed
    /// (one per gap, at the first byte of the character that follows it).
    /// Dashes never count as gaps; most references have few gaps or none.
    gaps: Vec<usize>,
}

impl Reference {
    /// Returns whether whitespace was removed right before the character that starts at byte
    /// `edge` of `text`.
    pub(crate) fn gap_before(&self, edge: usize) -> bool {
        self.gaps.binary_search(&edge).is_ok()
    }
}

/// Uppercases `input` and removes whitespace and dashes, tracking whitespace gaps.
///
/// Also used for invoice numbers, where the gap information is ignored.
pub(crate) fn normalize_reference(input: &str) -> Reference {
    let mut text = String::with_capacity(input.len());
    let mut gaps = Vec::new();
    let mut gap = false;
    for c in input.chars() {
        if c.is_whitespace() {
            gap = true;
        } else if c != '-' {
            if gap {
                gaps.push(text.len());
            }
            for u in c.to_uppercase() {
                text.push(u);
            }
            gap = false;
        }
    }
    Reference { text, gaps }
}

/// Normalizes a counterparty name: no accents, lowercase, no punctuation or company suffix.
///
/// The last remaining token is never stripped, so a name that is only a suffix survives.
pub(crate) fn normalize_name(input: &str) -> String {
    let cleaned: String = input
        .nfd()
        .filter(|&c| !is_combining_mark(c))
        .flat_map(char::to_lowercase)
        .filter(|&c| c != '.')
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let mut tokens: Vec<&str> = cleaned.split_whitespace().collect();
    while tokens.len() > 1 && tokens.last().is_some_and(|t| COMPANY_SUFFIXES.contains(t)) {
        tokens.pop();
    }
    tokens.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_is_uppercased_without_spaces_or_dashes() {
        let r = normalize_reference("inv - 2024 001");
        assert_eq!(r.text, "INV2024001");
    }

    #[test]
    fn reference_marks_gaps_only_for_removed_whitespace() {
        let r = normalize_reference("INV-1 10");
        assert_eq!(r.text, "INV110");
        let gaps: Vec<usize> = (0..=r.text.len()).filter(|&i| r.gap_before(i)).collect();
        assert_eq!(gaps, vec![4]);
    }

    #[test]
    fn reference_gap_is_marked_at_the_first_byte_of_a_multibyte_char() {
        let r = normalize_reference("A é");
        assert_eq!(r.text, "AÉ");
        assert!(!r.gap_before(0) && r.gap_before(1));
    }

    #[test]
    fn reference_gaps_are_kept_sorted_and_whitespace_runs_count_once() {
        let r = normalize_reference("  A 1  2\t3 ");
        assert_eq!(r.text, "A123");
        let gaps: Vec<usize> = (0..=r.text.len()).filter(|&i| r.gap_before(i)).collect();
        assert_eq!(
            gaps,
            vec![0, 1, 2, 3],
            "a leading gap, then one per run; a trailing one has no byte"
        );
        assert!(normalize_reference("ABC").gaps.is_empty());
    }

    #[test]
    fn empty_reference_is_empty() {
        let r = normalize_reference(" - ");
        assert!(r.text.is_empty());
        assert!(r.gaps.is_empty());
    }

    #[test]
    fn name_drops_accents_case_and_punctuation() {
        assert_eq!(
            normalize_name("  José   Pérez-García "),
            "jose perez garcia"
        );
    }

    #[test]
    fn name_strips_company_suffixes_with_dots() {
        assert_eq!(normalize_name("Acme S.L."), "acme");
        assert_eq!(normalize_name("Acme, Inc."), "acme");
        assert_eq!(normalize_name("ACME Co. Ltd"), "acme");
    }

    #[test]
    fn name_never_strips_the_last_remaining_token() {
        assert_eq!(normalize_name("Limited"), "limited");
        assert_eq!(normalize_name("Inc Ltd"), "inc");
    }

    #[test]
    fn name_keeps_inner_suffix_like_tokens() {
        assert_eq!(normalize_name("Co Acme"), "co acme");
    }

    #[test]
    fn empty_name_is_empty() {
        assert_eq!(normalize_name(" . "), "");
    }
}
