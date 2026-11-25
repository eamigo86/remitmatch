"""Tests of the benchmark data generator, the baselines and the metric computation."""

import datetime
import json
import re
from collections.abc import Callable
from pathlib import Path

import pytest

pytest.importorskip("pandas")
pytest.importorskip("rapidfuzz")
pytest.importorskip("matplotlib")

import generate  # noqa: E402
import plot  # noqa: E402
import run  # noqa: E402
from baselines import pandas_rapidfuzz, pure_python  # noqa: E402

BASELINES = [
    pytest.param(pure_python.match, id="pure-python"),
    pytest.param(pandas_rapidfuzz.match, id="pandas-rapidfuzz"),
]
Matcher = Callable[[list[generate.Invoice], list[generate.Payment], int], list[generate.Group]]


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


def _hand_made() -> tuple[list[generate.Invoice], list[generate.Payment]]:
    """Build a few rows that exercise the matching rules one by one.

    Returns:
        Four invoices and six payments with known outcomes.
    """
    due = datetime.date(2024, 3, 31)

    def invoice(k: int, amount: int, currency: str = "EUR", scope: str = "s") -> generate.Invoice:
        return generate.Invoice(
            f"i{k}", f"INV-2024-{k:07d}", amount, currency, due - datetime.timedelta(30), due,
            "Acme Trading S.L.", scope,
        )  # fmt: skip

    def payment(
        k: int, amount: int, ref: str, currency: str = "EUR", scope: str = "s", day: int = 0
    ) -> generate.Payment:
        return generate.Payment(
            f"p{k}", amount, currency, due + datetime.timedelta(day), ref, "ACME TRADING", scope
        )

    invoices = [invoice(1, 100_000), invoice(2, 200_000), invoice(3, 300_000), invoice(4, 400_000)]
    payments = [
        payment(1, 100_000, "Payment INV-2024-0000001"),  # exact
        payment(2, 199_700, "fact inv 2024 0000002 gracias"),  # short by 300, inside tolerance
        payment(3, 299_699, "Payment INV-2024-0000003"),  # short by 301, outside tolerance
        payment(4, 400_000, "Payment INV-2024-0000004", currency="USD"),  # other currency
        payment(5, 400_000, "Payment INV-2024-0000004", scope="other"),  # other scope
        payment(6, 300_000, "Payment INV-2024-000003", day=5),  # one digit dropped, same amount
    ]
    return invoices, payments


@pytest.mark.parametrize("match", BASELINES)
def test_baseline_follows_the_matching_rules(match: Matcher) -> None:
    """Tolerance, currency and scope are respected and a misspelled number still matches.

    Args:
        match: Baseline under test.
    """
    invoices, payments = _hand_made()
    found = {(tuple(p), tuple(i)) for p, i in match(invoices, payments, 300)}
    assert found == {
        (("p1",), ("i1",)),
        (("p2",), ("i2",)),
        (("p6",), ("i3",)),
    }


@pytest.mark.parametrize("match", BASELINES)
def test_baseline_matches_everything_on_noise_free_data(match: Matcher) -> None:
    """Without noise every payment is matched to its invoice, and nothing else.

    Args:
        match: Baseline under test.
    """
    data = generate.generate(2_000, generate.CLEAN, seed=1)
    accuracy = generate.evaluate(match(data.invoices, data.payments, 300), data.solution)
    assert accuracy.proposed == len(data.solution)
    assert accuracy.precision == 1.0
    assert accuracy.recall == 1.0


@pytest.mark.parametrize("match", BASELINES)
def test_baseline_is_precise_on_noisy_data(match: Matcher) -> None:
    """On realistic one-to-one noise the proposed matches are almost all right.

    Precision is held above 0.97: the only way to be wrong is a misspelled number that lands on
    another invoice of similar amount. Recall is held above 0.80: payments without an invoice
    number (5 percent) cannot be matched by reference, and the rest depend on the fuzzy pass.

    Args:
        match: Baseline under test.
    """
    data = generate.generate(5_000, generate.ONE_TO_ONE, seed=7)
    accuracy = generate.evaluate(match(data.invoices, data.payments, 300), data.solution)
    print(f"{match.__module__}: precision {accuracy.precision:.4f} recall {accuracy.recall:.4f}")
    assert accuracy.precision >= 0.97
    assert accuracy.recall >= 0.80


@pytest.mark.parametrize("match", BASELINES)
def test_baseline_uses_no_id_twice_and_is_deterministic(match: Matcher) -> None:
    """Every id appears in at most one group and a second run gives the same answer.

    Args:
        match: Baseline under test.
    """
    data = generate.generate(3_000, generate.ONE_TO_ONE, seed=8)
    first = match(data.invoices, data.payments, 300)
    payments = [pid for p, _ in first for pid in p]
    invoices = [iid for _, i in first for iid in i]
    assert len(set(payments)) == len(payments)
    assert len(set(invoices)) == len(invoices)
    assert all(len(p) == 1 and len(i) == 1 for p, i in first)
    assert match(data.invoices, data.payments, 300) == first


