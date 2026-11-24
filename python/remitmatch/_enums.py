"""Enumerations shared by the matching engine and its results."""

from enum import Enum


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
