//! Arrow table input for `reconcile` and the Arrow table of a result, through the Arrow
//! PyCapsule interface.
//!
//! Batches are pulled from the producer with the GIL held, because a producer may call back
//! into Python while it exports. The conversion into core structs needs no Python object and
//! runs with the GIL released. Every failure is a Python exception that names the column;
//! nothing here panics on caller data.

use arrow_array::cast::AsArray;
use arrow_array::types::{
    ArrowPrimitiveType, Date32Type, Date64Type, Int8Type, Int16Type, Int32Type, Int64Type,
    UInt8Type, UInt16Type, UInt32Type, UInt64Type,
};
use std::sync::Arc;

use arrow_array::{
    ArrayRef, Float64Array, Int64Array, LargeStringArray, RecordBatch, RecordBatchReader,
    StringArray, StringViewArray, UInt32Array,
};
use arrow_schema::DataType;
use chrono::{Datelike, NaiveDate};
use pyo3::exceptions::{PyOverflowError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyCapsule;
use pyo3_arrow::PyRecordBatchReader;
use remitmatch::{Invoice, Match, Payment};

/// Milliseconds in one day, the unit of an Arrow `date64` value.
const MILLIS_PER_DAY: i64 = 86_400_000;
/// Scope of rows that come from a table without a `scope` column.
const DEFAULT_SCOPE: &str = "default";

/// Reads the Arrow C stream of `obj`, or returns `None` when `obj` does not export one.
///
/// An empty stream yields one empty batch, so the column checks also run on the schema.
pub(crate) fn read_stream(obj: &Bound<'_, PyAny>) -> PyResult<Option<Vec<RecordBatch>>> {
    if !obj.hasattr("__arrow_c_stream__")? {
        return Ok(None);
    }
    let capsule = obj
        .call_method0("__arrow_c_stream__")?
        .cast_into::<PyCapsule>()?;
    let reader = PyRecordBatchReader::from_arrow_pycapsule(&capsule)?.into_reader()?;
    let schema = reader.schema();
    let mut batches = reader
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| PyValueError::new_err(format!("cannot read the Arrow stream: {error}")))?;
    if batches.is_empty() {
        batches.push(RecordBatch::new_empty(schema));
    }
    Ok(Some(batches))
}

/// Converts the batches of an invoice table into core invoices.
pub(crate) fn invoices(batches: &[RecordBatch]) -> PyResult<Vec<Invoice>> {
    let mut rows = Vec::with_capacity(batches.iter().map(RecordBatch::num_rows).sum());
    for batch in batches {
        let ids = text(batch, "id")?;
        let numbers = text(batch, "number")?;
        let amounts = ints(batch, "amount_minor")?;
        let currencies = text(batch, "currency")?;
        let issue_dates = dates(batch, "issue_date")?;
        let due_dates = dates(batch, "due_date")?;
        let counterparties = text(batch, "counterparty")?;
        let scopes = scopes(batch)?;
        rows.extend((0..batch.num_rows()).map(|row| Invoice {
            id: ids.value(row).to_owned(),
            number: numbers.value(row).to_owned(),
            amount_minor: amounts[row],
            currency: currencies.value(row).to_owned(),
            issue_date: issue_dates[row],
            due_date: due_dates[row],
            counterparty: counterparties.value(row).to_owned(),
            scope: scopes.value(row).to_owned(),
        }));
    }
    Ok(rows)
}

/// Converts the batches of a payment table into core payments.
pub(crate) fn payments(batches: &[RecordBatch]) -> PyResult<Vec<Payment>> {
    let mut rows = Vec::with_capacity(batches.iter().map(RecordBatch::num_rows).sum());
    for batch in batches {
        let ids = text(batch, "id")?;
        let amounts = ints(batch, "amount_minor")?;
        let currencies = text(batch, "currency")?;
        let value_dates = dates(batch, "value_date")?;
        let references = text(batch, "reference")?;
        let counterparties = text(batch, "counterparty")?;
        let scopes = scopes(batch)?;
        rows.extend((0..batch.num_rows()).map(|row| Payment {
            id: ids.value(row).to_owned(),
            amount_minor: amounts[row],
            currency: currencies.value(row).to_owned(),
            value_date: value_dates[row],
            reference: references.value(row).to_owned(),
            counterparty: counterparties.value(row).to_owned(),
            scope: scopes.value(row).to_owned(),
        }));
    }
    Ok(rows)
}

