//! Matches bank payments to open invoices and explains every match.
//!
//! Version 0.0.1 reserves the crate name and defines the public data model from
//! RFC 0001. It contains no matching logic yet; the first matching engine is
//! planned for 0.1.0.
//!
//! All monetary amounts are integers in minor units (for example cents), never
//! floating-point numbers.

mod normalize;

use std::collections::BTreeMap;

use chrono::NaiveDate;

/// Returns the version of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(remitmatch::version(), env!("CARGO_PKG_VERSION"));
/// ```
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// An open receivable that payments can settle.
#[derive(Debug, Clone, PartialEq)]
pub struct Invoice {
    /// Unique invoice identifier in the caller's system.
    pub id: String,
    /// Human-facing invoice number searched for in payment references.
    pub number: String,
    /// Outstanding amount in minor units.
    pub amount_minor: i64,
    /// ISO 4217 currency code; invoices only match payments in the same currency.
    pub currency: String,
    /// Date the invoice was issued.
    pub issue_date: NaiveDate,
    /// Date payment is expected.
    pub due_date: NaiveDate,
    /// Customer name as stored in the ledger.
    pub counterparty: String,
    /// Partition key (company, account); different scopes never match.
    pub scope: String,
}

/// A bank movement received that may settle one or more invoices.
#[derive(Debug, Clone, PartialEq)]
pub struct Payment {
    /// Unique movement identifier from the bank feed.
    pub id: String,
    /// Received amount in minor units.
    pub amount_minor: i64,
    /// ISO 4217 currency code.
    pub currency: String,
    /// Date the funds were credited.
    pub value_date: NaiveDate,
    /// Free-text remittance information from the bank.
    pub reference: String,
    /// Payer name as reported by the bank.
    pub counterparty: String,
    /// Partition key; must equal the invoice scope to match.
    pub scope: String,
}

/// Tolerances and thresholds that drive matching.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchConfig {
    /// Absolute amount difference accepted, in minor units (for example bank fees).
    pub amount_tolerance_minor: i64,
    /// Maximum number of days between due date and value date.
    pub date_window_days: u32,
    /// Maximum invoices (1:N) or payments (N:1) in one group.
    pub max_group_size: usize,
    /// Score at or above which a match is accepted automatically.
    pub auto_threshold: f64,
    /// Score at or above which a match is proposed for review.
    pub suggest_threshold: f64,
    /// Candidates whose scores differ by less than this are flagged ambiguous.
    pub ambiguity_margin: f64,
    /// Relative weight of each signal in the final score.
    pub weights: BTreeMap<String, f64>,
}

impl Default for MatchConfig {
    /// Returns the default configuration defined in RFC 0001.
    fn default() -> Self {
        let weights = [
            ("reference", 0.45),
            ("amount", 0.30),
            ("name", 0.15),
            ("date", 0.10),
        ]
        .into_iter()
        .map(|(signal, weight)| (signal.to_owned(), weight))
        .collect();
        Self {
            amount_tolerance_minor: 0,
            date_window_days: 30,
            max_group_size: 5,
            auto_threshold: 0.90,
            suggest_threshold: 0.60,
            ambiguity_margin: 0.05,
            weights,
        }
    }
}

/// Cardinality of a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchKind {
    /// One payment settles one invoice.
    OneToOne,
    /// One payment settles several invoices.
    OneToMany,
    /// Several payments settle one invoice.
    ManyToOne,
}

/// How confident the engine is in a proposed match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchStatus {
    /// Accepted automatically.
    Auto,
    /// Proposed for review.
    Suggested,
    /// More than one plausible candidate exists.
    Ambiguous,
}

/// A proposed link between payments and invoices.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    /// Cardinality of the match.
    pub kind: MatchKind,
    /// Confidence in the match.
    pub status: MatchStatus,
    /// Identifiers of the payments in the match.
    pub payment_ids: Vec<String>,
    /// Identifiers of the invoices in the match.
    pub invoice_ids: Vec<String>,
    /// Final weighted score in the range 0 to 1.
    pub score: f64,
    /// Per-signal scores that explain the final score.
    pub breakdown: BTreeMap<String, f64>,
    /// Amount difference left after matching, in minor units (fees, rounding).
    pub residual_minor: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_cargo_package_version() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn default_weights_sum_to_one() {
        let sum: f64 = MatchConfig::default().weights.values().sum();
        assert!((sum - 1.0).abs() < 1e-9);
    }
}
