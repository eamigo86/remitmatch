"""Matching of payments to invoices with pandas, numpy and rapidfuzz.

This baseline is written for the benchmark and vectorized as far as the task allows. It does
the same thing as the pure Python baseline:

1. Exact pass. Invoice-like numbers are extracted from the references with one regular
   expression, exploded to one row per payment and number, and merged with the invoices on
   the number. Pairs of the same scope and currency within the tolerance are kept.
2. Fuzzy pass. For the remaining payments that mention an invoice-like number, an amount
   window join (sorted keys and searchsorted, no row-by-row loop) produces the candidates;
   they are cut by the date window, and rapidfuzz scores the reference and the payer name.
   The score uses the four signals of remitmatch with the same default weights and threshold.
3. Greedy assignment, best score first, vectorized by repeatedly accepting the pairs that are
   the best choice of both their payment and their invoice.

What it does not do: it matches one payment to one invoice only (no grouped or partial
payments), it never flags an ambiguous match, it gives no per-signal explanation, and it
assumes the invoice numbers start with INV (remitmatch does not need to know the prefix).
"""

from __future__ import annotations

import re
from collections.abc import Sequence

import numpy as np
import pandas as pd
import pyarrow as pa
import pyarrow.compute as pc
from generate import Group, Invoice, Payment
from rapidfuzz import fuzz, process

INVOICE_PREFIX = "INV"
"""Prefix of the invoice numbers this baseline looks for in the references.

The choice favours the baseline: knowing the prefix turns the exact pass into one regular
expression. Invoice numbers with any other prefix are never found, so on such data the
baseline matches nothing (remitmatch does not need to know the prefix). A prefix-agnostic
pattern costs time.
"""
_TOKEN = rf"{INVOICE_PREFIX}\d+"
_WINDOW_DAYS = 30
_FULL_DATE_SCORE_DAYS = 3
_THRESHOLD = 0.60
_REFERENCE_FLOOR = 0.8
_W_REFERENCE, _W_AMOUNT, _W_NAME, _W_DATE = 0.45, 0.30, 0.15, 0.10
_SHIFT = 1 << 32
_SUFFIXES = r"\b(?:SL|SA|LLC|INC|GMBH|LTD)\b"


def _frame(rows: Sequence[Invoice] | Sequence[Payment] | pd.DataFrame) -> pd.DataFrame:
    """Return the rows as a DataFrame with a range index; the input is never modified.

    Args:
        rows: Rows as named tuples, or an existing DataFrame.

    Returns:
        A DataFrame with the rows and a range index.
    """
    frame = rows if isinstance(rows, pd.DataFrame) else pd.DataFrame(list(rows))
    return frame.reset_index(drop=True)


def _day_numbers(dates: pd.Series) -> np.ndarray:
    """Convert a date column to day numbers.

    Args:
        dates: Dates as Python dates or datetimes.

    Returns:
        Whole days since the epoch as int64.
    """
    return pd.to_datetime(dates).to_numpy().astype("datetime64[D]").astype(np.int64)


def _normalize(column: pd.Series) -> pa.Array:
    """Upper-case a text column and remove spaces, dashes and slashes, with pyarrow compute.

    Args:
        column: Text column.

    Returns:
        The normalized strings.
    """
    return pc.replace_substring_regex(pc.utf8_upper(pa.array(column)), r"[\s/-]", "")


def _tokens(references: pa.Array) -> tuple[np.ndarray, np.ndarray]:
    """Find the invoice-like numbers in normalized references.

    Most references name one number, which pyarrow extracts in one pass; the few that name
    several are split with the re module.

    Args:
        references: Normalized references.

    Returns:
        Payment positions and the number found there, one entry per payment and number.
    """
    counts = pc.count_substring_regex(references, _TOKEN).to_numpy(zero_copy_only=False)
    one = np.flatnonzero(counts == 1)
    first = pc.extract_regex(references, rf"(?P<t>{_TOKEN})").field("t")
    positions = [one]
    found = [first.take(pa.array(one)).to_numpy(zero_copy_only=False).astype(object)]
    many = np.flatnonzero(counts > 1)
    if len(many):
        extra = [
            (p, t)
            for p, text in zip(
                many.tolist(), references.take(pa.array(many)).to_pylist(), strict=True
            )
            for t in re.findall(_TOKEN, text)
        ]
        positions.append(np.array([p for p, _ in extra], dtype=np.int64))
        found.append(np.array([t for _, t in extra], dtype=object))
    return np.concatenate(positions).astype(np.int64), np.concatenate(found)


