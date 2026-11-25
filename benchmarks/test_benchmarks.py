"""Tests of the benchmark data generator, the baselines and the metric computation."""

import datetime
import re

import pytest

pytest.importorskip("pandas")
pytest.importorskip("rapidfuzz")

import generate  # noqa: E402


def _covered(data: generate.Dataset) -> tuple[set[str], set[str]]:
    """Collect the payment ids and the invoice ids named by the solution.

    Args:
        data: Dataset to inspect.

    Returns:
        The payment ids and the invoice ids that appear in some group.
    """
    payments = {pid for group in data.solution for pid in group[0]}
    invoices = {iid for group in data.solution for iid in group[1]}
    return payments, invoices


def _compact(number: str) -> str:
    """Normalize an invoice number or a reference the way a person would compare them.

    Args:
        number: Text to normalize.

    Returns:
        The text in upper case without spaces, dashes or slashes.
    """
    return re.sub(r"[\s/-]", "", number).upper()


def test_same_seed_gives_the_same_data() -> None:
    """Two runs with one seed produce identical rows and an identical solution."""
    first = generate.generate(500, generate.MIXED, seed=3)
    second = generate.generate(500, generate.MIXED, seed=3)
    assert first == second


def test_another_seed_gives_other_data() -> None:
    """A different seed changes the rows."""
    assert generate.generate(500, generate.MIXED, seed=3) != generate.generate(
        500, generate.MIXED, seed=4
    )


@pytest.mark.parametrize("noise", [generate.ONE_TO_ONE, generate.MIXED, generate.CLEAN])
def test_the_payment_count_is_exact(noise: generate.Noise) -> None:
    """The generator returns exactly the requested number of payments.

    Args:
        noise: Noise profile under test.
    """
    assert len(generate.generate(1_234, noise, seed=1).payments) == 1_234


@pytest.mark.parametrize("noise", [generate.ONE_TO_ONE, generate.MIXED])
def test_the_solution_is_consistent_with_the_rows(noise: generate.Noise) -> None:
    """The solution names existing ids only, and no id sits in two groups.

    Args:
        noise: Noise profile under test.
    """
    data = generate.generate(3_000, noise, seed=5)
    payment_ids = [p.id for p in data.payments]
    invoice_ids = [i.id for i in data.invoices]
    assert len(set(payment_ids)) == len(payment_ids)
    assert len(set(invoice_ids)) == len(invoice_ids)
    listed_payments = [pid for group in data.solution for pid in group[0]]
    listed_invoices = [iid for group in data.solution for iid in group[1]]
    assert len(set(listed_payments)) == len(listed_payments)
    assert len(set(listed_invoices)) == len(listed_invoices)
    assert set(listed_payments) <= set(payment_ids)
    assert set(listed_invoices) <= set(invoice_ids)
    for payments, invoices in data.solution:
        assert payments == tuple(sorted(payments))
        assert invoices == tuple(sorted(invoices))
        assert payments and invoices


def test_one_to_one_has_only_groups_of_one_and_one() -> None:
    """Without grouped or partial payments every group is one payment and one invoice."""
    data = generate.generate(3_000, generate.ONE_TO_ONE, seed=2)
    assert data.solution
    assert all(len(p) == 1 and len(i) == 1 for p, i in data.solution)


def test_mixed_has_grouped_and_partial_payments() -> None:
    """The mixed profile contains one payment for several invoices and the reverse."""
    data = generate.generate(3_000, generate.MIXED, seed=2)
    assert any(len(i) > 1 for _, i in data.solution)
    assert any(len(p) > 1 for p, _ in data.solution)


def test_clean_data_has_no_noise() -> None:
    """Without noise every invoice is paid in full and every payment settles an invoice."""
    data = generate.generate(2_000, generate.CLEAN, seed=1)
    payments, invoices = _covered(data)
    assert len(payments) == len(data.payments)
    assert len(invoices) == len(data.invoices)
    amount = {i.id: i.amount_minor for i in data.invoices}
    paid = {p.id: p.amount_minor for p in data.payments}
    for pids, iids in data.solution:
        assert sum(paid[p] for p in pids) == sum(amount[i] for i in iids)


def test_amounts_are_positive_integers() -> None:
    """Every amount is a positive int, never a float."""
    data = generate.generate(5_000, generate.MIXED, seed=9)
    amounts = [p.amount_minor for p in data.payments] + [i.amount_minor for i in data.invoices]
    assert all(type(a) is int and a > 0 for a in amounts)


