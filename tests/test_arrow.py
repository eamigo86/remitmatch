"""Tests for Arrow table input and output through the Arrow PyCapsule interface."""

import sys
from collections.abc import Sequence
from datetime import date, datetime

import polars as pl
import pyarrow as pa
import pytest
from hypothesis import given, settings
from remitmatch import Invoice, MatchConfig, Payment, ReconciliationResult, reconcile
from test_properties import Inputs, inputs

DAY = date(2024, 1, 10)


def invoice_table(invoices: Sequence[Invoice], **overrides: pa.Array) -> pa.Table:
    """Build an Arrow table with the invoice columns.

    Args:
        invoices: Invoices to turn into rows.
        **overrides: Replacement arrays keyed by column name.

    Returns:
        A table with the standard invoice schema, except for the overridden columns.
    """
    columns: dict[str, pa.Array] = {
        "id": pa.array([i.id for i in invoices], pa.string()),
        "number": pa.array([i.number for i in invoices], pa.string()),
        "amount_minor": pa.array([i.amount_minor for i in invoices], pa.int64()),
        "currency": pa.array([i.currency for i in invoices], pa.string()),
        "issue_date": pa.array([i.issue_date for i in invoices], pa.date32()),
        "due_date": pa.array([i.due_date for i in invoices], pa.date32()),
        "counterparty": pa.array([i.counterparty for i in invoices], pa.string()),
        "scope": pa.array([i.scope for i in invoices], pa.string()),
    }
    columns.update(overrides)
    return pa.table(columns)


def payment_table(payments: Sequence[Payment], **overrides: pa.Array) -> pa.Table:
    """Build an Arrow table with the payment columns.

    Args:
        payments: Payments to turn into rows.
        **overrides: Replacement arrays keyed by column name.

    Returns:
        A table with the standard payment schema, except for the overridden columns.
    """
    columns: dict[str, pa.Array] = {
        "id": pa.array([p.id for p in payments], pa.string()),
        "amount_minor": pa.array([p.amount_minor for p in payments], pa.int64()),
        "currency": pa.array([p.currency for p in payments], pa.string()),
        "value_date": pa.array([p.value_date for p in payments], pa.date32()),
        "reference": pa.array([p.reference for p in payments], pa.string()),
        "counterparty": pa.array([p.counterparty for p in payments], pa.string()),
        "scope": pa.array([p.scope for p in payments], pa.string()),
    }
    columns.update(overrides)
    return pa.table(columns)


INVOICES = [
    Invoice("i1", "INV-1", 6_000, "EUR", DAY, DAY, "Acme S.L.", "main"),
    Invoice("i2", "INV-2", 4_000, "EUR", DAY, DAY, "Acme S.L.", "main"),
    Invoice("i3", "INV-3", 777, "EUR", DAY, DAY, "Beta Corp", "main"),
]
PAYMENTS = [
    Payment("p1", 10_000, "EUR", DAY, "INV-2 INV-1", "ACME", "main"),
    Payment("p2", 1, "EUR", DAY, "unrelated", "Gamma", "main"),
]
EXPECTED = reconcile(INVOICES, PAYMENTS)


def test_expected_result_has_a_group_match_and_unmatched_rows() -> None:
    """The shared fixture produces a 1:N match plus unmatched ids on both sides."""
    assert len(EXPECTED.matches) == 1
    assert EXPECTED.unmatched_payments == ["p2"]
    assert EXPECTED.unmatched_invoices == ["i3"]


def test_tables_give_the_same_result_as_objects() -> None:
    """Invoice and payment tables equal the same rows passed as objects."""
    assert reconcile(invoice_table(INVOICES), payment_table(PAYMENTS)) == EXPECTED


def test_table_and_list_can_be_mixed() -> None:
    """The two arguments are independent: one may be a table and the other a list."""
    assert reconcile(invoice_table(INVOICES), PAYMENTS) == EXPECTED
    assert reconcile(INVOICES, payment_table(PAYMENTS)) == EXPECTED


def test_config_is_honored_with_tables() -> None:
    """The configuration applies to table input like to object input."""
    config = MatchConfig(max_group_size=1)
    expected = reconcile(INVOICES, PAYMENTS, config)
    assert reconcile(invoice_table(INVOICES), payment_table(PAYMENTS), config) == expected


def test_record_batch_reader_input() -> None:
    """A RecordBatchReader is accepted because it exports the Arrow C stream."""
    invoices = invoice_table(INVOICES).to_reader()
    payments = payment_table(PAYMENTS).to_reader()
    assert reconcile(invoices, payments) == EXPECTED


