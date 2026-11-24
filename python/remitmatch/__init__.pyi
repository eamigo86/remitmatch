"""Match bank payments to open invoices and explain every match."""

from typing import NoReturn

from remitmatch._core import Invoice as Invoice
from remitmatch._core import Match as Match
from remitmatch._core import MatchConfig as MatchConfig
from remitmatch._core import Payment as Payment
from remitmatch._core import ReconciliationResult as ReconciliationResult
from remitmatch._core import version as version
from remitmatch._enums import MatchKind as MatchKind
from remitmatch._enums import MatchStatus as MatchStatus

__all__ = [
    "Invoice",
    "Match",
    "MatchConfig",
    "MatchKind",
    "MatchStatus",
    "Payment",
    "ReconciliationResult",
    "__version__",
    "reconcile",
    "version",
]

__version__: str

def reconcile(
    invoices: object,
    payments: object,
    config: object | None = None,
) -> NoReturn:
    """Match payments against open invoices.

    Args:
        invoices: Open invoices to settle.
        payments: Bank movements received.
        config: Tolerances and thresholds; defaults to a standard configuration.

    Raises:
        NotImplementedError: Always, because the matching engine is not available yet.
    """
