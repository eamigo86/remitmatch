"""Native extension module for remitmatch. Not part of the public API."""

from collections.abc import Sequence
from datetime import date
from typing import Protocol, final

import pyarrow

from remitmatch._enums import MatchKind, MatchStatus

__all__ = [
    "Invoice",
    "Match",
    "MatchConfig",
    "Payment",
    "ReconciliationResult",
    "reconcile",
    "version",
]

class _ArrowStream(Protocol):
    def __arrow_c_stream__(self, requested_schema: object | None = None) -> object: ...

def version() -> str:
    """Return the version of the native core.

    Returns:
        The semantic version string of the remitmatch crate.
    """

def reconcile(
    invoices: Sequence[Invoice] | _ArrowStream,
    payments: Sequence[Payment] | _ArrowStream,
    config: MatchConfig | None = None,
) -> ReconciliationResult:
    """Match payments against open invoices.

    Runs in Rust and releases the GIL. Each invoice and payment appears in at most one
    match. The inputs are copied before matching starts.

    Each of invoices and payments is either a sequence of objects or an Arrow table: any
    object that exports the Arrow C stream, such as a pyarrow Table or RecordBatchReader or
    a Polars DataFrame. The two arguments are independent. Table columns are matched by name
    and extra columns are ignored. Invoices need id, number, amount_minor, currency,
    issue_date, due_date and counterparty; payments need id, amount_minor, currency,
    value_date, reference and counterparty; both accept an optional scope column, and a
    missing one means the default scope. Text columns may be any Arrow string type or a
    dictionary of strings, amount_minor any integer type that fits a signed 64-bit integer,
    and dates date32 or date64. Floats, decimals and timestamps are rejected.

    Args:
        invoices: Open invoices to settle, as a sequence such as a list or tuple (any
            sequence except a string) or as an Arrow table.
        payments: Bank movements received, as a sequence such as a list or tuple (any
            sequence except a string) or as an Arrow table.
        config: Tolerances and thresholds; defaults to MatchConfig().

    Returns:
        Matches with per-signal breakdown, plus unmatched ids on each side.

    Raises:
        ValueError: An id repeats on one side, the config is invalid, the reference index
            cannot be built, a table lacks a required column or has a null or an invalid
            date in a used column, or the Arrow stream cannot be read.
        TypeError: An input is neither a sequence of the expected objects nor an Arrow
            stream, or a table column has an unsupported Arrow type.
        OverflowError: An integer value in a table does not fit a signed 64-bit integer.
    """

@final
class Invoice:
    """An open receivable that payments can settle.

    Args:
        id: Unique invoice identifier in the caller's system.
        number: Human-facing invoice number searched for in payment references.
        amount_minor: Outstanding amount in minor units (e.g. cents).
        currency: ISO 4217 code; invoices only match payments in the same currency.
        issue_date: Date the invoice was issued.
        due_date: Date payment is expected.
        counterparty: Customer name as stored in the ledger.
        scope: Partition key (company, account); different scopes never match.

    Raises:
        TypeError: An argument has the wrong type.
        OverflowError: An integer argument is outside the accepted range.
    """

    def __new__(
        cls,
        id: str,
        number: str,
        amount_minor: int,
        currency: str,
        issue_date: date,
        due_date: date,
        counterparty: str,
        scope: str = "default",
    ) -> Invoice: ...
    @property
    def id(self) -> str:
        """Unique invoice identifier in the caller's system."""

    @property
    def number(self) -> str:
        """Human-facing invoice number searched for in payment references."""

    @property
    def amount_minor(self) -> int:
        """Outstanding amount in minor units (e.g. cents)."""

    @property
    def currency(self) -> str:
        """ISO 4217 code; invoices only match payments in the same currency."""

    @property
    def issue_date(self) -> date:
        """Date the invoice was issued."""

    @property
    def due_date(self) -> date:
        """Date payment is expected."""

    @property
    def counterparty(self) -> str:
        """Customer name as stored in the ledger."""

    @property
    def scope(self) -> str:
        """Partition key (company, account); different scopes never match."""

    def __hash__(self) -> int: ...

@final
class Payment:
    """A bank movement received that may settle one or more invoices.

    Args:
        id: Unique movement identifier from the bank feed.
        amount_minor: Received amount in minor units.
        currency: ISO 4217 code.
        value_date: Date the funds were credited.
        reference: Free-text remittance info / description from the bank.
        counterparty: Payer name as reported by the bank.
        scope: Partition key; must equal the invoice scope to match.

    Raises:
        TypeError: An argument has the wrong type.
        OverflowError: An integer argument is outside the accepted range.
    """

    def __new__(
        cls,
        id: str,
        amount_minor: int,
        currency: str,
        value_date: date,
        reference: str,
        counterparty: str,
        scope: str = "default",
    ) -> Payment: ...
    @property
    def id(self) -> str:
        """Unique movement identifier from the bank feed."""

    @property
    def amount_minor(self) -> int:
        """Received amount in minor units."""

    @property
    def currency(self) -> str:
        """ISO 4217 code."""

    @property
    def value_date(self) -> date:
        """Date the funds were credited."""

    @property
    def reference(self) -> str:
        """Free-text remittance info / description from the bank."""

    @property
    def counterparty(self) -> str:
        """Payer name as reported by the bank."""

    @property
    def scope(self) -> str:
        """Partition key; must equal the invoice scope to match."""

    def __hash__(self) -> int: ...

