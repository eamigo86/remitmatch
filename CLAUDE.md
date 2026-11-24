# remitmatch
 
Python library with a Rust core (PyO3) that matches bank payments to open invoices
(1:1, 1:N, N:1) and explains every match with a per-signal score.
Spec: `docs/rfc/0001-remitmatch.md`. Read it before changing the public API.
 
## Layout
 
- `crates/remitmatch/` — pure Rust core, no Python. Published to crates.io as `remitmatch`.
- `crates/remitmatch-py/` — PyO3 bindings (`cdylib`, `abi3-py310`), `publish = false`.
- `python/remitmatch/` — Python package: `__init__.py`, `.pyi` stubs, `py.typed`.
- `tests/` — pytest. `crates/remitmatch/benches/` — criterion.
## Commands
 
```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
uv run maturin develop          # build the extension into the venv
uv run ruff check . && uv run ruff format --check .
uv run mypy python tests
uv run pytest
```
 
Run all of the above before saying a task is done.
 
## Rules
 
- Money is `i64` minor units (cents). Never floats for amounts.
- The core crate must not depend on PyO3. Bindings only convert types and release the GIL.
- Public API changes must match the RFC; if they can't, update the RFC in the same PR and tell me.
- Documentation follows "Documentation Standards" below, `.pyi` stubs included.
- Tests first for matching logic; property tests (proptest / hypothesis) for invariants:
  no id in two matches, group sums within tolerance, no cross-currency or cross-scope matches.
- English for code, comments, commits (Conventional Commits) and docs.
## Documentation Standards

### Python (`.py` and `.pyi`)

- **A. Full Google style on everything public.** Every public module, class, function and
  method has a complete Google-style docstring, with exact, non-empty sections. Public means:
  an importable module; a name without a leading underscore or listed in `__all__`; a class
  member that is not private. `Args:` also documents `*args` and `**kwargs`.
- **B. Types live in the signature.** Parameters and return values carry type hints. `Args:`,
  `Returns:`/`Yields:` and `Raises:` describe behavior and never repeat the types.
- **C. Plain text.** No docstring contains backticks, including private, nested and dunder
  functions.

Enforced by ruff (`D` with the Google convention, `ANN`) and `tests/test_docstrings.py`.

### Rust

Rust follows the rustdoc conventions (Rust API Guidelines, RFC 1574). They match the Python
rules in spirit and differ in two places: docs are Markdown, and parameters are described in
prose instead of an `Args` list.

- **A. Everything public is documented.** The crate (`//!`), and every public module, struct,
  enum, trait, function, method, constant, field and variant has a doc comment. The first line
  is a one-sentence summary in the third person ("Returns the ...") ending with a period.
- **Sections are exact and non-empty, and present whenever they apply:** `# Errors` (the item
  returns `Result`), `# Panics` (it can panic), `# Safety` (it is `unsafe`), and `# Examples`
  on every public function and method, written as a doctest that compiles and passes.
  Trait implementations inherit the trait's documentation and need no example.
- **B. Types live in the signature.** Prose describes behavior, units and invariants
  ("amount in minor units"); it does not restate types.
- **C. Markdown, not plain text.** Code goes in backticks and other items are referenced with
  intra-doc links (``[`Invoice`]``). This is where the industry standard differs from the
  Python rule.
- **Exception: PyO3 items.** A `///` comment on a `#[pyfunction]`, `#[pyclass]` or
  `#[pymethods]` item becomes the Python `__doc__`, so it follows the Python rules
  (Google style, no backticks).

Enforced by `#![deny(missing_docs)]`, `clippy::missing_errors_doc`,
`clippy::missing_panics_doc`, doctests in `cargo test`, and
`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`.

## Ask me first
 
- Before publishing to PyPI or crates.io, pushing tags, or changing CI secrets.
- Never handle my passwords or tokens; tell me what to configure and wait.
 