def test_polars_dataframe_input() -> None:
    """A Polars DataFrame is accepted through the PyCapsule interface."""
    invoices = pl.from_arrow(invoice_table(INVOICES))
    payments = pl.from_arrow(payment_table(PAYMENTS))
    assert isinstance(invoices, pl.DataFrame)
    assert isinstance(payments, pl.DataFrame)
    assert reconcile(invoices, payments) == EXPECTED


def test_missing_scope_column_means_default() -> None:
    """Without a scope column every row is in the default scope."""
    invoices = [
        Invoice(i.id, i.number, i.amount_minor, "EUR", DAY, DAY, "Acme S.L.") for i in INVOICES
    ]
    payments = [Payment(p.id, p.amount_minor, "EUR", DAY, p.reference, "ACME") for p in PAYMENTS]
    expected = reconcile(invoices, payments)
    result = reconcile(
        invoice_table(INVOICES).drop_columns("scope"),
        payment_table(PAYMENTS).drop_columns("scope"),
    )
    assert result == expected


def test_scopes_from_tables_never_match_across_scopes() -> None:
    """A scope column keeps rows of different scopes apart."""
    other = [Payment("p1", 10_000, "EUR", DAY, "INV-2 INV-1", "ACME", "other")]
    result = reconcile(invoice_table(INVOICES), payment_table(other))
    assert result.matches == []


def test_extra_columns_and_column_order_are_ignored() -> None:
    """Columns are matched by name, so extra columns and any order work."""
    table = invoice_table(INVOICES)
    extra = table.append_column("note", pa.array(["x"] * table.num_rows))
    shuffled = extra.select(list(reversed(extra.column_names)))
    assert reconcile(shuffled, payment_table(PAYMENTS)) == EXPECTED


def test_multi_chunk_table() -> None:
    """A table made of several record batches equals the single-batch table."""
    chunked = pa.Table.from_batches(invoice_table(INVOICES).to_batches(max_chunksize=1))
    assert chunked.column("id").num_chunks == len(INVOICES)
    assert reconcile(chunked, payment_table(PAYMENTS)) == EXPECTED


def test_empty_tables() -> None:
    """Empty tables give an empty result."""
    result = reconcile(invoice_table([]), payment_table([]))
    assert result.matches == []
    assert result.unmatched_invoices == []
    assert result.unmatched_payments == []


@pytest.mark.parametrize(
    "text_type",
    [pa.large_string(), pa.string_view(), pa.dictionary(pa.int8(), pa.string())],
)
def test_other_string_types(text_type: pa.DataType) -> None:
    """Text columns may be large_utf8, string_view or a dictionary of strings."""
    invoices = invoice_table(INVOICES)
    payments = payment_table(PAYMENTS)
    for name in ("id", "currency", "counterparty"):
        invoices = invoices.set_column(
            invoices.schema.get_field_index(name), name, invoices.column(name).cast(text_type)
        )
        payments = payments.set_column(
            payments.schema.get_field_index(name), name, payments.column(name).cast(text_type)
        )
    assert reconcile(invoices, payments) == EXPECTED


def test_dictionary_with_a_null_key_is_a_null() -> None:
    """A null dictionary key in a used column is a null value."""
    ids = pa.DictionaryArray.from_arrays(
        pa.array([0, None, 1], pa.int8()), pa.array(["i1", "i2"], pa.string())
    )
    with pytest.raises(ValueError, match="id"):
        reconcile(invoice_table(INVOICES, id=ids), payment_table(PAYMENTS))


@pytest.mark.parametrize(
    "amount_type", [pa.int8(), pa.int16(), pa.int32(), pa.uint16(), pa.uint32()]
)
def test_narrow_integer_amounts(amount_type: pa.DataType) -> None:
    """Any integer type that fits i64 is accepted for amount_minor."""
    small = [Invoice("i1", "INV-1", 100, "EUR", DAY, DAY, "Acme", "main")]
    pays = [Payment("p1", 100, "EUR", DAY, "INV-1", "Acme", "main")]
    invoices = invoice_table(small, amount_minor=pa.array([100], amount_type))
    payments = payment_table(pays, amount_minor=pa.array([100], amount_type))
    assert reconcile(invoices, payments) == reconcile(small, pays)


def test_uint64_amount_that_fits_is_accepted() -> None:
    """A uint64 amount below the i64 maximum is accepted."""
    small = [Invoice("i1", "INV-1", 100, "EUR", DAY, DAY, "Acme", "main")]
    pays = [Payment("p1", 100, "EUR", DAY, "INV-1", "Acme", "main")]
    invoices = invoice_table(small, amount_minor=pa.array([100], pa.uint64()))
    assert reconcile(invoices, pays) == reconcile(small, pays)