@final
class MatchConfig:
    """Tolerances and thresholds that drive matching.

    Args:
        amount_tolerance_minor: Absolute amount difference accepted (e.g. bank fees).
        date_window_days: Max days between due date and value date.
        max_group_size: Max invoices (1:N) or payments (N:1) in one group; below 2 disables
            group search.
        auto_threshold: Score at or above which a match is accepted automatically.
        suggest_threshold: Score at or above which a match is proposed for review.
        ambiguity_margin: If the best two candidates differ by less than this, flag ambiguous.
        weights: Relative weight of each signal in the final score; None selects the default
            weights.

    Raises:
        TypeError: An argument has the wrong type.
        OverflowError: An integer argument is outside the accepted range.
    """

    def __new__(
        cls,
        amount_tolerance_minor: int = 0,
        date_window_days: int = 30,
        max_group_size: int = 5,
        auto_threshold: float = 0.90,
        suggest_threshold: float = 0.60,
        ambiguity_margin: float = 0.05,
        weights: dict[str, float] | None = None,
    ) -> MatchConfig: ...
    @property
    def amount_tolerance_minor(self) -> int:
        """Absolute amount difference accepted (e.g. bank fees)."""

    @property
    def date_window_days(self) -> int:
        """Max days between due date and value date."""

    @property
    def max_group_size(self) -> int:
        """Max invoices (1:N) or payments (N:1) in one group; below 2 disables group search."""

    @property
    def auto_threshold(self) -> float:
        """Score at or above which a match is accepted automatically."""

    @property
    def suggest_threshold(self) -> float:
        """Score at or above which a match is proposed for review."""

    @property
    def ambiguity_margin(self) -> float:
        """If the best two candidates differ by less than this, flag ambiguous."""

    @property
    def weights(self) -> dict[str, float]:
        """Relative weight of each signal in the final score.

        Each access returns a new dict, so bind it to a variable before indexing
        repeatedly.
        """

    __hash__: None  # type: ignore[assignment]

@final
class Match:
    """A proposed link between payments and invoices.

    Args:
        kind: 1:1, 1:N or N:1.
        status: auto, suggested or ambiguous.
        payment_ids: Payments in the match.
        invoice_ids: Invoices in the match.
        score: Final weighted score in [0, 1].
        breakdown: Per-signal scores that explain the final score.
        residual_minor: Amount difference left after matching (fees, rounding).

    Raises:
        TypeError: kind or status is not a member of the matching enum, or another argument
            has the wrong type.
        OverflowError: An integer argument is outside the accepted range.
    """

    def __new__(
        cls,
        kind: MatchKind,
        status: MatchStatus,
        payment_ids: Sequence[str],
        invoice_ids: Sequence[str],
        score: float,
        breakdown: dict[str, float],
        residual_minor: int,
    ) -> Match: ...
    @property
    def kind(self) -> MatchKind:
        """Cardinality of the match: a MatchKind member."""

    @property
    def status(self) -> MatchStatus:
        """Confidence in the match: a MatchStatus member."""

    @property
    def payment_ids(self) -> tuple[str, ...]:
        """Payments in the match.

        Each access returns a new tuple, so bind it to a variable before indexing
        repeatedly.
        """

    @property
    def invoice_ids(self) -> tuple[str, ...]:
        """Invoices in the match.

        Each access returns a new tuple, so bind it to a variable before indexing
        repeatedly.
        """

    @property
    def score(self) -> float:
        """Final weighted score in [0, 1]."""

    @property
    def breakdown(self) -> dict[str, float]:
        """Per-signal scores that explain the final score.

        Each access returns a new dict, so bind it to a variable before indexing
        repeatedly.
        """

    @property
    def residual_minor(self) -> int:
        """Amount difference left after matching (fees, rounding)."""

    __hash__: None  # type: ignore[assignment]

@final
class ReconciliationResult:
    """Output of a reconciliation run.

    Args:
        matches: Proposed matches, best score first.
        unmatched_payments: Payment ids with no acceptable candidate.
        unmatched_invoices: Invoice ids with no acceptable candidate.

    Raises:
        TypeError: An argument has the wrong type.
    """

    def __new__(
        cls,
        matches: Sequence[Match],
        unmatched_payments: Sequence[str],
        unmatched_invoices: Sequence[str],
    ) -> ReconciliationResult: ...
    @property
    def matches(self) -> list[Match]:
        """Proposed matches, best score first.

        Each access returns a new list, so bind it to a variable before indexing
        repeatedly.
        """

    @property
    def unmatched_payments(self) -> list[str]:
        """Payment ids with no acceptable candidate.

        Each access returns a new list, so bind it to a variable before indexing
        repeatedly.
        """

    @property
    def unmatched_invoices(self) -> list[str]:
        """Invoice ids with no acceptable candidate.

        Each access returns a new list, so bind it to a variable before indexing
        repeatedly.
        """

    def to_arrow(self) -> pyarrow.Table:
        """Return matches as a flat Arrow table (one row per payment-invoice pair).

        Returns:
            A table with one row for each payment-invoice pair in every match.

        Raises:
            NotImplementedError: Always, because Arrow output is planned for a later release.
        """

    __hash__: None  # type: ignore[assignment]
