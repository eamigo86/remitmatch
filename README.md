# remitmatch

A Python library with a Rust core (PyO3) that matches bank payments to open invoices,
including split and grouped payments (1:1, 1:N and N:1), and explains every match with a
per-signal score. Remittance matching is the accounts-receivable term for linking an
incoming payment to the invoices it settles.

**Status:** name reserved, under active development

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

## License

Licensed under either of the MIT license or the Apache License, Version 2.0, at your option.