def test_uint64_amount_that_does_not_fit_raises() -> None:
    """A uint64 amount above the i64 maximum raises instead of wrapping."""
    invoices = invoice_table(INVOICES[:1], amount_minor=pa.array([2**63], pa.uint64()))
    with pytest.raises(OverflowError, match="amount_minor"):
        reconcile(invoices, payment_table(PAYMENTS))


def test_date64_columns() -> None:
    """Dates may be date64, which counts milliseconds since the epoch."""
    invoices = invoice_table(
        INVOICES,
        issue_date=pa.array([DAY] * len(INVOICES), pa.date64()),
        due_date=pa.array([DAY] * len(INVOICES), pa.date64()),
    )
    payments = payment_table(PAYMENTS, value_date=pa.array([DAY] * len(PAYMENTS), pa.date64()))
    assert reconcile(invoices, payments) == EXPECTED


def test_date64_that_is_not_a_whole_day_raises() -> None:
    """A date64 value that is not a multiple of one day raises instead of rounding."""
    half_day = pa.array([43_200_000] * len(INVOICES), pa.date64())
    with pytest.raises(ValueError, match="due_date"):
        reconcile(invoice_table(INVOICES, due_date=half_day), payment_table(PAYMENTS))


def test_date_out_of_range_raises() -> None:
    """A date32 value outside the supported calendar range raises."""
    far = pa.array([2**31 - 1] * len(INVOICES), pa.date32())
    with pytest.raises(ValueError, match="issue_date"):
        reconcile(invoice_table(INVOICES, issue_date=far), payment_table(PAYMENTS))


@pytest.mark.parametrize(
    ("column", "wrong", "arrow_name"),
    [
        ("amount_minor", pa.array([1.0, 2.0, 3.0], pa.float64()), "Float64"),
        ("amount_minor", pa.array([1, 2, 3], pa.decimal128(10, 0)), "Decimal128"),
        ("amount_minor", pa.array(["1", "2", "3"]), "Utf8"),
        ("issue_date", pa.array([datetime(2024, 1, 10)] * 3, pa.timestamp("us")), "Timestamp"),
        ("due_date", pa.array(["2024-01-10"] * 3), "Utf8"),
        ("id", pa.array([1, 2, 3], pa.int64()), "Int64"),
        ("scope", pa.array([1, 2, 3], pa.int64()), "Int64"),
        ("number", pa.array([b"a", b"b", b"c"], pa.binary()), "Binary"),
    ],
)
def test_wrong_column_type_raises_type_error(column: str, wrong: pa.Array, arrow_name: str) -> None:
    """A column of the wrong Arrow type raises TypeError naming the column and the type."""
    table = invoice_table(INVOICES, **{column: wrong})
    with pytest.raises(TypeError, match=column) as error:
        reconcile(table, payment_table(PAYMENTS))
    assert arrow_name in str(error.value)


def test_wrong_type_is_detected_on_an_empty_table() -> None:
    """The type check uses the schema, so it also applies to a table without rows."""
    table = invoice_table([], amount_minor=pa.array([], pa.float64()))
    with pytest.raises(TypeError, match="amount_minor"):
        reconcile(table, payment_table(PAYMENTS))


@pytest.mark.parametrize("column", ["id", "amount_minor", "currency", "due_date", "counterparty"])
def test_missing_required_invoice_column_raises_value_error(column: str) -> None:
    """A missing required column raises ValueError naming it."""
    table = invoice_table(INVOICES).drop_columns(column)
    with pytest.raises(ValueError, match=column):
        reconcile(table, payment_table(PAYMENTS))


@pytest.mark.parametrize("column", ["id", "amount_minor", "currency", "value_date", "reference"])
def test_missing_required_payment_column_raises_value_error(column: str) -> None:
    """A missing required payment column raises ValueError naming it."""
    table = payment_table(PAYMENTS).drop_columns(column)
    with pytest.raises(ValueError, match=column):
        reconcile(invoice_table(INVOICES), table)


@pytest.mark.parametrize(
    ("column", "with_null"),
    [
        ("amount_minor", pa.array([1, None, 3], pa.int64())),
        ("due_date", pa.array([DAY, None, DAY], pa.date32())),
        ("number", pa.array(["a", None, "c"], pa.string())),
        ("scope", pa.array(["a", None, "c"], pa.string())),
    ],
)
def test_null_in_a_used_column_raises_value_error(column: str, with_null: pa.Array) -> None:
    """A null in a used column raises ValueError naming the column."""
    table = invoice_table(INVOICES, **{column: with_null})
    with pytest.raises(ValueError, match=column):
        reconcile(table, payment_table(PAYMENTS))


