//! PyO3 bindings for remitmatch. Exposes the native module `_core`.
//!
//! The bindings only convert types: Python values are copied into the core structs and the
//! core results are copied back. The classes are frozen, so they can be shared between
//! threads without a lock. All matching logic lives in the `remitmatch` crate.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyTuple, PyType};
use remitmatch::{
    Invoice, Match, MatchConfig, MatchKind, MatchStatus, Payment, ReconciliationResult,
};

/// The Python class `remitmatch._enums.MatchKind`, imported on first use.
static MATCH_KIND: PyOnceLock<Py<PyType>> = PyOnceLock::new();
/// The Python class `remitmatch._enums.MatchStatus`, imported on first use.
static MATCH_STATUS: PyOnceLock<Py<PyType>> = PyOnceLock::new();

/// Returns the member of the Python enum `name` whose value is `value`.
fn enum_member<'py>(
    py: Python<'py>,
    cell: &PyOnceLock<Py<PyType>>,
    name: &str,
    value: &str,
) -> PyResult<Bound<'py, PyAny>> {
    cell.import(py, "remitmatch._enums", name)?.call1((value,))
}

/// Returns the value of a member of the Python enum `name`, or a `TypeError` for anything else.
fn enum_value(
    obj: &Bound<'_, PyAny>,
    cell: &PyOnceLock<Py<PyType>>,
    name: &str,
) -> PyResult<String> {
    let class = cell.import(obj.py(), "remitmatch._enums", name)?;
    if !obj.is_instance(class)? {
        return Err(PyTypeError::new_err(format!("expected a {name} member")));
    }
    obj.getattr("value")?.extract()
}

/// Returns the Python enum value of a match kind.
fn kind_value(kind: MatchKind) -> &'static str {
    match kind {
        MatchKind::OneToOne => "1:1",
        MatchKind::OneToMany => "1:N",
        MatchKind::ManyToOne => "N:1",
    }
}

/// Returns the match kind for a Python enum value.
fn kind_from_value(value: &str) -> PyResult<MatchKind> {
    match value {
        "1:1" => Ok(MatchKind::OneToOne),
        "1:N" => Ok(MatchKind::OneToMany),
        "N:1" => Ok(MatchKind::ManyToOne),
        _ => Err(PyValueError::new_err(format!(
            "unknown match kind: {value}"
        ))),
    }
}

/// Returns the Python enum value of a match status.
fn status_value(status: MatchStatus) -> &'static str {
    match status {
        MatchStatus::Auto => "auto",
        MatchStatus::Suggested => "suggested",
        MatchStatus::Ambiguous => "ambiguous",
    }
}

/// Returns the match status for a Python enum value.
fn status_from_value(value: &str) -> PyResult<MatchStatus> {
    match value {
        "auto" => Ok(MatchStatus::Auto),
        "suggested" => Ok(MatchStatus::Suggested),
        "ambiguous" => Ok(MatchStatus::Ambiguous),
        _ => Err(PyValueError::new_err(format!(
            "unknown match status: {value}"
        ))),
    }
}

/// Formats `obj` like a dataclass repr from the Python repr of the named attributes.
fn repr_fields(obj: &Bound<'_, PyAny>, class: &str, fields: &[&str]) -> PyResult<String> {
    let parts = fields
        .iter()
        .map(|field| Ok(format!("{field}={}", obj.getattr(*field)?.repr()?)))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(format!("{class}({})", parts.join(", ")))
}

/// An open receivable that payments can settle.
///
/// Args:
///     id: Unique invoice identifier in the caller's system.
///     number: Human-facing invoice number searched for in payment references.
///     amount_minor: Outstanding amount in minor units (e.g. cents).
///     currency: ISO 4217 code; invoices only match payments in the same currency.
///     issue_date: Date the invoice was issued.
///     due_date: Date payment is expected.
///     counterparty: Customer name as stored in the ledger.
///     scope: Partition key (company, account); different scopes never match.
#[pyclass(
    frozen,
    eq,
    hash,
    from_py_object,
    module = "remitmatch",
    name = "Invoice"
)]
#[derive(Clone, PartialEq, Eq, Hash)]
struct PyInvoice(Invoice);

