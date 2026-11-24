//! Text normalization for payment references, invoice numbers and counterparty names.

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "wired into the engine in a later commit")
)]

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
    /// For each byte of `text`, whether whitespace was removed right before it.
    /// Dashes never count as gaps.
    pub(crate) gap_before: Vec<bool>,
}

/// Uppercases `input` and removes whitespace and dashes, tracking whitespace gaps.
///
/// Also used for invoice numbers, where the gap information is ignored.
pub(crate) fn normalize_reference(input: &str) -> Reference {
    let mut text = String::with_capacity(input.len());
    let mut gap_before = Vec::with_capacity(input.len());
    let mut gap = false;
    for c in input.chars() {
        if c.is_whitespace() {
            gap = true;
        } else if c != '-' {
            for u in c.to_uppercase() {
                text.push(u);
                gap_before.extend(std::iter::repeat_n(gap, u.len_utf8()));
            }
            gap = false;
        }
    }
    Reference { text, gap_before }
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
        assert_eq!(r.gap_before.len(), r.text.len());
        let gaps: Vec<usize> = (0..r.text.len()).filter(|&i| r.gap_before[i]).collect();
        assert_eq!(gaps, vec![4]);
    }

    #[test]
    fn reference_gap_flags_cover_every_byte_of_multibyte_chars() {
        let r = normalize_reference("A é");
        assert_eq!(r.text, "AÉ");
        assert_eq!(r.gap_before, vec![false, true, true]);
    }

    #[test]
    fn empty_reference_is_empty() {
        let r = normalize_reference(" - ");
        assert!(r.text.is_empty());
        assert!(r.gap_before.is_empty());
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
