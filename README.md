# remitmatch

A Python library with a Rust core (PyO3) that matches bank payments to open invoices,
including split and grouped payments (1:1, 1:N and N:1), and explains every match with a
per-signal score. Remittance matching is the accounts-receivable term for linking an
incoming payment to the invoices it settles.

**Status:** name reserved, under active development

1:1, 1:N and N:1 matching work from Python on the development branch (Rust core plus PyO3
bindings). The published 0.0.1 still only reserves the name, so the example below needs a
build from source, for example `uv run maturin develop`. Arrow tables work as input and as
output (see [Arrow tables](#arrow-tables)).

The design is described in
[RFC 0001](https://github.com/eamigo86/remitmatch/blob/main/docs/rfc/0001-remitmatch.md).

## Install

Python:

```bash
pip install remitmatch
```

Rust:

```bash
cargo add remitmatch
```

## Usage

Amounts are integers in minor units (cents). `reconcile` takes lists or tuples of `Invoice`
and `Payment` objects and returns a `ReconciliationResult`:

```python
from datetime import date

from remitmatch import Invoice, MatchConfig, Payment, reconcile

invoices = [
    Invoice(
        "i1", "INV-001", 60_000, "EUR", date(2024, 3, 1), date(2024, 3, 31), "Acme S.L.", "main"
    ),
    Invoice(
        "i2", "INV-002", 40_000, "EUR", date(2024, 3, 5), date(2024, 3, 31), "Acme S.L.", "main"
    ),
    Invoice(
        "i3", "INV-003", 12_500, "EUR", date(2024, 3, 7), date(2024, 4, 6), "Beta Corp", "main"
    ),
]
payments = [
    Payment("p1", 99_970, "EUR", date(2024, 3, 28), "Payment INV-001 INV-002", "ACME", "main"),
]

result = reconcile(invoices, payments, MatchConfig(amount_tolerance_minor=150))

for match in result.matches:
    print(match.kind.value, match.status.value, match.payment_ids, match.invoice_ids)
    print("residual:", match.residual_minor, "score:", round(match.score, 2))
print("unmatched invoices:", result.unmatched_invoices)
```

Output:

```text
1:N auto ('p1',) ('i1', 'i2')
residual: -30 score: 0.94
unmatched invoices: ['i3']
```

The types are immutable: attributes are read-only and `Invoice` and `Payment` are hashable.
`reconcile` raises `ValueError` for a repeated id or an invalid configuration and
`TypeError` for inputs that are not sequences of the expected objects. The example is run by
`tests/test_readme.py`.

## Arrow tables

`invoices` and `payments` also accept an Arrow table: any object that exports the Arrow C
stream, such as a pyarrow `Table` or `RecordBatchReader` or a Polars `DataFrame`. The two
arguments are independent, so one may be a table and the other a list.
`ReconciliationResult.to_arrow()` returns a flat `pyarrow.Table` with one row per
payment-invoice pair of every match. pyarrow is an optional extra
(`pip install "remitmatch[arrow]"`). Input needs no pyarrow at all.

```python
from datetime import date

import pyarrow as pa
from remitmatch import MatchConfig, reconcile

invoices = pa.table(
    {
        "id": ["i1", "i2", "i3"],
        "number": ["INV-001", "INV-002", "INV-003"],
        "amount_minor": pa.array([60_000, 40_000, 12_500], pa.int64()),
        "currency": ["EUR", "EUR", "EUR"],
        "issue_date": pa.array([date(2024, 3, 1), date(2024, 3, 5), date(2024, 3, 7)]),
        "due_date": pa.array([date(2024, 3, 31), date(2024, 3, 31), date(2024, 4, 6)]),
        "counterparty": ["Acme S.L.", "Acme S.L.", "Beta Corp"],
        "scope": ["main", "main", "main"],
    }
)
payments = pa.table(
    {
        "id": ["p1"],
        "amount_minor": pa.array([99_970], pa.int64()),
        "currency": ["EUR"],
        "value_date": pa.array([date(2024, 3, 28)]),
        "reference": ["Payment INV-001 INV-002"],
        "counterparty": ["ACME"],
        "scope": ["main"],
    }
)

result = reconcile(invoices, payments, MatchConfig(amount_tolerance_minor=150))

table = result.to_arrow()
print(table.column_names)
for row in table.select(["match_index", "kind", "payment_id", "invoice_id"]).to_pylist():
    print(row)
```

Output:

```text
['match_index', 'kind', 'status', 'payment_id', 'invoice_id', 'score', 'residual_minor', 'reference_score', 'amount_score', 'name_score', 'date_score']
{'match_index': 0, 'kind': '1:N', 'payment_id': 'p1', 'invoice_id': 'i1'}
{'match_index': 0, 'kind': '1:N', 'payment_id': 'p1', 'invoice_id': 'i2'}
```

Input contract. Columns are matched by name, so their order does not matter and extra
columns are ignored. Invoices need `id`, `number`, `amount_minor`, `currency`, `issue_date`,
`due_date` and `counterparty`. Payments need `id`, `amount_minor`, `currency`, `value_date`,
`reference` and `counterparty`. Both accept an optional `scope`; without it every row is in
the `default` scope. Text columns may be any Arrow string type (`utf8`, `large_utf8`,
`string_view`, or a dictionary of one of them). `amount_minor` may be any integer type that
fits a signed 64-bit integer; floats and decimals are rejected because money is never a float.
Dates are `date32` or `date64`; timestamps are rejected. Multi-chunk and empty tables work.

A missing required column or a null in a used column raises `ValueError`, a column of an
unsupported type raises `TypeError`, and an integer that does not fit raises `OverflowError`;
each message names the column. Values are never rounded or wrapped.

Output schema of `to_arrow()`: `match_index` (uint32, position in `matches`), `kind` and
`status` (utf8), `payment_id` and `invoice_id` (utf8), `score` (float64), `residual_minor`
(int64, repeated on each row of the match) and the signals `reference_score`, `amount_score`,
`name_score` and `date_score` (float64). Rows are in match order, then payment order, then
invoice order. Unmatched ids are not in the table, and an empty result gives an empty table
with the same schema.

"Zero-copy" here means that no Python object is created per row: batches are read through the
C stream and each value is copied once into the core structs. The core itself is not columnar.

## License

Licensed under either of the MIT license or the Apache License, Version 2.0, at your option.
