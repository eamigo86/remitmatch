"""Tests for reconcile through the Python API."""

from concurrent.futures import ThreadPoolExecutor
from datetime import date

import pytest
from remitmatch import (
    Invoice,
    MatchConfig,
    MatchKind,
    MatchStatus,
    Payment,
    ReconciliationResult,
    reconcile,
)

DAY = date(2024, 1, 10)


def inv(id: str, number: str, amount_minor: int) -> Invoice:
    """Build an invoice for Acme in the main scope.

    Args:
        id: Invoice identifier.
        number: Invoice number searched for in references.
        amount_minor: Amount in minor units.

    Returns:
        The invoice.
    """
    return Invoice(id, number, amount_minor, "EUR", DAY, DAY, "Acme S.L.", "main")


def pay(id: str, amount_minor: int, reference: str) -> Payment:
    """Build a payment from Acme in the main scope.

    Args:
        id: Payment identifier.
        amount_minor: Amount in minor units.
        reference: Remittance reference.

    Returns:
        The payment.
    """
    return Payment(id, amount_minor, "EUR", DAY, reference, "ACME", "main")


def test_one_to_one_match() -> None:
    """A payment that names one invoice with the same amount is an automatic 1:1 match."""
    result = reconcile([inv("i1", "INV-1", 10_000)], [pay("p1", 10_000, "Payment INV-1")])
    assert len(result.matches) == 1
    match = result.matches[0]
    assert match.kind is MatchKind.ONE_TO_ONE
    assert match.status is MatchStatus.AUTO
    assert match.payment_ids == ("p1",)
    assert match.invoice_ids == ("i1",)
    assert match.residual_minor == 0
    assert set(match.breakdown) == {"reference", "amount", "name", "date"}
    assert 0.9 <= match.score <= 1.0
    assert result.unmatched_payments == []
    assert result.unmatched_invoices == []


def test_one_to_many_match() -> None:
    """One payment that names two invoices adding up to it is a 1:N match."""
    result = reconcile(
        [inv("i1", "INV-1", 6_000), inv("i2", "INV-2", 4_000)],
        [pay("p1", 10_000, "INV-2 INV-1")],
    )
    assert len(result.matches) == 1
    match = result.matches[0]
    assert match.kind is MatchKind.ONE_TO_MANY
    assert match.status is MatchStatus.AUTO
    assert match.payment_ids == ("p1",)
    assert match.invoice_ids == ("i1", "i2")


def test_many_to_one_match() -> None:
    """Two payments that name one invoice and add up to it are an N:1 match."""
    result = reconcile(
        [inv("i1", "INV-1", 10_000)],
        [pay("p1", 4_000, "INV-1"), pay("p2", 6_000, "INV-1")],
    )
    assert len(result.matches) == 1
    match = result.matches[0]
    assert match.kind is MatchKind.MANY_TO_ONE
    assert match.payment_ids == ("p1", "p2")
    assert match.invoice_ids == ("i1",)


def test_unmatched_ids_keep_input_order() -> None:
    """Rows with no acceptable candidate are listed in input order."""
    result = reconcile(
        [inv("i1", "INV-1", 10_000), inv("i2", "INV-2", 777), inv("i3", "INV-3", 888)],
        [pay("p1", 10_000, "INV-1"), pay("p2", 1, "unrelated")],
    )
    assert result.unmatched_payments == ["p2"]
    assert result.unmatched_invoices == ["i2", "i3"]


def test_tuples_and_lists_are_accepted() -> None:
    """Inputs may be lists or tuples and give the same result."""
    invoices = [inv("i1", "INV-1", 10_000)]
    payments = [pay("p1", 10_000, "INV-1")]
    assert reconcile(tuple(invoices), tuple(payments)) == reconcile(invoices, payments)


def test_default_config_equals_explicit_default() -> None:
    """Omitting the config or passing None equals passing MatchConfig()."""
    invoices = [inv("i1", "INV-1", 6_000), inv("i2", "INV-2", 4_000)]
    payments = [pay("p1", 10_000, "INV-1 INV-2")]
    expected = reconcile(invoices, payments, MatchConfig())
    assert reconcile(invoices, payments) == expected
    assert reconcile(invoices, payments, None) == expected
    assert reconcile(invoices, payments, config=MatchConfig()) == expected


