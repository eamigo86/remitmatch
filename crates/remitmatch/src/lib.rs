//! Matches bank payments to open invoices and explains every match.
//!
//! The entry point is [`reconcile`], which pairs [`Payment`]s with [`Invoice`]s (1:1, 1:N
//! and N:1) and returns a [`ReconciliationResult`]. Every [`Match`] carries a per-signal
//! `breakdown` that explains its score.
//!
//! The reference passes look for invoice numbers inside the payment reference: a payment
//! that names several invoices whose amounts add up is a 1:N match, and several payments
//! that name one invoice and add up to it are an N:1 match. The candidate pass scores the
//! invoices whose amount is within tolerance and whose due date is inside the date window.
//! Groups are also searched by counterparty when the reference does not name them: the
//! open invoices (or payments) of a similar payer, inside the date window, whose amounts add
//! up to the payment (or invoice) within tolerance.
//! Payments and invoices only match inside the same scope and currency.
//!
//! All monetary amounts are integers in minor units (for example cents), never
//! floating-point numbers.

mod engine;
mod normalize;
mod score;
mod subset;

use std::collections::BTreeMap;
use std::fmt;

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
    /// Maximum invoices (1:N) or payments (N:1) in one group; below 2 disables groups.
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

/// The outcome of a reconciliation run.
#[derive(Debug, Clone, PartialEq)]
pub struct ReconciliationResult {
    /// Proposed matches, best score first; ties keep payment input order.
    pub matches: Vec<Match>,
    /// Identifiers of the payments with no acceptable candidate, in input order.
    pub unmatched_payments: Vec<String>,
    /// Identifiers of the invoices with no acceptable candidate, in input order.
    pub unmatched_invoices: Vec<String>,
}

/// An error returned by [`reconcile`] before any matching happens.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Two invoices share the same identifier.
    DuplicateInvoiceId(String),
    /// Two payments share the same identifier.
    DuplicatePaymentId(String),
    /// The configuration is invalid; the message names the offending field.
    InvalidConfig(String),
    /// The reference index could not be built, for example because the pattern set is too large.
    ReferenceIndex(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateInvoiceId(id) => write!(f, "duplicate invoice id: {id}"),
            Self::DuplicatePaymentId(id) => write!(f, "duplicate payment id: {id}"),
            Self::InvalidConfig(message) => write!(f, "invalid configuration: {message}"),
            Self::ReferenceIndex(message) => write!(f, "cannot build reference index: {message}"),
        }
    }
}

impl std::error::Error for Error {}

/// Matches payments to open invoices and explains every match.
///
/// Each invoice and each payment appears in at most one [`Match`]. A match is one to one,
/// one payment for several invoices, or several payments for one invoice, as named in
/// [`MatchKind`]; a group never exceeds [`MatchConfig::max_group_size`], and a value below
/// 2 disables groups. A group is only proposed when the payments add up to the invoices
/// within [`MatchConfig::amount_tolerance_minor`], applied once to the group total. Rows
/// whose amount is not positive never match. Payments and invoices only match inside the
/// same scope and currency (compared case-insensitively). Weights missing from
/// [`MatchConfig::weights`] count as zero.
///
/// # Errors
///
/// Returns [`Error::DuplicateInvoiceId`] or [`Error::DuplicatePaymentId`] when an
/// identifier repeats, [`Error::InvalidConfig`] when the configuration is out of range
/// (thresholds outside 0 to 1, a suggest threshold above the auto threshold, a negative
/// tolerance or margin, an unknown or negative weight, or weights that sum to zero), and
/// [`Error::ReferenceIndex`] when the reference index cannot be built.
///
/// # Examples
///
/// ```
/// use chrono::NaiveDate;
/// use remitmatch::{reconcile, Invoice, MatchConfig, MatchStatus, Payment};
///
/// let date = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
/// let invoice = Invoice {
///     id: "i1".to_owned(),
///     number: "INV-001".to_owned(),
///     amount_minor: 10_000,
///     currency: "EUR".to_owned(),
///     issue_date: date,
///     due_date: date,
///     counterparty: "Acme S.L.".to_owned(),
///     scope: "main".to_owned(),
/// };
/// let payment = Payment {
///     id: "p1".to_owned(),
///     amount_minor: 10_000,
///     currency: "EUR".to_owned(),
///     value_date: date,
///     reference: "Payment inv 001".to_owned(),
///     counterparty: "ACME".to_owned(),
///     scope: "main".to_owned(),
/// };
///
/// let result = reconcile(&[invoice], &[payment], &MatchConfig::default())?;
/// assert_eq!(result.matches.len(), 1);
/// assert_eq!(result.matches[0].status, MatchStatus::Auto);
/// assert!(result.unmatched_payments.is_empty());
/// # Ok::<(), remitmatch::Error>(())
/// ```
pub fn reconcile(
    invoices: &[Invoice],
    payments: &[Payment],
    config: &MatchConfig,
) -> Result<ReconciliationResult, Error> {
    engine::reconcile(invoices, payments, config)
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
