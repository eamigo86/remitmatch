"""Property tests: structural invariants of reconcile on small adversarial inputs."""

from collections.abc import Sequence
from datetime import date, timedelta

from hypothesis import Phase, given, settings
from hypothesis import strategies as st
from remitmatch import (
    Invoice,
    MatchConfig,
    MatchKind,
    MatchStatus,
    Payment,
    ReconciliationResult,
    reconcile,
)

# Small alphabets, so that collisions, shared invoice numbers and groups actually occur.
SCOPES = ["a", "b"]
CURRENCIES = ["EUR", "eur", "USD"]
NAMES = ["Acme S.L.", "ACME", "José Pérez", "Beta Corp"]
NOISE_TOKENS = ["TRANSFER", "pago", "x1", "12", "-"]
START = date(2024, 1, 1)
SIGNALS = ["amount", "date", "name", "reference"]

Inputs = tuple[list[Invoice], list[Payment], MatchConfig]
InvoiceRaw = tuple[int, int, int, int, int, int]
PaymentRaw = tuple[int, int, int, list[int], bool, int, int]

# (scope, currency, amount, number, due offset, name)
invoice_raw = st.tuples(
    st.integers(0, 1),
    st.integers(0, 2),
    st.integers(-2, 40),
    st.integers(0, 13),
    st.integers(0, 89),
    st.integers(0, 3),
)
# (scope, currency, amount, reference tokens, glued, date offset, name)
payment_raw = st.tuples(
    st.integers(0, 1),
    st.integers(0, 2),
    st.integers(-2, 40),
    st.lists(st.integers(0, 17), max_size=3),
    st.booleans(),
    st.integers(0, 89),
    st.integers(0, 3),
)
weights_raw = st.one_of(
    st.none(),
    st.tuples(*[st.floats(0.0, 1.0)] * 4),
)


def build_invoice(index: int, raw: InvoiceRaw) -> Invoice:
    """Turn a drawn tuple into an invoice with a unique id.

    Args:
        index: Position of the invoice, used for its id.
        raw: Drawn scope, currency, amount, number, due offset and name indexes.

    Returns:
        The invoice.
    """
    scope, currency, amount, number, due, name = raw
    due_date = START + timedelta(days=due)
    return Invoice(
        f"i{index}",
        "" if number > 12 else f"INV-{number}",
        amount,
        CURRENCIES[currency],
        due_date,
        due_date,
        NAMES[name],
        SCOPES[scope],
    )


def build_payment(index: int, raw: PaymentRaw) -> Payment:
    """Turn a drawn tuple into a payment with a unique id.

    Args:
        index: Position of the payment, used for its id.
        raw: Drawn scope, currency, amount, reference tokens, glue flag, date offset and name.

    Returns:
        The payment.
    """
    scope, currency, amount, tokens, glued, offset, name = raw
    parts = [f"INV-{t}" if t <= 12 else NOISE_TOKENS[t - 13] for t in tokens]
    return Payment(
        f"p{index}",
        amount,
        CURRENCIES[currency],
        START + timedelta(days=offset),
        ("" if glued else " ").join(parts),
        NAMES[name],
        SCOPES[scope],
    )


@st.composite
def configs(draw: st.DrawFn) -> MatchConfig:
    """Draw a valid configuration.

    Args:
        draw: Hypothesis draw function.

    Returns:
        A configuration with ordered thresholds and a positive weight sum.
    """
    first, second = draw(st.floats(0.0, 1.0)), draw(st.floats(0.0, 1.0))
    raw_weights = draw(weights_raw)
    # The reference weight is shifted so that the sum is always positive.
    weights = (
        None
        if raw_weights is None
        else dict(zip(SIGNALS, (raw_weights[3] + 0.01, *raw_weights[:3]), strict=True))
    )
    return MatchConfig(
        amount_tolerance_minor=draw(st.integers(0, 5)),
        date_window_days=draw(st.integers(0, 60)),
        max_group_size=draw(st.integers(0, 4)),
        auto_threshold=max(first, second),
        suggest_threshold=min(first, second),
        ambiguity_margin=draw(st.floats(0.0, 0.2, exclude_max=True)),
        weights=weights,
    )


@st.composite
def groups(draw: st.DrawFn) -> tuple[list[InvoiceRaw], list[PaymentRaw]]:
    """Draw one group that the engine can match: a 1:N or an N:1 shape.

    Args:
        draw: Hypothesis draw function.

    Returns:
        The raw invoices and payments of the group. Either one payment that names two
        invoices and pays their sum, or two payments that name one invoice and split it.
    """
    scope, currency, name, due = (
        draw(st.integers(0, 1)),
        draw(st.integers(0, 2)),
        draw(st.integers(0, 3)),
        draw(st.integers(0, 89)),
    )
    first, second = draw(st.integers(1, 20)), draw(st.integers(1, 20))
    one, other = draw(st.sampled_from([(0, 1), (2, 5), (7, 12)]))
    if draw(st.booleans()):
        return (
            [
                (scope, currency, first, one, due, name),
                (scope, currency, second, other, due, name),
            ],
            [(scope, currency, first + second, [one, other], False, due, name)],
        )
    return (
        [(scope, currency, first + second, one, due, name)],
        [
            (scope, currency, first, [one], False, due, name),
            (scope, currency, second, [one], False, due, name),
        ],
    )


