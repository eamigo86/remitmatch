//! PyO3 bindings for remitmatch. Exposes the native module `_core`.
//!
//! The bindings only convert types: Python values are copied into the core structs and the
//! core results are copied back. All matching logic lives in the `remitmatch` crate.
//!
//! - `Invoice`, `Payment`, `MatchConfig`, `Match` and `ReconciliationResult` are frozen
//!   classes that wrap the core structs, so they are shared between threads without a lock.
//!   Dates are `datetime.date`, amounts are Python `int` in minor units.
//! - `MatchKind` and `MatchStatus` are plain Python enums in `remitmatch._enums`; the native
//!   code imports them on first use and converts through their string values.
//! - `reconcile` copies its inputs, releases the GIL while the core runs and maps
//!   [`remitmatch::Error`] to `ValueError`. Each input is a sequence of the expected classes
//!   or an Arrow table (see the `arrow` module); anything else raises `TypeError`.
//! - A result shares its matches with the `Match` objects it hands out, so reading
//!   `matches` does not copy them.
//! - `ReconciliationResult.to_arrow` returns a `pyarrow.Table`; pyarrow is imported only when
//!   it is called, so it stays an optional dependency.
//! - Docstrings on the Python-facing items follow the Python documentation rules of the
//!   project (Google style, no backticks), because they become `__doc__`.

mod arrow;

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::NaiveDate;
use pyo3::exceptions::{PyImportError, PyModuleNotFoundError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyTuple, PyType};
use pyo3_arrow::PyTable;
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
pub(crate) fn kind_value(kind: MatchKind) -> &'static str {
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
pub(crate) fn status_value(status: MatchStatus) -> &'static str {
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

/// Constructor argument names of the Python class, in order.
const INVOICE_FIELDS: &[&str] = &[
    "id",
    "number",
    "amount_minor",
    "currency",
    "issue_date",
    "due_date",
    "counterparty",
    "scope",
];

/// Constructor argument names of the Python class, in order.
const PAYMENT_FIELDS: &[&str] = &[
    "id",
    "amount_minor",
    "currency",
    "value_date",
    "reference",
    "counterparty",
    "scope",
];

/// Constructor argument names of the Python class, in order.
const CONFIG_FIELDS: &[&str] = &[
    "amount_tolerance_minor",
    "date_window_days",
    "max_group_size",
    "auto_threshold",
    "suggest_threshold",
    "ambiguity_margin",
    "weights",
];

/// Constructor argument names of the Python class, in order.
const MATCH_FIELDS: &[&str] = &[
    "kind",
    "status",
    "payment_ids",
    "invoice_ids",
    "score",
    "breakdown",
    "residual_minor",
];

/// Constructor argument names of the Python class, in order.
const RESULT_FIELDS: &[&str] = &["matches", "unmatched_payments", "unmatched_invoices"];

/// Returns the class of `obj` and its constructor arguments, as `__reduce__` expects.
fn reduce_fields<'py>(
    obj: &Bound<'py, PyAny>,
    fields: &[&str],
) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
    let args = fields
        .iter()
        .map(|field| obj.getattr(*field))
        .collect::<PyResult<Vec<_>>>()?;
    Ok((obj.get_type().into_any(), PyTuple::new(obj.py(), args)?))
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
///
/// Raises:
///     TypeError: An argument has the wrong type.
///     OverflowError: An integer argument is outside the accepted range.
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
        repr_fields(slf.as_any(), "Invoice", INVOICE_FIELDS)
    }

    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
        reduce_fields(slf.as_any(), INVOICE_FIELDS)
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
///
/// Raises:
///     TypeError: An argument has the wrong type.
///     OverflowError: An integer argument is outside the accepted range.
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
        repr_fields(slf.as_any(), "Payment", PAYMENT_FIELDS)
    }

    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
        reduce_fields(slf.as_any(), PAYMENT_FIELDS)
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
///
/// Raises:
///     TypeError: An argument has the wrong type.
///     OverflowError: An integer argument is outside the accepted range.
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

    /// Relative weight of each signal in the final score.
    ///
    /// Each access returns a new dict, so bind it to a variable before indexing
    /// repeatedly.
    #[getter]
    fn weights(&self) -> BTreeMap<String, f64> {
        self.0.weights.clone()
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        repr_fields(slf.as_any(), "MatchConfig", CONFIG_FIELDS)
    }

    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
        reduce_fields(slf.as_any(), CONFIG_FIELDS)
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
///     TypeError: kind or status is not a member of the matching enum, or another argument
///         has the wrong type.
///     OverflowError: An integer argument is outside the accepted range.
#[pyclass(frozen, eq, from_py_object, module = "remitmatch", name = "Match")]
#[derive(Clone, PartialEq)]
struct PyMatch(Arc<Match>);

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
        Ok(Self(Arc::new(Match {
            kind: kind_from_value(&enum_value(kind, &MATCH_KIND, "MatchKind")?)?,
            status: status_from_value(&enum_value(status, &MATCH_STATUS, "MatchStatus")?)?,
            payment_ids,
            invoice_ids,
            score,
            breakdown,
            residual_minor,
        })))
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
    ///
    /// Each access returns a new tuple, so bind it to a variable before indexing
    /// repeatedly.
    #[getter]
    fn payment_ids<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, &self.0.payment_ids)
    }

    /// Invoices in the match.
    ///
    /// Each access returns a new tuple, so bind it to a variable before indexing
    /// repeatedly.
    #[getter]
    fn invoice_ids<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, &self.0.invoice_ids)
    }

    /// Final weighted score in [0, 1].
    #[getter]
    fn score(&self) -> f64 {
        self.0.score
    }

    /// Per-signal scores that explain the final score.
    ///
    /// Each access returns a new dict, so bind it to a variable before indexing
    /// repeatedly.
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
        repr_fields(slf.as_any(), "Match", MATCH_FIELDS)
    }

    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
        reduce_fields(slf.as_any(), MATCH_FIELDS)
    }
}