def test_dates_and_scopes() -> None:
    """Dates are dates, due dates follow issue dates, and the scopes are as requested."""
    data = generate.generate(4_000, generate.MIXED, seed=4, scopes=3)
    assert all(isinstance(p.value_date, datetime.date) for p in data.payments)
    assert all(i.issue_date < i.due_date for i in data.invoices)
    assert {i.scope for i in data.invoices} == {f"scope-{k}" for k in range(3)}
    assert {p.scope for p in data.payments} <= {i.scope for i in data.invoices}
    assert {i.currency for i in data.invoices} == {"EUR", "USD"}


def test_scopes_and_currencies_never_cross_inside_a_group() -> None:
    """Every payment and invoice of a group share the scope and the currency."""
    data = generate.generate(4_000, generate.MIXED, seed=6)
    payments = {p.id: p for p in data.payments}
    invoices = {i.id: i for i in data.invoices}
    for pids, iids in data.solution:
        keys = {(payments[p].scope, payments[p].currency) for p in pids}
        keys |= {(invoices[i].scope, invoices[i].currency) for i in iids}
        assert len(keys) == 1


def test_requested_noise_rates_are_roughly_met() -> None:
    """On a large sample each kind of noise appears at about its requested rate."""
    noise = generate.MIXED
    data = generate.generate(30_000, noise, seed=11)
    payments = {p.id: p for p in data.payments}
    invoices = {i.id: i for i in data.invoices}
    covered_payments, covered_invoices = _covered(data)

    unrelated = len(data.payments) - len(covered_payments)
    assert unrelated / len(data.payments) == pytest.approx(
        noise.unrelated / (1 + noise.unrelated), abs=0.002
    )
    assert 1 - len(covered_invoices) / len(data.invoices) == pytest.approx(noise.unpaid, abs=0.01)

    singles = [(p[0], i[0]) for p, i in data.solution if len(p) == 1 and len(i) == 1]
    assert len(singles) > 15_000
    fee = sum(payments[p].amount_minor < invoices[i].amount_minor for p, i in singles)
    assert fee / len(singles) == pytest.approx(noise.fee, abs=0.02)
    assert all(0 <= invoices[i].amount_minor - payments[p].amount_minor <= 300 for p, i in singles)
    abbreviated = sum(payments[p].counterparty != invoices[i].counterparty for p, i in singles)
    assert abbreviated / len(singles) == pytest.approx(noise.abbreviated_name, abs=0.02)
    missing = sum(payments[p].reference.startswith("TRANSFER ") for p, _ in singles)
    assert missing / len(singles) == pytest.approx(noise.missing_reference, abs=0.01)
    named = [(p, i) for p, i in singles if not payments[p].reference.startswith("TRANSFER ")]
    typos = sum(
        _compact(invoices[i].number) not in _compact(payments[p].reference) for p, i in named
    )
    assert typos / len(named) == pytest.approx(noise.typo, abs=0.01)

    in_groups = sum(len(i) > 1 for _, i in data.solution)
    grouped_invoices = sum(len(i) for _, i in data.solution if len(i) > 1)
    assert in_groups > 100
    # A batch that ends with one member is a plain payment, so the share of invoices in a
    # multi-invoice group stays below the probability of joining a batch.
    ceiling = noise.grouped * (1 - noise.unpaid) * (1 - noise.partial)
    assert 0.02 < grouped_invoices / len(data.invoices) < ceiling
    split = sum(len(p) > 1 for p, _ in data.solution)
    one_invoice_groups = sum(len(i) == 1 for _, i in data.solution)
    assert split / one_invoice_groups == pytest.approx(noise.partial, abs=0.01)


def test_evaluate_counts_a_group_as_correct_only_when_both_sides_are_equal() -> None:
    """Precision and recall use exact group equality, whatever the id order."""
    solution = [(("p1",), ("i1",)), (("p2", "p3"), ("i2",)), (("p4",), ("i3", "i4"))]
    found = [
        (("p1",), ("i1",)),
        (("p3", "p2"), ("i2",)),
        (("p4",), ("i3",)),
        (("p9",), ("i9",)),
    ]
    accuracy = generate.evaluate(found, solution)
    assert accuracy.correct == 2
    assert accuracy.proposed == 4
    assert accuracy.expected == 3
    assert accuracy.precision == pytest.approx(0.5)
    assert accuracy.recall == pytest.approx(2 / 3)


def test_evaluate_handles_an_empty_answer() -> None:
    """An empty answer has precision 1 by convention and recall 0."""
    accuracy = generate.evaluate([], [(("p1",), ("i1",))])
    assert accuracy.proposed == 0
    assert accuracy.precision == 1.0
    assert accuracy.recall == 0.0