def test_the_two_baselines_agree() -> None:
    """Both baselines implement the same rules, so they find almost the same matches."""
    data = generate.generate(5_000, generate.ONE_TO_ONE, seed=9)
    a = {(tuple(p), tuple(i)) for p, i in pure_python.match(data.invoices, data.payments, 300)}
    b = {(tuple(p), tuple(i)) for p, i in pandas_rapidfuzz.match(data.invoices, data.payments, 300)}
    assert len(a & b) / len(a | b) >= 0.97


def test_pick_best_prefers_the_fastest_finished_run() -> None:
    """The best run is the fastest one that finished; none is picked when none finished."""
    runs = [
        {"status": "ok", "time_match": 2.0},
        {"status": "ok", "time_match": 1.5},
        {"status": "did_not_finish"},
    ]
    assert run.pick_best(runs) == runs[1]
    assert run.pick_best([{"status": "did_not_finish"}]) is None
    assert run.pick_best([]) is None


def test_peak_memory_unit_follows_the_platform() -> None:
    """The operating system reports the peak resident size in bytes or in kilobytes."""
    assert run.peak_rss_bytes(1000, "darwin") == 1000
    assert run.peak_rss_bytes(1000, "linux") == 1_024_000


def _small_run(tmp_path: Path, *extra: str) -> dict[str, object]:
    """Run the benchmark on a tiny dataset and read the results file.

    Args:
        tmp_path: Directory for the results file.
        *extra: Further command line arguments.

    Returns:
        The parsed results file.
    """
    output = tmp_path / "results.json"
    code = run.main(
        ["--sizes", "2000", "--datasets", "one-to-one", "--output", str(output), *extra]
    )
    assert code == 0
    return json.loads(output.read_text(encoding="utf-8"))  # type: ignore[no-any-return]


def test_runner_measures_every_implementation(tmp_path: Path) -> None:
    """Each implementation runs in its own process and reports time, memory and accuracy.

    Args:
        tmp_path: Directory for the results file.
    """
    data = _small_run(tmp_path, "--runs", "2", "--remitmatch-build", "test")
    meta = data["meta"]
    assert meta["sizes"] == [2000]
    assert meta["scopes"] == 4
    assert meta["seed"] == 1
    assert meta["machine"]["cpu_count"] >= 1
    assert meta["packages"]["remitmatch"]["build"] == "test"
    assert {r["implementation"] for r in data["results"]} == {
        "remitmatch",
        "pure-python",
        "pandas-rapidfuzz",
    }
    for result in data["results"]:
        assert result["status"] == "ok"
        assert len(result["runs"]) == 2
        best = result["best"]
        assert best["time_match"] > 0
        assert best["peak_rss_bytes"] >= best["rss_after_load_bytes"] > 0
        assert 0.9 <= best["precision"] <= 1.0
        assert 0.5 <= best["recall"] <= 1.0
    remitmatch = next(r for r in data["results"] if r["implementation"] == "remitmatch")
    assert 0.9 <= remitmatch["best"]["auto_precision"] <= 1.0
    assert remitmatch["best"]["time_call"] <= remitmatch["best"]["time_match"]


def test_a_run_over_the_limit_is_recorded_as_not_finished(tmp_path: Path) -> None:
    """With a zero time limit the matching call is cut off and the result says so.

    Args:
        tmp_path: Directory for the results file.
    """
    data = _small_run(tmp_path, "--implementations", "pure-python", "--timeout", "0")
    [result] = data["results"]
    assert result["status"] == "did_not_finish"
    assert result["best"] is None
    assert result["runs"][0]["status"] == "did_not_finish"


def test_plot_draws_the_chart_files(tmp_path: Path) -> None:
    """The chart is written as PNG and SVG, and a line stops where a run did not finish.

    Args:
        tmp_path: Directory for the chart files.
    """

    def entry(name: str, size: int, seconds: float | None) -> dict[str, object]:
        best = None if seconds is None else {"time_match": seconds}
        return {
            "implementation": name,
            "dataset": "one-to-one",
            "size": size,
            "status": "ok" if seconds else "did_not_finish",
            "best": best,
        }

    results = {
        "meta": {"machine": {"model": "Test CPU", "cpu_count": 4}, "runs_per_size": 3},
        "results": [
            entry("remitmatch", 10_000, 0.05),
            entry("remitmatch", 100_000, 0.7),
            entry("pure-python", 10_000, 0.03),
            entry("pure-python", 100_000, 0.5),
            entry("pandas-rapidfuzz", 10_000, 0.03),
            entry("pandas-rapidfuzz", 100_000, None),
        ],
    }
    source = tmp_path / "results.json"
    source.write_text(json.dumps(results), encoding="utf-8")
    png, svg = tmp_path / "time.png", tmp_path / "time.svg"
    plot.draw(source, png, svg)
    assert png.read_bytes().startswith(b"\x89PNG")
    assert svg.read_text(encoding="utf-8").lstrip().startswith(("<?xml", "<svg"))