/// Output of a reconciliation run.
///
/// Args:
///     matches: Proposed matches, best score first.
///     unmatched_payments: Payment ids with no acceptable candidate.
///     unmatched_invoices: Invoice ids with no acceptable candidate.
///
/// Raises:
///     TypeError: An argument has the wrong type.
#[pyclass(
    frozen,
    eq,
    from_py_object,
    module = "remitmatch",
    name = "ReconciliationResult"
)]
#[derive(Clone, PartialEq)]
struct PyReconciliationResult {
    /// Shared with the `Match` objects returned by the `matches` getter.
    matches: Vec<Arc<Match>>,
    unmatched_payments: Vec<String>,
    unmatched_invoices: Vec<String>,
}

impl From<ReconciliationResult> for PyReconciliationResult {
    /// Moves each match into its own shared allocation.
    fn from(result: ReconciliationResult) -> Self {
        Self {
            matches: result.matches.into_iter().map(Arc::new).collect(),
            unmatched_payments: result.unmatched_payments,
            unmatched_invoices: result.unmatched_invoices,
        }
    }
}

#[pymethods]
impl PyReconciliationResult {
    #[new]
    fn new(
        matches: Vec<PyMatch>,
        unmatched_payments: Vec<String>,
        unmatched_invoices: Vec<String>,
    ) -> Self {
        Self {
            matches: matches.into_iter().map(|m| m.0).collect(),
            unmatched_payments,
            unmatched_invoices,
        }
    }

    /// Proposed matches, best score first.
    ///
    /// Each access returns a new list, so bind it to a variable before indexing
    /// repeatedly.
    #[getter]
    fn matches(&self) -> Vec<PyMatch> {
        self.matches.iter().cloned().map(PyMatch).collect()
    }

    /// Payment ids with no acceptable candidate.
    ///
    /// Each access returns a new list, so bind it to a variable before indexing
    /// repeatedly.
    #[getter]
    fn unmatched_payments(&self) -> Vec<String> {
        self.unmatched_payments.clone()
    }

    /// Invoice ids with no acceptable candidate.
    ///
    /// Each access returns a new list, so bind it to a variable before indexing
    /// repeatedly.
    #[getter]
    fn unmatched_invoices(&self) -> Vec<String> {
        self.unmatched_invoices.clone()
    }

    /// Return matches as a flat Arrow table (one row per payment-invoice pair).
    ///
    /// The rows follow the order of matches, then of payments, then of invoices. The columns
    /// are match_index (uint32, the position of the match), kind and status (strings, the
    /// values of the enums), payment_id and invoice_id (strings), score (float64),
    /// residual_minor (int64, repeated on each row of the match) and the per-signal scores
    /// reference_score, amount_score, name_score and date_score (float64). Unmatched ids are
    /// not included. A result without matches gives an empty table with the same columns.
    /// The pyarrow package is imported when this method is called, and an import failure other
    /// than a missing pyarrow propagates unchanged.
    ///
    /// Returns:
    ///     One row for each payment-invoice pair in every match, with the columns above, ordered by
    ///     match, then payment, then invoice.
    ///
    /// Raises:
    ///     ImportError: pyarrow is not installed; install it with the arrow extra.
    fn to_arrow<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        py.import("pyarrow").map_err(|cause| {
            // Only a missing pyarrow gets the install hint; a broken install propagates as is.
            let missing = cause.is_instance_of::<PyModuleNotFoundError>(py)
                && cause
                    .value(py)
                    .getattr("name")
                    .and_then(|name| name.extract::<String>())
                    .is_ok_and(|name| name == "pyarrow");
            if !missing {
                return cause;
            }
            let error = PyImportError::new_err(
                "to_arrow needs pyarrow; install it with: pip install \"remitmatch[arrow]\"",
            );
            error.set_cause(py, Some(cause));
            error
        })?;
        let batch = py.detach(|| arrow::matches_batch(&self.matches))?;
        let schema = batch.schema();
        PyTable::try_new(vec![batch], schema)?.into_pyarrow(py)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        repr_fields(slf.as_any(), "ReconciliationResult", RESULT_FIELDS)
    }

    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
        reduce_fields(slf.as_any(), RESULT_FIELDS)
    }
}

