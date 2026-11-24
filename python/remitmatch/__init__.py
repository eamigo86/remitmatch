"""Match bank payments to open invoices and explain every match.

Version 0.0.1 reserves the package name. The matching engine is planned for 0.1.0.
"""

from typing import NoReturn

from remitmatch._core import (
    Invoice,
    Match,
    MatchConfig,
    Payment,
    ReconciliationResult,
    version,
)
from remitmatch._enums import MatchKind, MatchStatus

__version__ = version()
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
    raise NotImplementedError("planned for 0.1.0")