def _greedy(p: np.ndarray, i: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """Accept pairs best first when neither side is already used.

    Args:
        p: Payment positions of the candidates, sorted best candidate first.
        i: Invoice positions of the candidates, in the same order.

    Returns:
        The accepted payment positions and invoice positions.
    """
    out_p: list[np.ndarray] = []
    out_i: list[np.ndarray] = []
    while len(p):
        first_p = np.unique(p, return_index=True)[1]
        first_i = np.unique(i, return_index=True)[1]
        both = np.intersect1d(first_p, first_i)
        out_p.append(p[both])
        out_i.append(i[both])
        keep = ~np.isin(p, p[both]) & ~np.isin(i, i[both])
        p, i = p[keep], i[keep]
    if not out_p:
        return np.empty(0, dtype=np.int64), np.empty(0, dtype=np.int64)
    return np.concatenate(out_p), np.concatenate(out_i)


def _name_keys(names: pd.Series) -> pd.Series:
    """Normalize names: no accents, upper case, no dots and no legal suffix.

    Args:
        names: Raw names.

    Returns:
        The normalized names.
    """
    text = names.astype("string").str.normalize("NFKD").str.replace(r"[̀-ͯ.]", "", regex=True)
    text = text.str.upper().str.replace(_SUFFIXES, "", regex=True)
    return text.str.split().str.join(" ")


def match(
    invoices: Sequence[Invoice] | pd.DataFrame,
    payments: Sequence[Payment] | pd.DataFrame,
    tolerance: int = 300,
) -> list[Group]:
    """Match payments to invoices one to one.

    Args:
        invoices: Open invoices as named tuples or as a DataFrame with the same columns.
        payments: Payments received, in the same two forms.
        tolerance: Largest accepted difference between payment and invoice, in minor units.

    Returns:
        One group of a single payment id and a single invoice id per match.
    """
    inv = _frame(invoices)
    pay = _frame(payments)
    inv_amount = inv["amount_minor"].to_numpy(dtype=np.int64)
    pay_amount = pay["amount_minor"].to_numpy(dtype=np.int64)
    scopes = pd.factorize(pd.concat([inv["scope"], pay["scope"]]))[0].astype(np.int64)
    currencies = pd.factorize(pd.concat([inv["currency"], pay["currency"]]))[0].astype(np.int64)
    codes = scopes * (int(currencies.max()) + 1) + currencies
    inv_key, pay_key = codes[: len(inv)], codes[len(inv) :]

    numbers = pd.Series(_normalize(inv["number"]), dtype=object)
    found_p, found_token = _tokens(_normalize(pay["reference"]))
    found = pd.DataFrame({"p": found_p, "token": found_token}).drop_duplicates()
    cand = found.merge(
        pd.DataFrame({"token": numbers.to_numpy(), "i": np.arange(len(inv), dtype=np.int64)}),
        on="token",
    )
    p_arr, i_arr = cand["p"].to_numpy(), cand["i"].to_numpy()
    diff = np.abs(pay_amount[p_arr] - inv_amount[i_arr])
    ok = (diff <= tolerance) & (pay_key[p_arr] == inv_key[i_arr])
    p_arr, i_arr, diff = p_arr[ok], i_arr[ok], diff[ok]
    order = np.lexsort((i_arr, p_arr, diff))
    exact_p, exact_i = _greedy(p_arr[order], i_arr[order])

    tokens = found
    tokens = tokens[~tokens["p"].isin(exact_p)]
    free = np.ones(len(inv), dtype=bool)
    free[exact_i] = False
    free_i = np.flatnonzero(free)
    sort_key = inv_key[free_i] * _SHIFT + inv_amount[free_i]
    by_key = np.argsort(sort_key, kind="stable")
    free_i, sort_key = free_i[by_key], sort_key[by_key]
    pk = pay_key[tokens["p"].to_numpy()] * _SHIFT
    amount = pay_amount[tokens["p"].to_numpy()]
    lo = np.searchsorted(sort_key, pk + np.maximum(amount - tolerance, 0), side="left")
    hi = np.searchsorted(sort_key, pk + amount + tolerance, side="right")
    counts = hi - lo
    row = np.repeat(np.arange(len(tokens)), counts)
    offset = np.arange(counts.sum()) - np.repeat(np.cumsum(counts) - counts, counts)
    fi = free_i[np.repeat(lo, counts) + offset]
    fp = tokens["p"].to_numpy()[row]
    token = tokens["token"].to_numpy()[row]

    days = np.abs(_day_numbers(pay["value_date"].iloc[fp]) - _day_numbers(inv["due_date"].iloc[fi]))
    in_window = days <= _WINDOW_DAYS
    fp, fi, token, days = fp[in_window], fi[in_window], token[in_window], days[in_window]
    cdiff = np.abs(pay_amount[fp] - inv_amount[fi])
    amount_signal = 1 - cdiff / tolerance if tolerance else np.ones(len(fp))
    date_signal = np.where(
        days <= _FULL_DATE_SCORE_DAYS,
        1.0,
        1 - (days - _FULL_DATE_SCORE_DAYS) / (_WINDOW_DAYS - _FULL_DATE_SCORE_DAYS),
    )
    reference = process.cpdist(token, numbers.to_numpy()[fi], scorer=fuzz.ratio) / 100
    reference = np.where(reference >= _REFERENCE_FLOOR, reference, 0.0)
    keep = reference > 0
    fp, fi, reference = fp[keep], fi[keep], reference[keep]
    amount_signal, date_signal = amount_signal[keep], date_signal[keep]
    pay_names = _name_keys(pay["counterparty"].iloc[fp]).to_numpy()
    inv_names = _name_keys(inv["counterparty"].iloc[fi]).to_numpy()
    name_signal = process.cpdist(pay_names, inv_names, scorer=fuzz.ratio) / 100
    score = (
        _W_REFERENCE * reference
        + _W_AMOUNT * amount_signal
        + _W_NAME * name_signal
        + _W_DATE * date_signal
    )
    keep = score >= _THRESHOLD
    fp, fi, score = fp[keep], fi[keep], score[keep]
    order = np.lexsort((fi, fp, -score))
    fuzzy_p, fuzzy_i = _greedy(fp[order], fi[order])

    pay_ids = pay["id"].to_numpy()
    inv_ids = inv["id"].to_numpy()
    matched_p = np.concatenate([exact_p, fuzzy_p])
    matched_i = np.concatenate([exact_i, fuzzy_i])
    return [
        ((a,), (b,))
        for a, b in zip(pay_ids[matched_p].tolist(), inv_ids[matched_i].tolist(), strict=True)
    ]
