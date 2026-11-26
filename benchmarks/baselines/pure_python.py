"""Matching of payments to invoices with the standard library only.

This baseline is written for the benchmark and kept as fast as a plain Python formulation
reasonably gets. It does the same thing as the pandas and rapidfuzz baseline:

1. Exact pass. An invoice number found in the payment reference (after removing spaces,
   dashes and slashes) is looked up in a dictionary; the pair is kept when scope and currency
   agree and the amounts differ by at most the tolerance.
2. Fuzzy pass. For payments that mention an invoice-like number but found no exact match,
   candidates come from an amount index (bisect) inside the date window. A candidate is scored
   with the four signals of remitmatch (reference, amount, name, date, same default weights)
   using difflib, and kept at the default suggest threshold of 0.60.
3. Greedy assignment: best score first, an id is used at most once.

What it does not do: it matches one payment to one invoice only (no grouped or partial
payments), it never flags an ambiguous match, it gives no per-signal explanation, and it
assumes the invoice numbers start with INV (remitmatch does not need to know the prefix).
"""

from __future__ import annotations

import re
import unicodedata
from bisect import bisect_left, bisect_right
from collections import defaultdict
from collections.abc import Sequence
from difflib import SequenceMatcher

from generate import Group, Invoice, Payment

_DROP = str.maketrans("", "", " -/")
INVOICE_PREFIX = "INV"
"""Prefix of the invoice numbers this baseline looks for in the references.

The choice favours the baseline: knowing the prefix turns the exact pass into one regular
expression. Invoice numbers with any other prefix are never found, so on such data the
baseline matches nothing (remitmatch does not need to know the prefix). A prefix-agnostic
pattern costs time.
"""
_TOKEN = re.compile(rf"{INVOICE_PREFIX}\d+")
_SUFFIXES = frozenset({"SL", "SA", "LLC", "INC", "GMBH", "LTD"})
_WINDOW_DAYS = 30
_FULL_DATE_SCORE_DAYS = 3
_THRESHOLD = 0.60
_REFERENCE_FLOOR = 0.8
_W_REFERENCE, _W_AMOUNT, _W_NAME, _W_DATE = 0.45, 0.30, 0.15, 0.10


def _name_key(name: str) -> str:
    """Normalize a name: no accents, upper case, no dots and no legal suffix."""
    text = unicodedata.normalize("NFKD", name)
    words = (
        "".join(c for c in text if not unicodedata.combining(c)).upper().replace(".", "").split()
    )
    return " ".join(w for w in words if w not in _SUFFIXES)


def _greedy(
    candidates: list[tuple[float, int, int]], taken_p: set[int], taken_i: set[int]
) -> list[tuple[int, int]]:
    """Accept candidates best first when neither side is already used.

    Args:
        candidates: Tuples of rank (lower is better), payment position, invoice position.
        taken_p: Payment positions already matched; updated in place.
        taken_i: Invoice positions already matched; updated in place.

    Returns:
        The accepted pairs of payment position and invoice position.
    """
    accepted = []
    for _, p, i in sorted(candidates):
        if p not in taken_p and i not in taken_i:
            taken_p.add(p)
            taken_i.add(i)
            accepted.append((p, i))
    return accepted


def match(
    invoices: Sequence[Invoice], payments: Sequence[Payment], tolerance: int = 300
) -> list[Group]:
    """Match payments to invoices one to one.

    Args:
        invoices: Open invoices.
        payments: Payments received.
        tolerance: Largest accepted difference between payment and invoice, in minor units.

    Returns:
        One group of a single payment id and a single invoice id per match.
    """
    by_number = {inv.number.translate(_DROP).upper(): k for k, inv in enumerate(invoices)}
    taken_p: set[int] = set()
    taken_i: set[int] = set()
    tokens: dict[int, list[str]] = {}

    exact = []
    for pi, pay in enumerate(payments):
        found = _TOKEN.findall(pay.reference.translate(_DROP).upper())
        if not found:
            continue
        tokens[pi] = found
        for token in set(found):
            ii = by_number.get(token)
            if ii is None:
                continue
            inv = invoices[ii]
            diff = abs(pay.amount_minor - inv.amount_minor)
            if diff <= tolerance and inv.scope == pay.scope and inv.currency == pay.currency:
                exact.append((float(diff), pi, ii))
    pairs = _greedy(exact, taken_p, taken_i)

    index: dict[tuple[str, str], list[tuple[int, int]]] = defaultdict(list)
    for ii, inv in enumerate(invoices):
        if ii not in taken_i:
            index[inv.scope, inv.currency].append((inv.amount_minor, ii))
    amounts = {}
    for key, rows in index.items():
        rows.sort()
        amounts[key] = [a for a, _ in rows]
    due = [inv.due_date.toordinal() for inv in invoices]
    numbers = [inv.number.translate(_DROP).upper() for inv in invoices]
    names: dict[str, str] = {}
    fuzzy = []
    for pi, found in tokens.items():
        if pi in taken_p:
            continue
        pay = payments[pi]
        key = (pay.scope, pay.currency)
        if key not in index:
            continue
        rows = index[key]
        lo = bisect_left(amounts[key], pay.amount_minor - tolerance)
        hi = bisect_right(amounts[key], pay.amount_minor + tolerance)
        value = pay.value_date.toordinal()
        # SequenceMatcher indexes its second sequence, so that one is fixed per payment.
        token_matchers = []
        for token in set(found):
            token_matcher = SequenceMatcher(autojunk=False)
            token_matcher.set_seq2(token)
            token_matchers.append(token_matcher)
        name_matcher = SequenceMatcher(autojunk=False)
        name_matcher.set_seq2(names.setdefault(pay.counterparty, _name_key(pay.counterparty)))
        for _, ii in rows[lo:hi]:
            days = abs(value - due[ii])
            if days > _WINDOW_DAYS:
                continue
            diff = abs(pay.amount_minor - invoices[ii].amount_minor)
            amount = 1 - diff / tolerance if tolerance else 1.0
            date = (
                1.0
                if days <= _FULL_DATE_SCORE_DAYS
                else 1 - (days - _FULL_DATE_SCORE_DAYS) / (_WINDOW_DAYS - _FULL_DATE_SCORE_DAYS)
            )
            reference = 0.0
            for token_matcher in token_matchers:
                token_matcher.set_seq1(numbers[ii])
                reference = max(reference, token_matcher.ratio())
            if reference < _REFERENCE_FLOOR:
                continue
            inv_name = invoices[ii].counterparty
            name_matcher.set_seq1(names.setdefault(inv_name, _name_key(inv_name)))
            score = (
                _W_REFERENCE * reference
                + _W_AMOUNT * amount
                + _W_NAME * name_matcher.ratio()
                + _W_DATE * date
            )
            if score >= _THRESHOLD:
                fuzzy.append((-score, pi, ii))
    pairs += _greedy(fuzzy, taken_p, taken_i)
    return [((payments[p].id,), (invoices[i].id,)) for p, i in pairs]
