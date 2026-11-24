"""Match bank payments to open invoices and explain every match.

Version 0.0.1 reserves the package name. The matching engine is planned for 0.1.0.
"""

from remitmatch._core import (
    Invoice,
    Match,
    MatchConfig,
    Payment,
    ReconciliationResult,
    reconcile,
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
