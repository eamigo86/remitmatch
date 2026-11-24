"""Tests for the native data model classes and the Python enums."""

from datetime import date
from enum import Enum

import pytest
from remitmatch import (
    Invoice,
    Match,
    MatchConfig,
    MatchKind,
    MatchStatus,
    Payment,
    ReconciliationResult,
)

DEFAULT_WEIGHTS = {"reference": 0.45, "amount": 0.30, "name": 0.15, "date": 0.10}


def make_invoice() -> Invoice:
    """Build an invoice with the default scope.

    Returns:
        A sample invoice.
    """
    return Invoice("i1", "INV-1", 10_000, "EUR", date(2024, 1, 1), date(2024, 1, 31), "Acme")


def make_payment() -> Payment:
    """Build a payment with the default scope.

    Returns:
        A sample payment.
    """
    return Payment("p1", 10_000, "EUR", date(2024, 2, 1), "Payment INV-1", "ACME")


def make_match() -> Match:
    """Build a one to one match.

    Returns:
        A sample match.
    """
    return Match(
        MatchKind.ONE_TO_ONE,
        MatchStatus.AUTO,
        ["p1"],
        ["i1"],
        0.95,
        {"reference": 1.0, "amount": 1.0},
        0,
    )


def test_invoice_exposes_fields_and_default_scope() -> None:
    """Positional arguments map to fields in RFC order and the scope defaults."""
    invoice = make_invoice()
    assert invoice.id == "i1"
    assert invoice.number == "INV-1"
    assert invoice.amount_minor == 10_000
    assert invoice.currency == "EUR"
    assert invoice.issue_date == date(2024, 1, 1)
    assert invoice.due_date == date(2024, 1, 31)
    assert invoice.counterparty == "Acme"
    assert invoice.scope == "default"


def test_invoice_accepts_keyword_arguments() -> None:
    """Every field can be passed by keyword, including the scope."""
    invoice = Invoice(
        id="i2",
        number="INV-2",
        amount_minor=5,
        currency="USD",
        issue_date=date(2024, 3, 1),
        due_date=date(2024, 3, 2),
        counterparty="Beta",
        scope="eu",
    )
    assert (invoice.id, invoice.amount_minor, invoice.scope) == ("i2", 5, "eu")


def test_payment_exposes_fields_and_default_scope() -> None:
    """Payment fields follow the RFC order and the scope defaults."""
    payment = make_payment()
    assert payment.id == "p1"
    assert payment.amount_minor == 10_000
    assert payment.currency == "EUR"
    assert payment.value_date == date(2024, 2, 1)
    assert payment.reference == "Payment INV-1"
    assert payment.counterparty == "ACME"
    assert payment.scope == "default"


def test_match_config_defaults_follow_the_rfc() -> None:
    """A bare MatchConfig carries the RFC defaults, including the weights."""
    config = MatchConfig()
    assert config.amount_tolerance_minor == 0
    assert config.date_window_days == 30
    assert config.max_group_size == 5
    assert config.auto_threshold == 0.90
    assert config.suggest_threshold == 0.60
    assert config.ambiguity_margin == 0.05
    assert config.weights == DEFAULT_WEIGHTS
    assert MatchConfig(weights=None) == config


def test_match_config_accepts_custom_values() -> None:
    """Custom values and weights are stored as given."""
    config = MatchConfig(
        amount_tolerance_minor=150,
        date_window_days=10,
        max_group_size=3,
        auto_threshold=0.8,
        suggest_threshold=0.5,
        ambiguity_margin=0.1,
        weights={"reference": 1.0},
    )
    assert config.amount_tolerance_minor == 150
    assert config.date_window_days == 10
    assert config.max_group_size == 3
    assert config.weights == {"reference": 1.0}
    assert config != MatchConfig()


def test_match_exposes_fields_with_enum_members() -> None:
    """Kind and status come back as members of the Python enums."""
    match = make_match()
    assert match.kind is MatchKind.ONE_TO_ONE
    assert match.status is MatchStatus.AUTO
    assert match.payment_ids == ("p1",)
    assert match.invoice_ids == ("i1",)
    assert match.score == 0.95
    assert match.breakdown == {"reference": 1.0, "amount": 1.0}
    assert match.residual_minor == 0


def test_result_exposes_fields() -> None:
    """The result keeps matches and unmatched ids as lists."""
    result = ReconciliationResult([make_match()], ["p2"], ["i2", "i3"])
    assert result.matches == [make_match()]
    assert result.unmatched_payments == ["p2"]
    assert result.unmatched_invoices == ["i2", "i3"]


def test_containers_are_fresh_copies() -> None:
    """Mutating a returned dict, tuple holder or list never changes the object."""
    config = MatchConfig()
    config.weights["reference"] = 0.0
    assert config.weights == DEFAULT_WEIGHTS
    match = make_match()
    match.breakdown["amount"] = 0.0
    assert match.breakdown == {"reference": 1.0, "amount": 1.0}
    assert isinstance(match.payment_ids, tuple)
    result = ReconciliationResult([make_match()], ["p2"], [])
    result.matches.clear()
    result.unmatched_payments.append("p9")
    assert len(result.matches) == 1
    assert result.unmatched_payments == ["p2"]
    assert result.matches is not result.matches