#[pymethods]
impl PyInvoice {
    // The constructor mirrors the eight RFC fields, so the argument count is the contract.
    #[allow(clippy::too_many_arguments)]
    #[new]
    #[pyo3(signature = (id, number, amount_minor, currency, issue_date, due_date, counterparty, scope="default"))]
    fn new(
        id: String,
        number: String,
        amount_minor: i64,
        currency: String,
        issue_date: NaiveDate,
        due_date: NaiveDate,
        counterparty: String,
        scope: &str,
    ) -> Self {
        Self(Invoice {
            id,
            number,
            amount_minor,
            currency,
            issue_date,
            due_date,
            counterparty,
            scope: scope.to_owned(),
        })
    }

    /// Unique invoice identifier in the caller's system.
    #[getter]
    fn id(&self) -> &str {
        &self.0.id
    }

    /// Human-facing invoice number searched for in payment references.
    #[getter]
    fn number(&self) -> &str {
        &self.0.number
    }

    /// Outstanding amount in minor units (e.g. cents).
    #[getter]
    fn amount_minor(&self) -> i64 {
        self.0.amount_minor
    }

    /// ISO 4217 code; invoices only match payments in the same currency.
    #[getter]
    fn currency(&self) -> &str {
        &self.0.currency
    }

    /// Date the invoice was issued.
    #[getter]
    fn issue_date(&self) -> NaiveDate {
        self.0.issue_date
    }

    /// Date payment is expected.
    #[getter]
    fn due_date(&self) -> NaiveDate {
        self.0.due_date
    }

    /// Customer name as stored in the ledger.
    #[getter]
    fn counterparty(&self) -> &str {
        &self.0.counterparty
    }

    /// Partition key (company, account); different scopes never match.
    #[getter]
    fn scope(&self) -> &str {
        &self.0.scope
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        repr_fields(
            slf.as_any(),
            "Invoice",
            &[
                "id",
                "number",
                "amount_minor",
                "currency",
                "issue_date",
                "due_date",
                "counterparty",
                "scope",
            ],
        )
    }
}

/// A bank movement received that may settle one or more invoices.
///
/// Args:
///     id: Unique movement identifier from the bank feed.
///     amount_minor: Received amount in minor units.
///     currency: ISO 4217 code.
///     value_date: Date the funds were credited.
///     reference: Free-text remittance info / description from the bank.
///     counterparty: Payer name as reported by the bank.
///     scope: Partition key; must equal the invoice scope to match.
#[pyclass(
    frozen,
    eq,
    hash,
    from_py_object,
    module = "remitmatch",
    name = "Payment"
)]
#[derive(Clone, PartialEq, Eq, Hash)]
struct PyPayment(Payment);

#[pymethods]
impl PyPayment {
    #[new]
    #[pyo3(signature = (id, amount_minor, currency, value_date, reference, counterparty, scope="default"))]
    fn new(
        id: String,
        amount_minor: i64,
        currency: String,
        value_date: NaiveDate,
        reference: String,
        counterparty: String,
        scope: &str,
    ) -> Self {
        Self(Payment {
            id,
            amount_minor,
            currency,
            value_date,
            reference,
            counterparty,
            scope: scope.to_owned(),
        })
    }

    /// Unique movement identifier from the bank feed.
    #[getter]
    fn id(&self) -> &str {
        &self.0.id
    }

    /// Received amount in minor units.
    #[getter]
    fn amount_minor(&self) -> i64 {
        self.0.amount_minor
    }

    /// ISO 4217 code.
    #[getter]
    fn currency(&self) -> &str {
        &self.0.currency
    }

    /// Date the funds were credited.
    #[getter]
    fn value_date(&self) -> NaiveDate {
        self.0.value_date
    }

    /// Free-text remittance info / description from the bank.
    #[getter]
    fn reference(&self) -> &str {
        &self.0.reference
    }

    /// Payer name as reported by the bank.
    #[getter]
    fn counterparty(&self) -> &str {
        &self.0.counterparty
    }

