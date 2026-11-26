"""Deterministic synthetic data with a known solution, for the benchmarks.

The noise model mirrors the Rust generator used by the accuracy tests
(crates/remitmatch/tests/common/mod.rs): bank fees, misspelled references, missing
references, abbreviated payer names, unpaid invoices, unrelated payments and, optionally,
grouped and partial payments. Everything is drawn from random.Random with the given seed, so
the same arguments always give the same rows and the same solution.
"""

from __future__ import annotations

import datetime
import random
from collections.abc import Iterable, Sequence
from dataclasses import dataclass
from typing import NamedTuple

__all__ = [
    "CLEAN",
    "DATASETS",
    "MIXED",
    "ONE_TO_ONE",
    "Accuracy",
    "Dataset",
    "Group",
    "Invoice",
    "Noise",
    "Payment",
    "evaluate",
    "generate",
]


class Invoice(NamedTuple):
    """An open invoice.

    Attributes:
        id: Unique identifier.
        number: Invoice number as printed on the invoice.
        amount_minor: Amount in minor units (cents).
        currency: Currency code.
        issue_date: Date the invoice was issued.
        due_date: Date the invoice is due.
        counterparty: Customer name.
        scope: Partition key such as a company or tenant id.
    """

    id: str
    number: str
    amount_minor: int
    currency: str
    issue_date: datetime.date
    due_date: datetime.date
    counterparty: str
    scope: str


class Payment(NamedTuple):
    """A bank payment.

    Attributes:
        id: Unique identifier.
        amount_minor: Amount in minor units (cents).
        currency: Currency code.
        value_date: Date the money arrived.
        reference: Free text written by the payer.
        counterparty: Payer name as the bank reports it.
        scope: Partition key such as a company or tenant id.
    """

    id: str
    amount_minor: int
    currency: str
    value_date: datetime.date
    reference: str
    counterparty: str
    scope: str


Group = tuple[tuple[str, ...], tuple[str, ...]]
"""A solution group: the payment ids and the invoice ids it links, each sorted ascending."""


@dataclass(frozen=True)
class Noise:
    """Probability of each kind of noise, each between 0 and 1.

    Attributes:
        fee: A payment is short by a bank fee of 1 to 300 minor units.
        typo: The invoice number in the reference has exactly one edit.
        missing_reference: The reference carries no invoice number.
        abbreviated_name: The payer name is a bank-style variant of the customer name.
        unpaid: An invoice has no payment.
        unrelated: Payments that settle nothing, as a fraction of the other payments.
        grouped: An invoice joins a batch of 2 to 4 invoices paid with one payment.
        partial: An invoice is paid by 2 or 3 payments that add up to its amount.
    """

    fee: float = 0.0
    typo: float = 0.0
    missing_reference: float = 0.0
    abbreviated_name: float = 0.0
    unpaid: float = 0.0
    unrelated: float = 0.0
    grouped: float = 0.0
    partial: float = 0.0


CLEAN = Noise()
"""No noise at all: every invoice is paid in full by one payment."""

MIXED = Noise(
    fee=0.15,
    typo=0.05,
    missing_reference=0.05,
    abbreviated_name=0.30,
    unpaid=0.10,
    unrelated=0.05,
    grouped=0.08,
    partial=0.05,
)
"""The realistic noise level of the accuracy tests, with grouped and partial payments."""

ONE_TO_ONE = Noise(
    fee=MIXED.fee,
    typo=MIXED.typo,
    missing_reference=MIXED.missing_reference,
    abbreviated_name=MIXED.abbreviated_name,
    unpaid=MIXED.unpaid,
    unrelated=MIXED.unrelated,
)
"""The realistic noise level without grouped or partial payments."""

DATASETS: dict[str, Noise] = {"one-to-one": ONE_TO_ONE, "mixed": MIXED}
"""The two benchmark datasets by name."""


@dataclass(frozen=True)
class Dataset:
    """Generated rows plus the correct pairing.

    Attributes:
        invoices: Open invoices in creation order.
        payments: Payments in shuffled order, so input order carries no information.
        solution: The true groups. A one-to-one payment is a group of one payment and one
            invoice; payments that settle nothing and unpaid invoices appear in no group.
    """

    invoices: list[Invoice]
    payments: list[Payment]
    solution: list[Group]