/// Match payments against open invoices.
///
/// Runs in Rust and releases the GIL. Each invoice and payment appears in at most one
/// match. The inputs are copied before matching starts.
///
/// Each of invoices and payments is either a sequence of objects or an Arrow table: any
/// object that exports the Arrow C stream, such as a pyarrow Table or RecordBatchReader or
/// a Polars DataFrame. The two arguments are independent. Table columns are matched by name
/// and extra columns are ignored. Invoices need id, number, amount_minor, currency,
/// issue_date, due_date and counterparty; payments need id, amount_minor, currency,
/// value_date, reference and counterparty; both accept an optional scope column, and a
/// missing one means the default scope. Text columns may be any Arrow string type or a
/// dictionary of strings, amount_minor any integer type that fits a signed 64-bit integer,
/// and dates date32 or date64. Floats, decimals and timestamps are rejected.
///
/// An object that exports an Arrow stream is always read as a table, even if it is also a
/// sequence. A column name used by the engine must be unique in the table, while unused
/// columns may repeat names. A reader that was already consumed yields no rows.
///
/// Args:
///     invoices: Open invoices to settle, as a sequence such as a list or tuple (any
///         sequence except a string) or as an Arrow table.
///     payments: Bank movements received, as a sequence such as a list or tuple (any
///         sequence except a string) or as an Arrow table.
///     config: Tolerances and thresholds; defaults to MatchConfig().
///
/// Returns:
///     Matches with per-signal breakdown, plus unmatched ids on each side.
///
/// Raises:
///     ValueError: An id repeats on one side, the config is invalid, the reference index
///         cannot be built, a table lacks a required column or has a null or an invalid
///         date in a used column, or the Arrow stream cannot be read.
///     TypeError: An input is neither a sequence of the expected objects nor an Arrow
///         stream, or a table column has an unsupported Arrow type.
///     OverflowError: An integer value in a table does not fit a signed 64-bit integer.
#[pyfunction]
#[pyo3(signature = (invoices, payments, config=None))]
fn reconcile(
    py: Python<'_>,
    invoices: &Bound<'_, PyAny>,
    payments: &Bound<'_, PyAny>,
    config: Option<PyMatchConfig>,
) -> PyResult<PyReconciliationResult> {
    // ponytail: inputs are copied into core structs (and results copied back), O(n) extra
    // memory; borrow from the frozen Python objects or from the Arrow buffers instead (a
    // columnar core API) if copies ever dominate the run. Arrow input creates no Python
    // object per row, but each value is still copied once.
    let invoices: Vec<Invoice> = match arrow::read_stream(invoices)? {
        Some(batches) => py.detach(|| arrow::invoices(&batches))?,
        None => invoices
            .extract::<Vec<PyInvoice>>()?
            .into_iter()
            .map(|i| i.0)
            .collect(),
    };
    let payments: Vec<Payment> = match arrow::read_stream(payments)? {
        Some(batches) => py.detach(|| arrow::payments(&batches))?,
        None => payments
            .extract::<Vec<PyPayment>>()?
            .into_iter()
            .map(|p| p.0)
            .collect(),
    };
    let config = config.map_or_else(MatchConfig::default, |c| c.0);
    py.detach(|| {
        remitmatch::reconcile(&invoices, &payments, &config).map(PyReconciliationResult::from)
    })
    .map_err(|error| PyValueError::new_err(error.to_string()))
}

/// Native extension module for remitmatch. Not part of the public API.
#[pymodule]
mod _core {
    use pyo3::prelude::*;

    #[pymodule_export]
    use super::{PyInvoice, PyMatch, PyMatchConfig, PyPayment, PyReconciliationResult, reconcile};

    /// Return the version of the native core.
    ///
    /// Returns:
    ///     The semantic version string of the remitmatch crate.
    #[pyfunction]
    fn version() -> &'static str {
        remitmatch::version()
    }
}