    /// Partition key; must equal the invoice scope to match.
    #[getter]
    fn scope(&self) -> &str {
        &self.0.scope
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        repr_fields(
            slf.as_any(),
            "Payment",
            &[
                "id",
                "amount_minor",
                "currency",
                "value_date",
                "reference",
                "counterparty",
                "scope",
            ],
        )
    }
}

/// Tolerances and thresholds that drive matching.
///
/// Args:
///     amount_tolerance_minor: Absolute amount difference accepted (e.g. bank fees).
///     date_window_days: Max days between due date and value date.
///     max_group_size: Max invoices (1:N) or payments (N:1) in one group; below 2 disables
///         group search.
///     auto_threshold: Score at or above which a match is accepted automatically.
///     suggest_threshold: Score at or above which a match is proposed for review.
///     ambiguity_margin: If the best two candidates differ by less than this, flag ambiguous.
///     weights: Relative weight of each signal in the final score; None selects the default
///         weights.
#[pyclass(
    frozen,
    eq,
    from_py_object,
    module = "remitmatch",
    name = "MatchConfig"
)]
#[derive(Clone, PartialEq)]
struct PyMatchConfig(MatchConfig);

#[pymethods]
impl PyMatchConfig {
    #[new]
    #[pyo3(signature = (
        amount_tolerance_minor=0,
        date_window_days=30,
        max_group_size=5,
        auto_threshold=0.90,
        suggest_threshold=0.60,
        ambiguity_margin=0.05,
        weights=None,
    ))]
    fn new(
        amount_tolerance_minor: i64,
        date_window_days: u32,
        max_group_size: usize,
        auto_threshold: f64,
        suggest_threshold: f64,
        ambiguity_margin: f64,
        weights: Option<BTreeMap<String, f64>>,
    ) -> Self {
        Self(MatchConfig {
            amount_tolerance_minor,
            date_window_days,
            max_group_size,
            auto_threshold,
            suggest_threshold,
            ambiguity_margin,
            weights: weights.unwrap_or_else(|| MatchConfig::default().weights),
        })
    }

    /// Absolute amount difference accepted (e.g. bank fees).
    #[getter]
    fn amount_tolerance_minor(&self) -> i64 {
        self.0.amount_tolerance_minor
    }

    /// Max days between due date and value date.
    #[getter]
    fn date_window_days(&self) -> u32 {
        self.0.date_window_days
    }

    /// Max invoices (1:N) or payments (N:1) in one group; below 2 disables group search.
    #[getter]
    fn max_group_size(&self) -> usize {
        self.0.max_group_size
    }

    /// Score at or above which a match is accepted automatically.
    #[getter]
    fn auto_threshold(&self) -> f64 {
        self.0.auto_threshold
    }

    /// Score at or above which a match is proposed for review.
    #[getter]
    fn suggest_threshold(&self) -> f64 {
        self.0.suggest_threshold
    }

    /// If the best two candidates differ by less than this, flag ambiguous.
    #[getter]
    fn ambiguity_margin(&self) -> f64 {
        self.0.ambiguity_margin
    }

    /// Relative weight of each signal in the final score; a new dict on each access.
    #[getter]
    fn weights(&self) -> BTreeMap<String, f64> {
        self.0.weights.clone()
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        repr_fields(
            slf.as_any(),
            "MatchConfig",
            &[
                "amount_tolerance_minor",
                "date_window_days",
                "max_group_size",
                "auto_threshold",
                "suggest_threshold",
                "ambiguity_margin",
                "weights",
            ],
        )
    }
}

/// A proposed link between payments and invoices.
///
/// Args:
///     kind: 1:1, 1:N or N:1.
///     status: auto, suggested or ambiguous.
///     payment_ids: Payments in the match.
///     invoice_ids: Invoices in the match.
///     score: Final weighted score in [0, 1].
///     breakdown: Per-signal scores that explain the final score.
///     residual_minor: Amount difference left after matching (fees, rounding).
///
/// Raises:
///     TypeError: kind or status is not a member of the matching enum.
#[pyclass(frozen, eq, from_py_object, module = "remitmatch", name = "Match")]
#[derive(Clone, PartialEq)]
struct PyMatch(Match);