class Accuracy(NamedTuple):
    """Precision and recall of a set of proposed groups.

    Attributes:
        correct: Proposed groups equal to a solution group.
        proposed: Groups proposed.
        expected: Groups in the solution.
        precision: Share of proposed groups that are correct; 1.0 when nothing was proposed.
        recall: Share of solution groups that were proposed.
    """

    correct: int
    proposed: int
    expected: int
    precision: float
    recall: float


def evaluate(
    found: Iterable[tuple[Iterable[str], Iterable[str]]], solution: Sequence[Group]
) -> Accuracy:
    """Score proposed groups against the solution.

    A proposed group is correct when its payment ids and its invoice ids both equal those of
    a solution group, whatever their order. The same rule applies to every implementation.

    Args:
        found: Proposed groups as pairs of payment ids and invoice ids.
        solution: The true groups.

    Returns:
        The counts, the precision and the recall.
    """
    truth = {(tuple(sorted(p)), tuple(sorted(i))) for p, i in solution}
    proposed = 0
    correct = 0
    for payment_ids, invoice_ids in found:
        proposed += 1
        correct += (tuple(sorted(payment_ids)), tuple(sorted(invoice_ids))) in truth
    return Accuracy(
        correct=correct,
        proposed=proposed,
        expected=len(truth),
        precision=correct / proposed if proposed else 1.0,
        recall=correct / len(truth) if truth else 1.0,
    )


_FIRST = (
    "Construcciones",
    "Transportes",
    "Industrias",
    "Servicios",
    "Distribuciones",
    "Maderas",
    "Aceros",
    "Suministros",
)
_SURNAMES = ("Ibáñez", "Pérez", "García", "Núñez", "Martín", "Fernández", "López", "Muñoz")
_CITIES = ("Valencia", "Sevilla", "Bilbao", "Málaga", "Girona", "León")
_SUFFIXES = ("S.L.", "S.A.", "LLC", "Inc.", "GmbH", "Ltd")
_CURRENCIES = ("EUR", "USD")
_ACCENTS = str.maketrans("áéíóúñÁÉÍÓÚÑ", "aeiounAEIOUN")
_BASE = datetime.date(2024, 1, 1)


class _Rng(random.Random):
    """Random numbers with the helpers the generator needs."""

    def below(self, n: int) -> int:
        """Return a uniform integer in 0 to n - 1."""
        return self.randrange(n)

    def between(self, low: int, high: int) -> int:
        """Return a uniform integer in low to high, both included."""
        return self.randint(low, high)

    def chance(self, p: float) -> bool:
        """Return True with probability p; never when p is 0."""
        return p > 0 and self.random() <= p


@dataclass
class _Customer:
    """A customer: names, partition and, for some, a fixed invoice amount."""

    name: str
    bank_name: str
    scope: str
    currency: str
    fixed_amount: int | None


def _strip_accents(text: str) -> str:
    return text.translate(_ACCENTS)


def _customer(rng: _Rng, index: int, scopes: int, currencies: int) -> _Customer:
    words = [rng.choice(_FIRST), rng.choice(_SURNAMES), rng.choice(_CITIES)]
    suffix = rng.choice(_SUFFIXES)
    bank = [_strip_accents(w).upper() for w in words]
    if rng.chance(0.5):
        bank = [w[:8] for w in bank]
    if rng.chance(0.5):
        bank.append(_strip_accents(suffix).upper())
    currency = rng.choice(_CURRENCIES[:currencies])
    fixed = rng.between(1_000, 5_000_000) if rng.chance(0.2) else None
    return _Customer(
        name=" ".join([*words, suffix]),
        bank_name=" ".join(bank),
        scope=f"scope-{index % scopes}",
        currency=currency,
        fixed_amount=fixed,
    )


def _render_number(compact: list[str], style: int) -> str:
    """Write the compact number with dashes (0), lower case and spaces (1) or as is (2)."""
    out = []
    for i, char in enumerate(compact):
        if style < 2 and i in (3, 7):
            out.append("-" if style == 0 else " ")
        out.append(char.lower() if style == 1 else char)
    return "".join(out)