def test_null_in_an_ignored_column_is_fine() -> None:
    """Nulls in columns that are not used do not matter."""
    table = invoice_table(INVOICES)
    extra = table.append_column("note", pa.array([None, None, None], pa.string()))
    assert reconcile(extra, payment_table(PAYMENTS)) == EXPECTED


@pytest.mark.parametrize("bad", [42, None, "abc", object(), {"id": ["i1"]}])
def test_input_that_is_neither_a_sequence_nor_an_arrow_stream_raises(bad: object) -> None:
    """Anything else than a sequence of the right class or an Arrow stream is a TypeError."""
    with pytest.raises(TypeError):
        reconcile(bad, PAYMENTS)  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        reconcile(INVOICES, bad)  # type: ignore[arg-type]


def test_sequence_of_the_wrong_class_raises_type_error() -> None:
    """Passing payments where invoices are expected is a TypeError."""
    with pytest.raises(TypeError):
        reconcile(PAYMENTS, PAYMENTS)  # type: ignore[arg-type]


def test_producer_that_raises_propagates_its_exception() -> None:
    """An exception raised by __arrow_c_stream__ is propagated unchanged."""

    class Broken:
        """Object whose Arrow export fails."""

        def __arrow_c_stream__(self, requested_schema: object = None) -> object:
            """Raise instead of exporting.

            Args:
                requested_schema: Ignored.

            Raises:
                RuntimeError: Always.
            """
            raise RuntimeError("boom")

    with pytest.raises(RuntimeError, match="boom"):
        reconcile(Broken(), PAYMENTS)


def test_stream_export_that_is_not_a_capsule_raises_type_error() -> None:
    """An export that does not return a capsule is a TypeError."""

    class NotACapsule:
        """Object whose Arrow export returns the wrong thing."""

        def __arrow_c_stream__(self, requested_schema: object = None) -> object:
            """Return a plain integer.

            Args:
                requested_schema: Ignored.

            Returns:
                An integer.
            """
            return 1

    with pytest.raises(TypeError):
        reconcile(NotACapsule(), PAYMENTS)


def test_a_consumed_reader_is_empty_the_second_time() -> None:
    """A RecordBatchReader is consumed by reconcile; reusing it reads no rows and never crashes."""
    reader = invoice_table(INVOICES).to_reader()
    assert reconcile(reader, PAYMENTS) == EXPECTED
    again = reconcile(reader, PAYMENTS)
    assert again.matches == []
    assert again.unmatched_payments == ["p1", "p2"]


def test_repeated_ids_in_a_table_raise_value_error() -> None:
    """Core validation applies to table input too."""
    twice = invoice_table(INVOICES + INVOICES[:1])
    with pytest.raises(ValueError, match="i1"):
        reconcile(twice, payment_table(PAYMENTS))


@settings(max_examples=100, deadline=None, database=None)
@given(inputs())
def test_tables_and_objects_give_equal_results(data: Inputs) -> None:
    """For generated rows, reconcile on tables equals reconcile on objects.

    Args:
        data: Drawn invoices, payments and configuration.
    """
    invoices, payments, config = data
    from_tables = reconcile(invoice_table(invoices), payment_table(payments), config)
    assert from_tables == reconcile(invoices, payments, config)


OUTPUT_SCHEMA = pa.schema(
    [
        ("match_index", pa.uint32()),
        ("kind", pa.string()),
        ("status", pa.string()),
        ("payment_id", pa.string()),
        ("invoice_id", pa.string()),
        ("score", pa.float64()),
        ("residual_minor", pa.int64()),
        ("reference_score", pa.float64()),
        ("amount_score", pa.float64()),
        ("name_score", pa.float64()),
        ("date_score", pa.float64()),
    ]
)


def expected_rows(result: ReconciliationResult) -> list[dict[str, object]]:
    """Compute the documented rows of to_arrow from the matches of a result.

    Args:
        result: The result to flatten.

    Returns:
        One row per payment-invoice pair, in match, payment and invoice order.
    """
    return [
        {
            "match_index": index,
            "kind": match.kind.value,
            "status": match.status.value,
            "payment_id": payment_id,
            "invoice_id": invoice_id,
            "score": match.score,
            "residual_minor": match.residual_minor,
            "reference_score": match.breakdown["reference"],
            "amount_score": match.breakdown["amount"],
            "name_score": match.breakdown["name"],
            "date_score": match.breakdown["date"],
        }
        for index, match in enumerate(result.matches)
        for payment_id in match.payment_ids
        for invoice_id in match.invoice_ids
    ]


