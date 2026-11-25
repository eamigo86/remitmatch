"""Match bank payments to open invoices and explain every match.

Build Invoice and Payment objects, with amounts as integers in minor units, and pass lists of
them to reconcile. It returns a ReconciliationResult whose Match objects carry the kind
(1:1, 1:N or N:1), the status, the score and a per-signal breakdown, plus the ids left
unmatched. Invoices and payments may also be passed as Arrow tables (pyarrow, Polars or any
object that exports the Arrow C stream), and ReconciliationResult.to_arrow returns the
matches as a pyarrow table; install the arrow extra for that. All types are immutable.
"""

from importlib import metadata

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

# The installed metadata carries the canonical Python form (such as 0.1.0a2); the native core
# reports the crate form (such as 0.1.0-alpha.2) through version().
__version__ = metadata.version("remitmatch")
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
