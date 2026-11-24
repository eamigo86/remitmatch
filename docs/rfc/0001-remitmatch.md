# RFC 0001 — remitmatch: matching payments to invoices (Rust + Python)

**Status:** draft · **Type:** RFC 0001 · **Reviewers:** pending · **Date:** 2026-11-22

## Problem and success metrics

Matching payments to invoices is still manual work for many small businesses and finance teams: someone opens the bank statement and searches by hand for the invoice each movement pays.

It's hard because bank descriptions are noisy free text, payer names don't match the names in the ERP, fees change the amount, and one payment can cover several invoices (or the other way round). Python solutions tend to be pandas loops that don't scale; the fast ones have no Python API.

**Users:** developers building invoicing, accounting or treasury software in Python. In InvoiceHub, the end user is whoever reviews the reconciliation inbox.

| Metric | v1 target | How it's measured |
| --- | --- | --- |
| Payments matched as `auto` | ≥ 85 % | Generator data with realistic noise |
| Precision of `auto` matches | ≥ 99 % | Against the generator's known solution |
| Time for 100k payments | < 5 s on an 8-core laptop | Reproducible benchmark in the repo |
| Speed-up vs. pandas + rapidfuzz | ≥ 20× | Same dataset and machine |

These targets are initial estimates and will be revisited after the first round of benchmarks.

## Goal and positioning

A Python library with a Rust core (PyO3) that matches payments to invoices, including split and grouped payments, and explains every match with a per-signal score. It's called `remitmatch`: *remittance matching* is the accounts-receivable term for linking an incoming payment to the invoices it settles. The name is free on PyPI and crates.io, no product uses it, and only 3 GitHub repos do.

`ledgermatch` was rejected: it's already used by a commercial Google Sheets add-on with the same features (exact, near-match, split and grouped), an enterprise reconciliation platform, and about 20 GitHub repos.

The gap it fills: nothing today combines a Python API, a Rust core, 1:N and N:1 matching, explainable matches and exact money.