def test_custom_config_changes_the_outcome() -> None:
    """A tolerance lets a group with a bank fee match and reports the residual."""
    invoices = [inv("i1", "INV-1", 6_000), inv("i2", "INV-2", 4_000)]
    payments = [pay("p1", 9_970, "INV-1 INV-2")]
    assert reconcile(invoices, payments).matches == []
    result = reconcile(invoices, payments, MatchConfig(amount_tolerance_minor=150))
    assert len(result.matches) == 1
    assert result.matches[0].kind is MatchKind.ONE_TO_MANY
    assert result.matches[0].residual_minor == -30


def test_empty_input() -> None:
    """Empty sequences give an empty result."""
    assert reconcile([], []) == ReconciliationResult([], [], [])
    assert reconcile([inv("i1", "INV-1", 1)], []).unmatched_invoices == ["i1"]
    assert reconcile([], [pay("p1", 1, "x")]).unmatched_payments == ["p1"]


def test_extreme_amounts_do_not_crash() -> None:
    """Amounts at the edge of i64 are handled without wrapping or crashing."""
    top = 2**63 - 1
    result = reconcile([inv("i1", "INV-1", top)], [pay("p1", top, "INV-1")])
    assert len(result.matches) == 1
    assert result.matches[0].residual_minor == 0


def test_duplicate_ids_raise_value_error() -> None:
    """A repeated id on either side is a ValueError with the core message."""
    with pytest.raises(ValueError, match="duplicate invoice id: i1"):
        reconcile([inv("i1", "INV-1", 1), inv("i1", "INV-2", 2)], [])
    with pytest.raises(ValueError, match="duplicate payment id: p1"):
        reconcile([], [pay("p1", 1, "a"), pay("p1", 2, "b")])


def test_invalid_config_raises_value_error() -> None:
    """A configuration the core rejects is a ValueError."""
    with pytest.raises(ValueError, match="invalid configuration"):
        reconcile([], [], MatchConfig(auto_threshold=2.0))
    with pytest.raises(ValueError, match="invalid configuration"):
        reconcile([], [], MatchConfig(weights={"unknown": 1.0}))


@pytest.mark.parametrize("bad", [None, 5, "text", {"i1": 1}, {inv("i1", "INV-1", 1)}])
def test_invoices_must_be_a_sequence_of_invoices(bad: object) -> None:
    """Anything but a sequence of Invoice objects raises TypeError.

    Args:
        bad: A value that is not a sequence of invoices.
    """
    with pytest.raises(TypeError):
        reconcile(bad, [])  # type: ignore[arg-type]


@pytest.mark.parametrize("bad", [None, 5, "text", {"p1": 1}])
def test_payments_must_be_a_sequence_of_payments(bad: object) -> None:
    """Anything but a sequence of Payment objects raises TypeError.

    Args:
        bad: A value that is not a sequence of payments.
    """
    with pytest.raises(TypeError):
        reconcile([], bad)  # type: ignore[arg-type]


def test_wrong_element_and_config_types_raise_type_error() -> None:
    """Elements of the wrong class and a non-MatchConfig config raise TypeError."""
    with pytest.raises(TypeError):
        reconcile([pay("p1", 1, "x")], [])  # type: ignore[list-item]
    with pytest.raises(TypeError):
        reconcile([], [inv("i1", "INV-1", 1)])  # type: ignore[list-item]
    with pytest.raises(TypeError):
        reconcile([], [], {"amount_tolerance_minor": 1})  # type: ignore[arg-type]


def test_concurrent_calls_agree() -> None:
    """Many threads can reconcile at once and all get the single-threaded result."""
    invoices = [inv(f"i{n}", f"INV-{n}", 1_000 + n) for n in range(60)]
    payments = [pay(f"p{n}", 1_000 + n, f"INV-{n}") for n in range(60)]
    expected = reconcile(invoices, payments)
    assert len(expected.matches) == 60
    with ThreadPoolExecutor(max_workers=8) as pool:
        results = list(pool.map(lambda _: reconcile(invoices, payments), range(32)))
    assert all(result == expected for result in results)