@pytest.mark.parametrize(
    "instance",
    [
        make_invoice(),
        make_payment(),
        MatchConfig(),
        make_match(),
        ReconciliationResult([], [], []),
    ],
)
def test_attributes_are_read_only(instance: object) -> None:
    """Assigning an existing attribute or adding a new one raises AttributeError."""
    name = next(name for name in dir(instance) if not name.startswith("_"))
    with pytest.raises(AttributeError):
        setattr(instance, name, 1)
    with pytest.raises(AttributeError):
        instance.extra = 1  # type: ignore[attr-defined]


def test_equality_compares_field_by_field() -> None:
    """Equal field values compare equal and any difference breaks equality."""
    assert make_invoice() == make_invoice()
    assert make_invoice() != Invoice(
        "i1", "INV-1", 10_001, "EUR", date(2024, 1, 1), date(2024, 1, 31), "Acme"
    )
    assert make_payment() == make_payment()
    other: object = make_invoice()
    assert make_payment() != other
    assert make_match() == make_match()
    other_match = Match(MatchKind.ONE_TO_MANY, MatchStatus.AUTO, ["p1"], ["i1"], 0.95, {}, 0)
    assert make_match() != other_match
    assert ReconciliationResult([], [], []) == ReconciliationResult([], [], [])
    assert ReconciliationResult([], ["p1"], []) != ReconciliationResult([], [], [])


def test_invoice_and_payment_are_hashable() -> None:
    """Equal invoices and payments hash alike and work in sets."""
    assert hash(make_invoice()) == hash(make_invoice())
    assert len({make_invoice(), make_invoice()}) == 1
    assert hash(make_payment()) == hash(make_payment())
    assert len({make_payment(), make_payment()}) == 1


@pytest.mark.parametrize(
    "instance",
    [MatchConfig(), make_match(), ReconciliationResult([], [], [])],
)
def test_other_types_are_unhashable(instance: object) -> None:
    """Types that hold a dict or a list are not hashable."""
    with pytest.raises(TypeError):
        hash(instance)


def test_repr_is_dataclass_style() -> None:
    """The repr lists every field as name=value."""
    assert repr(make_invoice()) == (
        "Invoice(id='i1', number='INV-1', amount_minor=10000, currency='EUR', "
        "issue_date=datetime.date(2024, 1, 1), due_date=datetime.date(2024, 1, 31), "
        "counterparty='Acme', scope='default')"
    )
    assert repr(make_payment()).startswith("Payment(id='p1', amount_minor=10000, currency='EUR', ")
    assert repr(MatchConfig()).startswith(
        "MatchConfig(amount_tolerance_minor=0, date_window_days=30, "
    )
    assert repr(make_match()).startswith("Match(kind=<MatchKind.ONE_TO_ONE: '1:1'>, ")
    assert repr(ReconciliationResult([], ["p1"], [])) == (
        "ReconciliationResult(matches=[], unmatched_payments=['p1'], unmatched_invoices=[])"
    )


def test_enums_are_real_enums_with_rfc_values() -> None:
    """MatchKind and MatchStatus are Enum classes with the RFC names and values."""
    assert issubclass(MatchKind, Enum)
    assert issubclass(MatchStatus, Enum)
    assert {member.name: member.value for member in MatchKind} == {
        "ONE_TO_ONE": "1:1",
        "ONE_TO_MANY": "1:N",
        "MANY_TO_ONE": "N:1",
    }
    assert {member.name: member.value for member in MatchStatus} == {
        "AUTO": "auto",
        "SUGGESTED": "suggested",
        "AMBIGUOUS": "ambiguous",
    }


def test_constructors_reject_wrong_types() -> None:
    """Wrong argument types surface as TypeError, never as a crash."""
    with pytest.raises(TypeError):
        Invoice(1, "n", 1, "EUR", date(2024, 1, 1), date(2024, 1, 1), "x")  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        Invoice("i", "n", 1, "EUR", "2024-01-01", date(2024, 1, 1), "x")  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        Payment("p", 1.5, "EUR", date(2024, 1, 1), "r", "x")  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        MatchConfig(weights=["reference"])  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        Match("1:1", MatchStatus.AUTO, [], [], 1.0, {}, 0)  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        Match(MatchKind.ONE_TO_ONE, MatchStatus.AUTO, "p1", [], 1.0, {}, 0)
    with pytest.raises(TypeError):
        ReconciliationResult([make_invoice()], [], [])  # type: ignore[list-item]


def test_out_of_range_numbers_raise() -> None:
    """Integers that do not fit the native type raise instead of wrapping around."""
    with pytest.raises(OverflowError):
        Payment("p", 2**63, "EUR", date(2024, 1, 1), "r", "x")
    with pytest.raises(OverflowError):
        MatchConfig(date_window_days=-1)
    with pytest.raises(OverflowError):
        MatchConfig(max_group_size=-1)
