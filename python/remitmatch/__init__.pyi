"""Match bank payments to open invoices and explain every match."""

from dataclasses import dataclass, field
from datetime import date
from enum import Enum

import pyarrow

__version__: str

def version() -> str:
    """Return the version of the native core.

    Returns:
        The semantic version string of the package.
    """

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
        max_group_size: Max invoices (1:N) or payments (N:1) in one group.
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

    def to_arrow(self) -> pyarrow.Table:
        """Return matches as a flat Arrow table (one row per payment-invoice pair).

        Returns:
            A table with one row for each payment-invoice pair in every match.
        """

def reconcile(
    invoices: list[Invoice] | pyarrow.Table,
    payments: list[Payment] | pyarrow.Table,
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
    """