def test_to_arrow_schema_and_group_rows() -> None:
    """A 1:N match gives one row per invoice with the documented columns and types."""
    table = EXPECTED.to_arrow()
    assert isinstance(table, pa.Table)
    assert table.schema.equals(OUTPUT_SCHEMA)
    assert table.to_pylist() == expected_rows(EXPECTED)
    assert [row["invoice_id"] for row in table.to_pylist()] == ["i1", "i2"]
    assert {row["kind"] for row in table.to_pylist()} == {"1:N"}


def test_to_arrow_one_to_one() -> None:
    """A 1:1 match gives exactly one row."""
    result = reconcile(INVOICES[:1], [Payment("p1", 6_000, "EUR", DAY, "INV-1", "ACME", "main")])
    rows = result.to_arrow().to_pylist()
    assert rows == expected_rows(result)
    assert len(rows) == 1
    assert rows[0]["kind"] == "1:1"
    assert rows[0]["match_index"] == 0


def test_to_arrow_many_to_one() -> None:
    """An N:1 match gives one row per payment, with the residual repeated."""
    result = reconcile(
        [Invoice("i1", "INV-1", 10_000, "EUR", DAY, DAY, "Acme S.L.", "main")],
        [
            Payment("p1", 4_000, "EUR", DAY, "INV-1", "ACME", "main"),
            Payment("p2", 6_000, "EUR", DAY, "INV-1", "ACME", "main"),
        ],
    )
    rows = result.to_arrow().to_pylist()
    assert rows == expected_rows(result)
    assert [(row["payment_id"], row["invoice_id"]) for row in rows] == [("p1", "i1"), ("p2", "i1")]
    assert {row["kind"] for row in rows} == {"N:1"}


def test_to_arrow_orders_rows_by_match_then_payment_then_invoice() -> None:
    """Several matches keep the order of matches, with match_index counting them."""
    invoices = [
        Invoice("i1", "INV-1", 10_000, "EUR", DAY, DAY, "Acme S.L.", "main"),
        Invoice("i2", "INV-2", 5_000, "EUR", DAY, DAY, "Acme S.L.", "main"),
        Invoice("i3", "INV-3", 3_000, "EUR", DAY, DAY, "Acme S.L.", "main"),
    ]
    payments = [
        Payment("p1", 4_000, "EUR", DAY, "INV-1", "ACME", "main"),
        Payment("p2", 6_000, "EUR", DAY, "INV-1", "ACME", "main"),
        Payment("p3", 5_000, "EUR", DAY, "INV-2", "ACME", "main"),
        Payment("p4", 3_000, "EUR", DAY, "INV-3", "ACME", "main"),
    ]
    result = reconcile(invoices, payments)
    table = result.to_arrow()
    assert len(result.matches) == 3
    assert table.to_pylist() == expected_rows(result)
    assert table.column("match_index").to_pylist() == sorted(
        table.column("match_index").to_pylist()
    )
    assert set(table.column("match_index").to_pylist()) == {0, 1, 2}


def test_to_arrow_excludes_unmatched_ids() -> None:
    """Unmatched ids are not in the table."""
    ids = set(EXPECTED.to_arrow().column("payment_id").to_pylist())
    ids |= set(EXPECTED.to_arrow().column("invoice_id").to_pylist())
    assert ids.isdisjoint({"p2", "i3"})


def test_to_arrow_of_an_empty_result_keeps_the_schema() -> None:
    """A result without matches gives an empty table with the same schema."""
    table = ReconciliationResult([], [], []).to_arrow()
    assert table.num_rows == 0
    assert table.schema.equals(OUTPUT_SCHEMA)


def test_to_arrow_output_is_readable_by_polars() -> None:
    """The output table is a plain pyarrow Table that other libraries can read."""
    assert pl.from_arrow(EXPECTED.to_arrow()).shape == (2, 11)


def test_to_arrow_without_pyarrow_raises_import_error_naming_the_extra(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """When pyarrow cannot be imported, to_arrow raises ImportError that names the extra."""
    monkeypatch.setitem(sys.modules, "pyarrow", None)
    with pytest.raises(ImportError, match=r"remitmatch\[arrow\]"):
        EXPECTED.to_arrow()


def test_input_does_not_need_pyarrow_to_be_importable(monkeypatch: pytest.MonkeyPatch) -> None:
    """Object input and the match data work without pyarrow."""
    monkeypatch.setitem(sys.modules, "pyarrow", None)
    result = reconcile(INVOICES, PAYMENTS)
    assert len(result.matches) == 1