@st.composite
def inputs(draw: st.DrawFn) -> Inputs:
    """Draw invoices, payments and a configuration.

    Args:
        draw: Hypothesis draw function.

    Returns:
        Invoices and payments with unique ids, mixing random rows with a few matchable
        groups so that 1:N and N:1 matches are common, and a configuration.
    """
    invoices = draw(st.lists(invoice_raw, max_size=8))
    payments = draw(st.lists(payment_raw, max_size=8))
    for group_invoices, group_payments in draw(st.lists(groups(), max_size=3)):
        invoices += group_invoices
        payments += group_payments
    return (
        [build_invoice(n, raw) for n, raw in enumerate(invoices)],
        [build_payment(n, raw) for n, raw in enumerate(payments)],
        draw(configs()),
    )


def check(
    invoices: Sequence[Invoice],
    payments: Sequence[Payment],
    config: MatchConfig,
    result: ReconciliationResult,
) -> None:
    """Assert every structural invariant of one run.

    Args:
        invoices: The invoices that were reconciled.
        payments: The payments that were reconciled.
        config: The configuration used.
        result: The result to check.
    """
    invoice_by_id = {invoice.id: invoice for invoice in invoices}
    payment_by_id = {payment.id: payment for payment in payments}
    seen_invoices: set[str] = set()
    seen_payments: set[str] = set()
    for match in result.matches:
        shape = (len(match.payment_ids), len(match.invoice_ids))
        if shape == (1, 1):
            assert match.kind is MatchKind.ONE_TO_ONE
        elif shape[0] == 1:
            assert match.kind is MatchKind.ONE_TO_MANY
            assert 2 <= len(match.invoice_ids) <= config.max_group_size
        else:
            assert shape[1] == 1, f"bad group shape {shape}"
            assert match.kind is MatchKind.MANY_TO_ONE
            assert 2 <= len(match.payment_ids) <= config.max_group_size
        matched_payments = [payment_by_id[pid] for pid in match.payment_ids]
        matched_invoices = [invoice_by_id[iid] for iid in match.invoice_ids]
        for payment in matched_payments:
            assert payment.id not in seen_payments, f"payment {payment.id} matched twice"
            seen_payments.add(payment.id)
            assert payment.amount_minor > 0
            for invoice in matched_invoices:
                assert payment.scope == invoice.scope
                assert payment.currency.lower() == invoice.currency.lower()
        for invoice in matched_invoices:
            assert invoice.id not in seen_invoices, f"invoice {invoice.id} matched twice"
            seen_invoices.add(invoice.id)
            assert invoice.amount_minor > 0
        payment_sum = sum(p.amount_minor for p in matched_payments)
        invoice_sum = sum(i.amount_minor for i in matched_invoices)
        assert abs(payment_sum - invoice_sum) <= config.amount_tolerance_minor
        assert match.residual_minor == payment_sum - invoice_sum
        assert sorted(match.breakdown) == SIGNALS
        assert 0.0 <= match.score <= 1.0
        assert all(0.0 <= value <= 1.0 for value in match.breakdown.values())
        assert match.score >= config.suggest_threshold - 1e-9
        if match.status is MatchStatus.AUTO:
            assert match.score >= config.auto_threshold - 1e-9
    for pid in result.unmatched_payments:
        assert pid not in seen_payments, f"payment {pid} both matched and unmatched"
        seen_payments.add(pid)
    for iid in result.unmatched_invoices:
        assert iid not in seen_invoices, f"invoice {iid} both matched and unmatched"
        seen_invoices.add(iid)
    assert seen_payments == set(payment_by_id)
    assert seen_invoices == set(invoice_by_id)
    scores = [match.score for match in result.matches]
    assert scores == sorted(scores, reverse=True)


@settings(max_examples=300, deadline=None, database=None)
@given(inputs())
def test_invariants_hold_on_adversarial_inputs(data: Inputs) -> None:
    """No id is matched twice, every id is accounted for once and groups add up.

    Args:
        data: Drawn invoices, payments and configuration.
    """
    invoices, payments, config = data
    result = reconcile(invoices, payments, config)
    check(invoices, payments, config, result)
    assert reconcile(invoices, payments, config) == result


def test_group_invariants_run_on_many_group_matches() -> None:
    """Every invariant is checked on a derandomized run that yields many matches of each kind."""
    counts = dict.fromkeys(MatchKind, 0)

    @settings(
        derandomize=True,
        max_examples=1000,
        deadline=None,
        database=None,
        phases=[Phase.generate],
    )
    @given(inputs())
    def run(data: Inputs) -> None:
        invoices, payments, config = data
        result = reconcile(invoices, payments, config)
        check(invoices, payments, config, result)
        for match in result.matches:
            counts[match.kind] += 1

    run()
    assert all(count >= 200 for count in counts.values()), counts