#[pymethods]
impl PyMatch {
    #[new]
    fn new(
        kind: &Bound<'_, PyAny>,
        status: &Bound<'_, PyAny>,
        payment_ids: Vec<String>,
        invoice_ids: Vec<String>,
        score: f64,
        breakdown: BTreeMap<String, f64>,
        residual_minor: i64,
    ) -> PyResult<Self> {
        Ok(Self(Match {
            kind: kind_from_value(&enum_value(kind, &MATCH_KIND, "MatchKind")?)?,
            status: status_from_value(&enum_value(status, &MATCH_STATUS, "MatchStatus")?)?,
            payment_ids,
            invoice_ids,
            score,
            breakdown,
            residual_minor,
        }))
    }

    /// Cardinality of the match: a MatchKind member.
    #[getter]
    fn kind<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        enum_member(py, &MATCH_KIND, "MatchKind", kind_value(self.0.kind))
    }

    /// Confidence in the match: a MatchStatus member.
    #[getter]
    fn status<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        enum_member(
            py,
            &MATCH_STATUS,
            "MatchStatus",
            status_value(self.0.status),
        )
    }

    /// Payments in the match.
    #[getter]
    fn payment_ids<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, &self.0.payment_ids)
    }

    /// Invoices in the match.
    #[getter]
    fn invoice_ids<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, &self.0.invoice_ids)
    }

    /// Final weighted score in [0, 1].
    #[getter]
    fn score(&self) -> f64 {
        self.0.score
    }

    /// Per-signal scores that explain the final score; a new dict on each access.
    #[getter]
    fn breakdown(&self) -> BTreeMap<String, f64> {
        self.0.breakdown.clone()
    }

    /// Amount difference left after matching (fees, rounding).
    #[getter]
    fn residual_minor(&self) -> i64 {
        self.0.residual_minor
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        repr_fields(
            slf.as_any(),
            "Match",
            &[
                "kind",
                "status",
                "payment_ids",
                "invoice_ids",
                "score",
                "breakdown",
                "residual_minor",
            ],
        )
    }
}

/// Output of a reconciliation run.
///
/// Args:
///     matches: Proposed matches, best score first.
///     unmatched_payments: Payment ids with no acceptable candidate.
///     unmatched_invoices: Invoice ids with no acceptable candidate.
#[pyclass(
    frozen,
    eq,
    from_py_object,
    module = "remitmatch",
    name = "ReconciliationResult"
)]
#[derive(Clone, PartialEq)]
struct PyReconciliationResult(ReconciliationResult);

#[pymethods]
impl PyReconciliationResult {
    #[new]
    fn new(
        matches: Vec<PyMatch>,
        unmatched_payments: Vec<String>,
        unmatched_invoices: Vec<String>,
    ) -> Self {
        Self(ReconciliationResult {
            matches: matches.into_iter().map(|m| m.0).collect(),
            unmatched_payments,
            unmatched_invoices,
        })
    }

    /// Proposed matches, best score first; a new list on each access.
    #[getter]
    fn matches(&self) -> Vec<PyMatch> {
        self.0.matches.iter().cloned().map(PyMatch).collect()
    }

    /// Payment ids with no acceptable candidate; a new list on each access.
    #[getter]
    fn unmatched_payments(&self) -> Vec<String> {
        self.0.unmatched_payments.clone()
    }

    /// Invoice ids with no acceptable candidate; a new list on each access.
    #[getter]
    fn unmatched_invoices(&self) -> Vec<String> {
        self.0.unmatched_invoices.clone()
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        repr_fields(
            slf.as_any(),
            "ReconciliationResult",
            &["matches", "unmatched_payments", "unmatched_invoices"],
        )
    }
}

/// Native extension module for remitmatch. Not part of the public API.
#[pymodule]
mod _core {
    use pyo3::prelude::*;

    #[pymodule_export]
    use super::{PyInvoice, PyMatch, PyMatchConfig, PyPayment, PyReconciliationResult};

    /// Return the version of the native core.
    ///
    /// Returns:
    ///     The semantic version string of the remitmatch crate.
    #[pyfunction]
    fn version() -> &'static str {
        remitmatch::version()
    }
}
