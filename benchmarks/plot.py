"""Draw the run-time chart of the benchmark from the results file.

One line per implementation, run time in seconds against the number of payments, both axes
logarithmic, for the one-to-one dataset. Usage: python benchmarks/plot.py [results.json]
"""

from __future__ import annotations

import json
import math
import sys
from collections.abc import Sequence
from pathlib import Path
from typing import Any

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
from matplotlib.ticker import FuncFormatter, NullFormatter  # noqa: E402

RESULTS = Path(__file__).resolve().parent / "results"
DATASET = "one-to-one"
SURFACE = "#fcfcfb"
INK = "#1f1f1e"
MUTED = "#5f5f5c"
GRID = "#e8e8e5"
# Fixed order, one color per identity, never cycled.
SERIES = [
    ("remitmatch", "remitmatch", "#2a78d6"),
    ("pure-python", "pure Python", "#eb6834"),
    ("pandas-rapidfuzz", "pandas + rapidfuzz", "#1baf7a"),
    ("fourth", "fourth", "#eda100"),
]


def _seconds(value: float, _: object) -> str:
    """Format a tick of the time axis.

    Args:
        value: Tick value in seconds.
        _: Tick position, unused.

    Returns:
        The tick label.
    """
    return f"{value:g}"


def _count(value: float, _: object) -> str:
    """Format a tick of the size axis.

    Args:
        value: Tick value, a number of payments.
        _: Tick position, unused.

    Returns:
        The tick label with thousands separators.
    """
    return f"{value:,.0f}"


def _spread(positions: list[float], gap: float) -> list[float]:
    """Move label positions apart, in log space, until neighbours are at least gap apart.

    Args:
        positions: Wanted positions, as base-10 logarithms.
        gap: Smallest distance between two labels.

    Returns:
        The adjusted positions in the original order.
    """
    order = sorted(range(len(positions)), key=positions.__getitem__)
    placed = list(positions)
    for previous, current in zip(order, order[1:], strict=False):
        placed[current] = max(placed[current], placed[previous] + gap)
    shift = (sum(positions) - sum(placed)) / len(placed)
    return [p + shift for p in placed]


def draw(source: Path, png: Path, svg: Path) -> None:
    """Read a results file and write the chart as PNG (2x resolution) and SVG.

    Args:
        source: Path of the results JSON file.
        png: Path of the PNG file to write.
        svg: Path of the SVG file to write.
    """
    data: dict[str, Any] = json.loads(source.read_text(encoding="utf-8"))
    meta = data["meta"]
    plt.rcParams.update({"font.family": "sans-serif", "font.size": 10})
    fig, ax = plt.subplots(figsize=(8.0, 4.8), dpi=100, facecolor=SURFACE)
    ax.set_facecolor(SURFACE)
    ax.set_xscale("log")
    ax.set_yscale("log")

    labels: list[tuple[float, float, str]] = []
    every: list[float] = []
    unfinished: list[str] = []
    for key, label, color in SERIES:
        rows = sorted(
            (r for r in data["results"] if r["implementation"] == key and r["dataset"] == DATASET),
            key=lambda r: r["size"],
        )
        finished = [r for r in rows if r["status"] == "ok"]
        if any(r["status"] == "did_not_finish" for r in rows) and finished:
            first = min(r["size"] for r in rows if r["status"] != "ok")
            unfinished.append(f"{label} did not finish {first:,} payments")
        if not finished:
            continue
        xs = [r["size"] for r in finished]
        ys = [r["best"]["time_match"] for r in finished]
        ax.plot(xs, ys, color=color, linewidth=1.5, marker="o", markersize=6, label=label, zorder=3)
        labels.append((xs[-1], ys[-1], label))
        every += ys

    ax.grid(True, which="major", color=GRID, linewidth=0.6, zorder=0)
    ax.set_axisbelow(True)
    for spine in ("top", "right"):
        ax.spines[spine].set_visible(False)
    for spine in ("left", "bottom"):
        ax.spines[spine].set_color(GRID)
    ax.tick_params(colors=MUTED, which="both", labelsize=9)
    ax.xaxis.set_major_formatter(FuncFormatter(_count))
    ax.yaxis.set_major_formatter(FuncFormatter(_seconds))
    ax.xaxis.set_minor_formatter(NullFormatter())
    ax.yaxis.set_minor_formatter(NullFormatter())
    ax.set_xlabel("Number of payments", color=INK)
    ax.set_ylabel("Run time (seconds, lower is better)", color=INK)
    ax.set_title(
        f"Time to match payments to invoices ({DATASET} dataset)", color=INK, loc="left", pad=12
    )

    sizes = sorted({r["size"] for r in data["results"] if r["dataset"] == DATASET})
    ax.set_xticks(sizes)
    ax.set_xlim(min(sizes) * 0.8, max(sizes) * 5)
    if labels:
        low, high = min(every), max(every)
        decades = range(math.floor(math.log10(low)) - 1, math.ceil(math.log10(high)) + 2)
        ticks = [m * 10.0**d for d in decades for m in (1, 2, 5)]
        ax.set_yticks([t for t in ticks if low / 2.5 <= t <= high * 2.5])
        ys = (
            [math.log10(y) for _, _, _ in labels for y in [_[1]]]
            if False
            else [math.log10(t) for _, t, _ in labels]
        )
        for (x, _, text), y in zip(labels, _spread(ys, 0.13), strict=True):
            ax.annotate(
                text, (x, 10**y), xytext=(10, 0), textcoords="offset points",
                va="center", color=INK, fontsize=9,
            )  # fmt: skip
    legend = ax.legend(loc="upper left", frameon=False, labelcolor=INK, fontsize=9)
    legend.set_zorder(4)

    machine = meta.get("machine", {})
    caption = (
        f"{machine.get('model', 'unknown CPU')}, {machine.get('cpu_count', '?')} threads. "
        f"Best of {meta.get('runs_per_size', 3)} runs up to 100,000 payments, one run at 1,000,000."
    )
    if unfinished:
        caption += " " + "; ".join(unfinished) + "."
    fig.text(0.01, 0.01, caption, color=MUTED, fontsize=8, ha="left", va="bottom")
    fig.tight_layout(rect=(0, 0.04, 1, 1))
    fig.savefig(png, dpi=200, facecolor=SURFACE)
    fig.savefig(svg, facecolor=SURFACE)
    plt.close(fig)


def main(argv: Sequence[str] | None = None) -> int:
    """Draw the chart next to the results file.

    Args:
        argv: Optional path of the results file; defaults to results/results.json.

    Returns:
        The process exit code.
    """
    args = list(sys.argv[1:] if argv is None else argv)
    source = Path(args[0]) if args else RESULTS / "results.json"
    draw(source, source.with_name("time.png"), source.with_name("time.svg"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