/// Returns the required column `name`, or a `ValueError` when the batch has no such column.
fn column<'a>(batch: &'a RecordBatch, name: &str) -> PyResult<&'a ArrayRef> {
    let schema = batch.schema_ref();
    if schema
        .fields()
        .iter()
        .filter(|field| field.name() == name)
        .count()
        > 1
    {
        return Err(PyValueError::new_err(format!(
            "column '{name}' appears more than once"
        )));
    }
    batch
        .column_by_name(name)
        .ok_or_else(|| PyValueError::new_err(format!("missing required column '{name}'")))
}

/// Returns the `TypeError` for a column whose Arrow type is not accepted.
fn type_error(name: &str, data_type: &DataType) -> PyErr {
    PyTypeError::new_err(format!(
        "column '{name}' has unsupported Arrow type {data_type}"
    ))
}

/// Returns the `ValueError` for a null in a used column.
fn null_error(name: &str) -> PyErr {
    PyValueError::new_err(format!("column '{name}' contains a null"))
}

/// A text column without nulls, read in place.
enum Text {
    Utf8(StringArray),
    LargeUtf8(LargeStringArray),
    Utf8View(StringViewArray),
    /// Every row has this value (the default scope of a table without a scope column).
    Constant(&'static str),
}

impl Text {
    /// Returns the value of row `row`, which must be below the number of rows of the batch.
    fn value(&self, row: usize) -> &str {
        match self {
            Self::Utf8(array) => array.value(row),
            Self::LargeUtf8(array) => array.value(row),
            Self::Utf8View(array) => array.value(row),
            Self::Constant(value) => value,
        }
    }
}

/// Reads a text column: utf8, large_utf8, string_view or a dictionary of one of them.
fn text(batch: &RecordBatch, name: &str) -> PyResult<Text> {
    let original = column(batch, name)?;
    let array = match original.data_type() {
        DataType::Dictionary(_, values) => arrow_cast::cast(original, values).map_err(|error| {
            PyValueError::new_err(format!("column '{name}' cannot be read: {error}"))
        })?,
        _ => original.clone(),
    };
    let column = match array.data_type() {
        DataType::Utf8 => Text::Utf8(array.as_string::<i32>().clone()),
        DataType::LargeUtf8 => Text::LargeUtf8(array.as_string::<i64>().clone()),
        DataType::Utf8View => Text::Utf8View(array.as_string_view().clone()),
        _ => return Err(type_error(name, original.data_type())),
    };
    if array.null_count() > 0 {
        return Err(null_error(name));
    }
    Ok(column)
}

/// Reads the optional `scope` column; a missing column means the default scope.
fn scopes(batch: &RecordBatch) -> PyResult<Text> {
    if batch.schema_ref().column_with_name("scope").is_some() {
        text(batch, "scope")
    } else {
        Ok(Text::Constant(DEFAULT_SCOPE))
    }
}

/// Reads an integer column of primitive type `T` as `i64`, failing on any value that does
/// not fit.
fn primitive_ints<T>(array: &ArrayRef, name: &str) -> PyResult<Vec<i64>>
where
    T: ArrowPrimitiveType,
    i64: TryFrom<T::Native>,
{
    array
        .as_primitive::<T>()
        .iter()
        .map(|value| {
            let value = value.ok_or_else(|| null_error(name))?;
            i64::try_from(value).map_err(|_| {
                PyOverflowError::new_err(format!(
                    "column '{name}' has the value {value:?}, which does not fit in a signed 64-bit integer"
                ))
            })
        })
        .collect()
}

/// Reads the amount column: any integer type, never a float or a decimal.
fn ints(batch: &RecordBatch, name: &str) -> PyResult<Vec<i64>> {
    let array = column(batch, name)?;
    match array.data_type() {
        DataType::Int8 => primitive_ints::<Int8Type>(array, name),
        DataType::Int16 => primitive_ints::<Int16Type>(array, name),
        DataType::Int32 => primitive_ints::<Int32Type>(array, name),
        DataType::Int64 => primitive_ints::<Int64Type>(array, name),
        DataType::UInt8 => primitive_ints::<UInt8Type>(array, name),
        DataType::UInt16 => primitive_ints::<UInt16Type>(array, name),
        DataType::UInt32 => primitive_ints::<UInt32Type>(array, name),
        DataType::UInt64 => primitive_ints::<UInt64Type>(array, name),
        other => Err(type_error(name, other)),
    }
}

/// Returns the date that is `days` days after 1970-01-01, or a `ValueError` when it is outside
/// the years 1 to 9999 that Python's `datetime.date` can hold.
fn from_epoch_days(name: &str, days: i64) -> PyResult<NaiveDate> {
    i32::try_from(days)
        .ok()
        .and_then(NaiveDate::from_epoch_days)
        .filter(|date| (1..=9999).contains(&date.year()))
        .ok_or_else(|| PyValueError::new_err(format!("column '{name}' has a date out of range")))
}

/// Reads a date column: date32 (days) or date64 (whole days in milliseconds).
fn dates(batch: &RecordBatch, name: &str) -> PyResult<Vec<NaiveDate>> {
    let array = column(batch, name)?;
    match array.data_type() {
        DataType::Date32 => array
            .as_primitive::<Date32Type>()
            .iter()
            .map(|days| from_epoch_days(name, i64::from(days.ok_or_else(|| null_error(name))?)))
            .collect(),
        DataType::Date64 => array
            .as_primitive::<Date64Type>()
            .iter()
            .map(|millis| {
                let millis = millis.ok_or_else(|| null_error(name))?;
                if millis.rem_euclid(MILLIS_PER_DAY) != 0 {
                    return Err(PyValueError::new_err(format!(
                        "column '{name}' has a date64 value that is not a whole number of days"
                    )));
                }
                from_epoch_days(name, millis.div_euclid(MILLIS_PER_DAY))
            })
            .collect(),
        other => Err(type_error(name, other)),
    }
}

/// Builds the output table: one row per payment-invoice pair of every match, in match order,
/// then payment order, then invoice order.
///
/// A signal missing from a hand-built match breakdown is a null in its column.
pub(crate) fn matches_batch(matches: &[Arc<Match>]) -> PyResult<RecordBatch> {
    let pairs: Vec<(usize, &Match, &str, &str)> = matches
        .iter()
        .map(Arc::as_ref)
        .enumerate()
        .flat_map(|(index, m)| {
            m.payment_ids.iter().flat_map(move |payment| {
                m.invoice_ids
                    .iter()
                    .map(move |invoice| (index, m, payment.as_str(), invoice.as_str()))
            })
        })
        .collect();
    let indexes = pairs
        .iter()
        .map(|(index, ..)| u32::try_from(*index))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| PyOverflowError::new_err("too many matches for a uint32 match_index"))?;
    let signal = |name: &str| -> ArrayRef {
        Arc::new(Float64Array::from_iter(
            pairs
                .iter()
                .map(|(_, m, ..)| m.breakdown.get(name).copied()),
        ))
    };
    let columns: Vec<(&str, ArrayRef)> = vec![
        ("match_index", Arc::new(UInt32Array::from(indexes))),
        (
            "kind",
            Arc::new(StringArray::from_iter_values(
                pairs.iter().map(|(_, m, ..)| crate::kind_value(m.kind)),
            )),
        ),
        (
            "status",
            Arc::new(StringArray::from_iter_values(
                pairs.iter().map(|(_, m, ..)| crate::status_value(m.status)),
            )),
        ),
        (
            "payment_id",
            Arc::new(StringArray::from_iter_values(
                pairs.iter().map(|(_, _, payment, _)| *payment),
            )),
        ),
        (
            "invoice_id",
            Arc::new(StringArray::from_iter_values(
                pairs.iter().map(|(_, _, _, invoice)| *invoice),
            )),
        ),
        (
            "score",
            Arc::new(Float64Array::from_iter_values(
                pairs.iter().map(|(_, m, ..)| m.score),
            )),
        ),
        (
            "residual_minor",
            Arc::new(Int64Array::from_iter_values(
                pairs.iter().map(|(_, m, ..)| m.residual_minor),
            )),
        ),
        ("reference_score", signal("reference")),
        ("amount_score", signal("amount")),
        ("name_score", signal("name")),
        ("date_score", signal("date")),
    ];
    // Every field is nullable, so the schema is the same for an empty and a full result.
    RecordBatch::try_from_iter_with_nullable(
        columns.into_iter().map(|(name, array)| (name, array, true)),
    )
    .map_err(|error| PyValueError::new_err(format!("cannot build the Arrow table: {error}")))
}