| Project | Language | What it lacks compared to this library |
| --- | --- | --- |
| [hyperswitch-reconciliation](https://github.com/juspay/hyperswitch-reconciliation) | Rust | Closest in features (1:1, 1:N, N:M), but it's a service with its own API and Postgres that you deploy; you can't call it in-process |
| [recon-kit](https://github.com/WyattAu/recon-kit) | Rust | 1:1 only and no tolerances, by design; no Python |
| [OpenRec](https://github.com/GrandmasterTash/OpenRec) | Rust | A CLI with YAML and Lua rules, not a Python library |
| [reconPy](https://pypi.org/project/reconPy) | Python | Pure Python and one-to-one only |
| [florecon](https://docs.rs/crate/florecon/0.1.2/source/README.md) | Rust + Wasm | Defining a strategy requires writing Rust |
| [Reconify](https://pkg.go.dev/github.com/reconifyhq/reconify) | Go | No Python interface |
| [ReconProof](https://apify.com/zahi90/reconproof.md) | Closed | Commercial; does handle split and grouped payments |

As a portfolio piece it shows three things: Rust used where it matters in production, public API design, and fintech domain knowledge.

## Scope v1

v1 includes split and grouped payments; what's left out is whatever doesn't help the demo or the benchmark.

**In v1**

- 1:1, 1:N (one payment covers several invoices) and N:1 (one invoice paid in several payments) matching
- Configurable tolerances: amount (absolute and %), date window in days
- Reference extraction from the bank description (invoice number inside free text)
- Counterparty name similarity (normalization + Jaro-Winkler / token set)
- Per-signal score breakdown and status: `auto`, `suggested`, `ambiguous`
- Partitioning by scope and currency: different currencies or companies never match
- Exact money as integer minor units (`i64`), no floats
- Input: lists of Python objects or Arrow tables (Polars/pandas via pyarrow); output: objects + `to_arrow()`
- Parallelism with rayon, releasing the GIL

**Deferred to v2**

- N:M matching (several payments against several invoices at once)
- Currency conversion
- Bank statement parsers (CAMT.053, MT940, OFX)
- Learning from user corrections
- CLI and streaming processing

## Data model and Python API

A single entry point, `reconcile()`, with immutable types and a result you can inspect or export to Arrow. Types are defined in Rust (`#[pyclass]`) and shipped with `.pyi` stubs; this is the contract as seen from Python:

```python
from dataclasses import dataclass, field
from datetime import date
from enum import Enum


@dataclass(frozen=True)
class Invoice:
    """An open receivable that payments can settle.

    Attributes:
        id: Unique invoice identifier in the caller's system.
        number: Human-facing invoice number searched for in payment references.
        amount_minor: Outstanding amount in minor units (e.g. cents).
        currency: ISO 4217 code; invoices only match payments in the same currency.
        issue_date: Date the invoice was issued.
        due_date: Date payment is expected.
        counterparty: Customer name as stored in the ledger.
        scope: Partition key (company, account); different scopes never match.
    """

    id: str
    number: str
    amount_minor: int
    currency: str
    issue_date: date
    due_date: date
    counterparty: str
    scope: str = "default"


@dataclass(frozen=True)
class Payment:
    """A bank movement received that may settle one or more invoices.

    Attributes:
        id: Unique movement identifier from the bank feed.
        amount_minor: Received amount in minor units.
        currency: ISO 4217 code.
        value_date: Date the funds were credited.
        reference: Free-text remittance info / description from the bank.
        counterparty: Payer name as reported by the bank.
        scope: Partition key; must equal the invoice scope to match.
    """

    id: str
    amount_minor: int
    currency: str
    value_date: date
    reference: str
    counterparty: str
    scope: str = "default"


@dataclass(frozen=True)
class MatchConfig:
    """Tolerances and thresholds that drive matching.

    Attributes:
        amount_tolerance_minor: Absolute amount difference accepted (e.g. bank fees).
        date_window_days: Max days between due date and value date.
        max_group_size: Max invoices (1:N) or payments (N:1) in one group; below 2 disables group search.
        auto_threshold: Score at or above which a match is accepted automatically.
        suggest_threshold: Score at or above which a match is proposed for review.
        ambiguity_margin: If the best two candidates differ by less than this, flag ambiguous.
        weights: Relative weight of each signal in the final score.
    """

    amount_tolerance_minor: int = 0
    date_window_days: int = 30
    max_group_size: int = 5
    auto_threshold: float = 0.90
    suggest_threshold: float = 0.60
    ambiguity_margin: float = 0.05
    weights: dict[str, float] = field(
        default_factory=lambda: {"reference": 0.45, "amount": 0.30, "name": 0.15, "date": 0.10}
    )


class MatchKind(Enum):
    """Cardinality of a match."""

    ONE_TO_ONE = "1:1"
    ONE_TO_MANY = "1:N"
    MANY_TO_ONE = "N:1"


class MatchStatus(Enum):
    """How confident the engine is in a proposed match."""

    AUTO = "auto"
    SUGGESTED = "suggested"
    AMBIGUOUS = "ambiguous"


@dataclass(frozen=True)
class Match:
    """A proposed link between payments and invoices.

    Attributes:
        kind: 1:1, 1:N or N:1.
        status: auto, suggested or ambiguous.
        payment_ids: Payments in the match.
        invoice_ids: Invoices in the match.
        score: Final weighted score in [0, 1].
        breakdown: Per-signal scores that explain the final score.
        residual_minor: Amount difference left after matching (fees, rounding).
    """

    kind: MatchKind
    status: MatchStatus
    payment_ids: tuple[str, ...]
    invoice_ids: tuple[str, ...]
    score: float
    breakdown: dict[str, float]
    residual_minor: int


@dataclass(frozen=True)
class ReconciliationResult:
    """Output of a reconciliation run.

    Attributes:
        matches: Proposed matches, best score first.
        unmatched_payments: Payment ids with no acceptable candidate.
        unmatched_invoices: Invoice ids with no acceptable candidate.
    """

    matches: list[Match]
    unmatched_payments: list[str]
    unmatched_invoices: list[str]

    def to_arrow(self) -> "pyarrow.Table":
        """Return matches as a flat Arrow table (one row per payment-invoice pair)."""
        ...


def reconcile(
    invoices: "list[Invoice] | pyarrow.Table",
    payments: "list[Payment] | pyarrow.Table",
    config: MatchConfig | None = None,
) -> ReconciliationResult:
    """Match payments against open invoices.

    Runs in Rust and releases the GIL. Each invoice and payment appears in at
    most one match. Inputs may be Python objects or Arrow tables with the same
    column names as the dataclass fields.

    Args:
        invoices: Open invoices to settle.
        payments: Bank movements received.
        config: Tolerances and thresholds; defaults to MatchConfig().

    Returns:
        Matches with per-signal breakdown, plus unmatched ids on each side.

    Raises:
        ValueError: An id repeats on one side, or the config is invalid.
    """
    ...
```

Usage example: `result = reconcile(invoices, payments, MatchConfig(amount_tolerance_minor=150))`.

The Rust core exposes the same contract as `remitmatch::reconcile(&[Invoice], &[Payment], &MatchConfig) -> Result<ReconciliationResult, Error>`.

## Matching algorithm

The engine works in passes, from safest to most expensive: whatever one pass matches is removed from the next, so the group search only sees what's still unmatched.

1. **Normalize and partition.** References uppercased with spaces and dashes removed; names without accents or company suffixes (S.L., LLC, Inc.). Currency codes are compared case-insensitively. Data is grouped by `(scope, currency)` and each partition is processed in parallel.
2. **Reference pass.** An Aho-Corasick automaton built from the open invoice numbers finds, in a single scan, every invoice each description mentions. Each mentioned invoice is tried alone (amount fits → 1:1) and every subset of 2 to `max_group_size` mentioned invoices is tried together (sum fits → 1:N). Payments that cite the same invoice and add up to it form an N:1. The tolerance applies once, to the group total. A number glued to another digit is not a mention and gets no partial reference score either, so `INV-1` is not found inside `INV-10`; whitespace counts as a boundary. This pass does not apply the date window.
3. **1:1 candidates.** An index sorted by amount returns only the invoices within tolerance; those are then filtered by date window and scored. This avoids comparing everything against everything. Fuzzy reference similarity is not used for a payment whose reference already names an open invoice exactly.
4. **1:N groups.** For each unmatched payment: open invoices from the same counterparty (name signal of at least 0.85), within the window and not larger than the payment plus the tolerance. The engine looks for a subset whose sum equals the payment amount ± tolerance, with at most `max_group_size` invoices.
   1. It's a *subset-sum* variant: bounded by a maximum number of candidates (24) and solved with *meet-in-the-middle*; if there are more, the 24 oldest invoices are searched, and at most 32 subsets are kept.
   2. If more than one subset is valid, the match is flagged `ambiguous`. This holds in every group pass, the reference pass included.
5. **N:1 groups.** The same in reverse: unmatched payments from the same counterparty (name, window and amount filters mirrored) whose sum equals one invoice amount ± tolerance. A group is proposed only when the sum fits, so partial payments that do not yet add up stay unmatched.
6. **Global assignment.** All candidates are sorted by score and accepted greedily when none of their ids is already taken. If an anchor's second-best candidate is within `ambiguity_margin`, the status is `ambiguous`, never `auto`. The anchor is the payment for 1:1 and 1:N, and the invoice for N:1. Ambiguity is evaluated per pass. Rows with a non-positive amount never match.
7. **Final status.** `auto` if the score is above `auto_threshold` and not ambiguous; `suggested` if above `suggest_threshold`; everything else stays unmatched.

**Scoring.** A weighted sum of four signals in [0, 1], also returned individually in `breakdown`. The weighted sum is divided by the sum of the weights:

| Signal | How it's computed | Default weight |
| --- | --- | --- |
| Reference | 1 if the exact number appears; partial if a sliding window of the reference has a normalized Damerau-Levenshtein similarity of at least 0.8 (numbers shorter than 4 characters get no partial score) | 0.45 |
| Amount | 1 if exact; decreases linearly to 0 at the tolerance limit | 0.30 |
| Name | Jaro-Winkler on normalized names | 0.15 |
| Date | 1 within 3 days of the due date; decreases linearly to the end of the window | 0.10 |

**Group scoring.** The same four signals, computed over the payment-invoice pairs of the group: reference is the share of pairs where the payment names the invoice exactly (no partial score in groups), amount reads the difference between the two totals, name is the minimum over the pairs, and date is the mean over the pairs.

Greedy assignment is enough for v1; the Hungarian algorithm for optimal 1:1 assignment is a follow-up improvement measured in the benchmarks.

## Rust architecture, packaging and CI

Two crates in one workspace: the core knows nothing about Python, so it's tested and benchmarked with Rust tooling and also published on crates.io.

```text
remitmatch/
├── Cargo.toml              # workspace
├── pyproject.toml          # maturin as build backend
├── crates/
│   ├── remitmatch/         # pure Rust core, published on crates.io as remitmatch
│   │   └── benches/        # criterion
│   └── remitmatch-py/      # PyO3: #[pyclass], Arrow conversion, GIL release; publish = false
├── python/remitmatch/      # __init__.py, .pyi stubs, py.typed
├── tests/                  # pytest + hypothesis
├── benchmarks/             # comparison against pure Python, pandas and reconPy
└── docs/                   # mkdocs-material
```

| Piece | Choice | Reason |
| --- | --- | --- |
| Bindings | PyO3 + maturin | De facto standard; builds wheels for Linux, macOS and Windows |
| ABI | `abi3` (Python ≥ 3.10) | One wheel per platform works for every Python version |
| Parallelism | rayon, one task per partition | Scales with cores; the GIL is released for the whole computation |
| References | `aho-corasick` crate | Searches thousands of invoice numbers in one pass |
| Similarity | `strsim` crate (Jaro-Winkler) | Simple, no heavy dependencies |
| Name normalization | `unicode-normalization` crate | Accent stripping for names |
| Arrow | `arrow-rs` + PyCapsule interface | Zero-copy input and output from Polars or pandas |
| CI | GitHub Actions + `maturin-action` | Rust and Python tests, per-platform wheels |
| Publishing | PyPI Trusted Publishing | No tokens; same flow as my other packages |

Quality: `clippy` and `rustfmt` for Rust, `ruff` and `mypy` for Python, and coverage with Codecov as in django-graphex.

## Tests and benchmarks

A synthetic data generator with a known solution serves both the accuracy tests and the benchmarks.

**Generator.** Creates N invoices and their payments with controlled noise: bank fees, misspelled references, abbreviated names, partial payments and grouped payments. It stores the correct solution so accuracy can be measured.

**Tests**

- Property tests (proptest in Rust, hypothesis in Python): no id appears in two matches; every group's sum respects the tolerance; currencies and scopes never cross.
- Golden cases: small hand-written files for each match type and each edge case.
- Precision and recall against the generator's solution, per match type.

**Benchmarks**

| Dimension | Values |
| --- | --- |
| Size | 10k, 100k and 1M payments |
| Compared against | Pure Python, pandas + rapidfuzz, reconPy |
| Metrics | Total time, peak memory, precision and recall |
| Tools | criterion (Rust), pytest-benchmark (Python) |

Results go in the README as a table and a chart, with the script to reproduce them. This is the part most likely to be shared on LinkedIn.

## Integration with InvoiceHub

InvoiceHub is the library's first real user: the user uploads a bank statement and the system marks invoices as paid (see [RFC 0005](0005-reconciliation.md)).

1. The user uploads the statement (CSV in v1) and it's saved to file storage.
2. A Celery task parses it, loads the tenant's open invoices and calls `reconcile()`.
3. `auto` matches are applied directly; `suggested` and `ambiguous` ones go to a review inbox.
4. In the inbox, the UI (HTMX) shows the score breakdown and the user accepts, rejects or reassigns.
5. For `ambiguous` matches, an LLM can explain in plain language why it's unsure and which option it suggests. AI only steps in on the hard cases and never decides on its own.

This way InvoiceHub shows Rust, Celery, multi-tenancy and applied AI together in one flow.

## Milestones and open decisions

The library takes weeks 3 to 6 of the 12-week plan, in parallel with the InvoiceHub foundation.

```mermaid
gantt
    title The library ships to PyPI at the end of week 5
    dateFormat  X
    axisFormat  W%s
    section Weeks 3–6
    1:1 core (model, normalization, reference pass, scoring, generator)  :2, 3
    Grouped payments (1:N, N:1, subset-sum, assignment, properties)      :3, 4
    Bindings and PyPI (PyO3, stubs, Arrow, wheels, 0.1.0)                :4, 5
    Benchmarks and demo (README, docs, InvoiceHub, LinkedIn)             :5, 6
    Gate: 0.1.0 published on PyPI                                         :milestone, 5, 0
```

If week 4 runs long, 0.1.0 ships with 1:1 and 1:N only, and N:1 moves to 0.2.0.

**Open decisions**

- [x] Reserve the name by publishing an empty 0.0.1 on PyPI and crates.io (done 2026-11-24)
- [ ] Default scoring weights: tune them with the generator data. With the current defaults, a payment without a reference signal can score at most 0.55, below the 0.60 suggest threshold, and an amount at the tolerance limit scores 0. For groups the same ceiling applies: a group found only by counterparty has no reference signal and scores at most 0.55 under the defaults, so the counterparty passes only produce matches for a partly mentioned group or with custom weights or thresholds
- [ ] Percentage amount tolerance is listed in Scope v1, but `MatchConfig` has no field for it yet
- [ ] Decide whether a match with only a partial (misspelled) reference may be `auto` or must stay `suggested`; today it can be `auto` when the other signals are strong (for example invoice INV-2024-001 and reference INV-2024-010)
- [ ] Short or purely numeric invoice numbers (for example 7 or 100) can be found inside unrelated text such as an amount, and an exact amount plus a close date is then enough for `auto`
- [ ] The candidate pass scores every same-amount pair and the partial reference search is unoptimized (about 66 microseconds per pair); address it in the benchmarks milestone
- [ ] Candidate limit for the group search (24 is an initial estimate)
- [ ] The per-payment time budget from the risk table is not implemented; the group search is bounded by the 24-candidate and 32-subset limits instead
- [x] License: MIT OR Apache-2.0 (dual, as is common in Rust)

## Alternatives considered

Rust with PyO3 is chosen because it's the only option that delivers performance, an ergonomic Python API and skills that transfer to today's ecosystem (Polars, Pydantic, Ruff) at the same time.

| Alternative | Pros | Why it's rejected |
| --- | --- | --- |
| Pure Python + rapidfuzz | Faster to write; rapidfuzz is already C++ | The expensive part (indexes, groups, assignment) stays in Python and doesn't scale or parallelize |
| Polars without custom Rust | Vectorized, nothing to compile | 1:N and N:1 groups don't map well to dataframe operations |
| Cython or Numba | Direct Python integration | Worse ergonomics for complex data structures and less portfolio value |
| Deploy hyperswitch-reconciliation | Rust engine from a payments company, with 1:1, 1:N and N:M | A separate service with its own API and Postgres; one more thing to run for InvoiceHub, and not a library you call from Python |
| Use recon-kit from Rust | Careful design: integers, ambiguous matches go to review | 1:1 only and no tolerances, by design; groups and bindings would still have to be built |
| Reuse OpenRec | Proven Rust engine | A CLI with Lua rules, not a library; no grouped invoice payments |
| Custom Go or Rust service over HTTP | Language-independent | Network latency and extra deployment for what is a pure function |

## Risks and mitigations

The main risk is the combinatorial explosion of group search; it's bounded by design.

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Combinatorial explosion in subset-sum | Unpredictable run times | `max_group_size`, candidate limit, meet-in-the-middle and a per-payment time budget |
| False positives in `auto` | Invoices wrongly marked as paid | High threshold, ambiguity margin, and never `auto` for groups with several solutions |
| Poorly calibrated weights | Too many unnecessary `suggested` matches | Calibrate with the generator and expose weights in `MatchConfig` |
| Per-platform wheels | Broken installs on some systems | `abi3`, a CI matrix with maturin-action, and an install test for every wheel |
| 4-week scope | 0.1.0 slips | Plan B from the milestones section: N:1 moves to 0.2.0 |
| Real data differs from synthetic data | Benchmarks lose credibility | Add an anonymized or public dataset before 0.2.0 |
