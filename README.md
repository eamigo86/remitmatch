# remitmatch

A Python library with a Rust core (PyO3) that matches bank payments to open invoices,
including split and grouped payments (1:1, 1:N and N:1), and explains every match with a
per-signal score. Remittance matching is the accounts-receivable term for linking an
incoming payment to the invoices it settles.

**Status:** name reserved, under active development

1:1, 1:N and N:1 matching work from Python on the development branch (Rust core plus PyO3
bindings). The published 0.0.1 still only reserves the name, so the example below needs a
build from source, for example `uv run maturin develop`. Arrow input and `to_arrow()` are
planned for a later change.

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

## License

Licensed under either of the MIT license or the Apache License, Version 2.0, at your option.