def _typo(rng: _Rng, compact: list[str]) -> None:
    """Apply exactly one edit (change, swap or drop) to the digits of the compact number."""
    pos = 3 + rng.below(len(compact) - 3)
    kind = rng.below(3)
    if kind == 0:
        compact[pos] = "0" if compact[pos] == "9" else chr(ord(compact[pos]) + 1)
    elif kind == 1:
        swap = next((i for i in range(3, len(compact) - 1) if compact[i] != compact[i + 1]), None)
        if swap is None:
            del compact[pos]
        else:
            compact[swap], compact[swap + 1] = compact[swap + 1], compact[swap]
    else:
        del compact[pos]


def _reference(rng: _Rng, number: str, short_name: str, noise: Noise) -> str:
    if rng.chance(noise.missing_reference):
        return f"TRANSFER {rng.below(1_000_000):06d}"
    compact = [c for c in number if c != "-"]
    if rng.chance(noise.typo):
        _typo(rng, compact)
    style = rng.below(3)
    if style == 0:
        return f"Payment {_render_number(compact, 0)}"
    if style == 1:
        return f"fact {_render_number(compact, 1)} gracias"
    return f"TRANSFER/{_render_number(compact, 2)}/{short_name}"


def _shift(date: datetime.date, days: int) -> datetime.date:
    return date + datetime.timedelta(days=days)


class _Builder:
    """Accumulates payments and the solution while the invoices are drawn."""

    def __init__(self, rng: _Rng, noise: Noise) -> None:
        self.rng = rng
        self.noise = noise
        self.payments: list[Payment] = []
        self.solution: list[Group] = []

    def payment(
        self,
        customer: _Customer,
        amount: int,
        date: datetime.date,
        reference: str,
        counterparty: str,
    ) -> str:
        """Append a payment of the customer and return its id."""
        pid = f"P-{len(self.payments):07d}"
        self.payments.append(
            Payment(pid, amount, customer.currency, date, reference, counterparty, customer.scope)
        )
        return pid

    def payer(self, customer: _Customer) -> str:
        """Return the payer name as the bank reports it."""
        return customer.bank_name if self.rng.chance(self.noise.abbreviated_name) else customer.name

    def late_offset(self) -> int:
        """Return days after the due date: usually close, sometimes very late."""
        if self.rng.chance(0.05):
            return self.rng.between(0, 60)
        return self.rng.between(-10, 25)

    def pay_single(self, customer: _Customer, invoice: Invoice) -> None:
        """Pay one invoice with one payment."""
        rng = self.rng
        paid = invoice.amount_minor
        if rng.chance(self.noise.fee):
            paid -= rng.between(1, 300)
        offset = self.late_offset()
        short = customer.bank_name.split(" ")[0]
        reference = _reference(rng, invoice.number, short, self.noise)
        pid = self.payment(
            customer, paid, _shift(invoice.due_date, offset), reference, self.payer(customer)
        )
        self.solution.append(((pid,), (invoice.id,)))

    def pay_partial(self, customer: _Customer, invoice: Invoice) -> None:
        """Pay one invoice with 2 or 3 payments, each at least 10 percent of the amount."""
        rng = self.rng
        parts = 2 + rng.below(2)
        min_part = (invoice.amount_minor + 9) // 10
        remaining = invoice.amount_minor
        amounts = []
        for left in range(parts - 1, 0, -1):
            part = rng.between(min_part, remaining - min_part * left)
            amounts.append(part)
            remaining -= part
        amounts.append(remaining)
        if rng.chance(self.noise.fee):
            amounts[-1] -= rng.between(1, min(300, amounts[-1] - 1))
        short = customer.bank_name.split(" ")[0]
        ids = []
        for amount in amounts:
            offset = rng.between(-10, 25)
            reference = _reference(rng, invoice.number, short, self.noise)
            ids.append(
                self.payment(
                    customer,
                    amount,
                    _shift(invoice.due_date, offset),
                    reference,
                    self.payer(customer),
                )
            )
        self.solution.append((tuple(sorted(ids)), (invoice.id,)))

    def pay_batch(self, customer: _Customer, members: list[Invoice]) -> None:
        """Pay a batch of invoices with one payment that lists every number."""
        rng = self.rng
        total = sum(i.amount_minor for i in members)
        if rng.chance(self.noise.fee):
            total -= rng.between(1, 300)
        last_due = max(i.due_date for i in members)
        offset = self.late_offset()
        if rng.chance(self.noise.missing_reference):
            reference = f"TRANSFER {rng.below(1_000_000):06d}"
        else:
            style = rng.below(3)
            numbers = []
            for invoice in members:
                compact = [c for c in invoice.number if c != "-"]
                if rng.chance(self.noise.typo):
                    _typo(rng, compact)
                numbers.append(_render_number(compact, style))
            reference = f"Payment {' '.join(numbers)}"
        pid = self.payment(
            customer, total, _shift(last_due, offset), reference, self.payer(customer)
        )
        self.solution.append(((pid,), tuple(sorted(i.id for i in members))))

    def close_batch(self, customer: _Customer, members: list[Invoice]) -> None:
        """Close a batch: one payment for two or more members, a plain payment for one."""
        if len(members) == 1:
            self.pay_single(customer, members[0])
        else:
            self.pay_batch(customer, members)


