"""Enumerations shared by the matching engine and its results."""

from enum import Enum


class MatchKind(Enum):
    """Cardinality of a match.

    Attributes:
        ONE_TO_ONE: One payment settles one invoice.
        ONE_TO_MANY: One payment settles several invoices.
        MANY_TO_ONE: Several payments settle one invoice.
    """

    ONE_TO_ONE = "1:1"
    ONE_TO_MANY = "1:N"
    MANY_TO_ONE = "N:1"


class MatchStatus(Enum):
    """How confident the engine is in a proposed match.

    Attributes:
        AUTO: Accepted automatically.
        SUGGESTED: Proposed for review.
        AMBIGUOUS: More than one plausible candidate exists.
    """

    AUTO = "auto"
    SUGGESTED = "suggested"
    AMBIGUOUS = "ambiguous"