def generate(
    n_payments: int, noise: Noise, seed: int = 1, scopes: int = 4, currencies: int = 2
) -> Dataset:
    """Generate invoices, exactly n_payments payments and the correct pairing.

    Per invoice, in order: unpaid, else paid in parts, else joined to a batch, else paid by
    one payment. Invoices are drawn until the payments they produce, plus the unrelated ones,
    reach the requested count. Customers are spread over the scopes in turn and each one has
    one currency, so rows of different scopes or currencies never belong to one group. The
    grouped probability is the chance that an invoice joins its customer's open batch; a
    batch that ends with one member is paid like any other invoice, so the share of invoices
    that end up in a multi-invoice group is lower than the probability.

    Args:
        n_payments: Number of payments to produce.
        noise: Probability of each kind of noise.
        seed: Seed of the random generator; the same seed gives the same data.
        scopes: Number of distinct scopes the customers are spread over.
        currencies: Number of distinct currencies, 1 or 2.

    Returns:
        The invoices, the payments and the solution.
    """
    rng = _Rng(seed)
    unrelated = round(n_payments * noise.unrelated / (1 + noise.unrelated))
    target = n_payments - unrelated
    customers = [_customer(rng, k, scopes, currencies) for k in range(max(1, n_payments // 10))]
    builder = _Builder(rng, noise)
    invoices: list[Invoice] = []
    batches: dict[int, tuple[list[Invoice], int]] = {}
    while (room := target - len(builder.payments) - len(batches)) > 0:
        n = len(invoices)
        ci = rng.below(len(customers))
        customer = customers[ci]
        is_unpaid = rng.chance(noise.unpaid)
        # A partial payment adds up to 3 payments: allow it only while they fit in the target.
        is_partial = not is_unpaid and rng.chance(noise.partial) and room >= 3
        is_grouped = not is_unpaid and not is_partial and rng.chance(noise.grouped)
        if is_grouped and ci in batches:
            due = _shift(batches[ci][0][0].due_date, rng.between(0, 20))
        else:
            due = _shift(_BASE, rng.between(0, 364))
        amount = customer.fixed_amount or rng.between(1_000, 5_000_000)
        invoice = Invoice(
            id=f"I-{n:07d}",
            number=f"INV-2024-{n + 1:07d}",
            amount_minor=amount,
            currency=customer.currency,
            issue_date=_shift(due, -30),
            due_date=due,
            counterparty=customer.name,
            scope=customer.scope,
        )
        invoices.append(invoice)
        if is_unpaid:
            continue
        if is_partial:
            builder.pay_partial(customer, invoice)
        elif is_grouped:
            if ci not in batches:
                batches[ci] = ([], 2 + rng.below(3))
            members, size = batches[ci]
            members.append(invoice)
            if len(members) == size:
                del batches[ci]
                builder.close_batch(customer, members)
        else:
            builder.pay_single(customer, invoice)
    for ci, (members, _) in batches.items():
        builder.close_batch(customers[ci], members)
    for _ in range(n_payments - len(builder.payments)):
        customer = customers[rng.below(len(customers))]
        builder.payment(
            customer,
            rng.between(1_000, 5_000_000),
            _shift(_BASE, rng.between(0, 364)),
            f"TRANSFER {rng.below(1_000_000):06d}",
            customer.name,
        )
    payments = builder.payments
    for i in range(len(payments) - 1, 0, -1):
        j = rng.below(i + 1)
        payments[i], payments[j] = payments[j], payments[i]
    return Dataset(invoices=invoices, payments=payments, solution=builder.solution)
