"""Run the comparison benchmark and write the results file.

Every implementation, dataset and size is measured in a fresh child process, so that its peak
memory is its own. The parent generates each dataset once, writes it to a temporary directory
as Parquet and starts the children one after another (never two timed jobs at once).

Each child loads the data into the native form of its implementation (Arrow tables for
remitmatch, lists of named tuples for pure Python, DataFrames for pandas), reports the time
that took, then times the matching call alone with the garbage collector disabled (as timeit
does) and reports the peak resident memory. Only then does it score the answer against the
known solution, so the scoring never counts in the time or in the memory.

The timed region of every implementation ends with the answer as a list of groups of ids, the
same shape for all of them. For remitmatch that includes turning the result object into
groups; the time of the reconcile call alone is recorded as well.

Usage:
    python benchmarks/run.py [--sizes N ...] [--implementations NAME ...]
        [--datasets NAME ...] [--scopes N] [--output PATH]

Only POSIX systems are supported.
"""

from __future__ import annotations

import argparse
import datetime
import gc
import importlib.metadata
import json
import os
import pickle
import platform
import resource
import select
import shutil
import subprocess
import sys
import tempfile
import time
from collections.abc import Callable, Sequence
from pathlib import Path
from typing import Any

import generate

IMPLEMENTATIONS = ("remitmatch", "pure-python", "pandas-rapidfuzz", "remitmatch-1-thread")
LAYOUTS = {
    "four-scopes": {"scopes": 4, "currencies": 2},
    "one-scope": {"scopes": 1, "currencies": 1},
}
TOLERANCE = 300
RUNS_UP_TO = 100_000
LOAD_LIMIT_SECONDS = 1800
DEFAULT_OUTPUT = Path(__file__).resolve().parent / "results" / "results.json"
_START = b"MATCH_START"
_RESULT = "RESULT "


def pick_best(runs: Sequence[dict[str, Any]]) -> dict[str, Any] | None:
    """Pick the fastest run that finished.

    Args:
        runs: The runs of one implementation, dataset and size.

    Returns:
        The finished run with the smallest matching time, or None when none finished.
    """
    finished = [r for r in runs if r.get("status") == "ok"]
    return min(finished, key=lambda r: r["time_match"]) if finished else None


def peak_rss_bytes(value: int, platform_name: str) -> int:
    """Convert the peak resident size reported by the operating system to bytes.

    Args:
        value: Value of ru_maxrss: bytes on macOS, kilobytes on Linux.
        platform_name: Value of sys.platform.

    Returns:
        The size in bytes.
    """
    return value if platform_name == "darwin" else value * 1024


def _current_rss_bytes() -> int:
    """Return the current resident size of this process in bytes."""
    out = subprocess.run(
        ["ps", "-o", "rss=", "-p", str(os.getpid())], capture_output=True, text=True, check=True
    )
    return int(out.stdout.strip()) * 1024


def _peak_rss() -> int:
    """Return the peak resident size of this process so far, in bytes."""
    return peak_rss_bytes(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss, sys.platform)


# --- child: one implementation, one dataset, one run ---------------------------------------


def _remitmatch(
    inv: object, pay: object, tolerance: int
) -> tuple[list[generate.Group], dict[str, float], object]:
    """Time reconcile on Arrow tables and the conversion of its result into groups.

    Args:
        inv: Invoices as an Arrow table.
        pay: Payments as an Arrow table.
        tolerance: Amount tolerance in minor units.

    Returns:
        The groups, the timings and the result object.
    """
    import remitmatch

    config = remitmatch.MatchConfig(amount_tolerance_minor=tolerance)
    gc.collect()
    gc.disable()
    print(_START.decode(), flush=True)
    start = time.perf_counter()
    result = remitmatch.reconcile(inv, pay, config)
    called = time.perf_counter()
    matches = result.matches
    groups = [(m.payment_ids, m.invoice_ids) for m in matches]
    done = time.perf_counter()
    return groups, {"time_match": done - start, "time_call": called - start}, result


def _timed(
    call: Callable[..., list[generate.Group]], *args: object
) -> tuple[list[generate.Group], dict[str, float], None]:
    """Time a baseline call with the garbage collector disabled.

    Args:
        call: The baseline's match function.
        *args: Its arguments.

    Returns:
        The groups, the timing and None.
    """
    gc.collect()
    gc.disable()
    print(_START.decode(), flush=True)
    start = time.perf_counter()
    groups = call(*args)
    return groups, {"time_match": time.perf_counter() - start}, None


def _os_threads() -> int | None:
    """Count the operating system threads of this process, when the platform shows them.

    Returns:
        The number of threads, or None when it cannot be read.
    """
    try:
        if sys.platform == "darwin":
            out = subprocess.run(
                ["ps", "-M", "-p", str(os.getpid())], capture_output=True, text=True, check=True
            )
            return len(out.stdout.strip().splitlines()) - 1
        return len(os.listdir(f"/proc/{os.getpid()}/task"))
    except (OSError, subprocess.CalledProcessError):
        return None


def _child(implementation: str, directory: Path, tolerance: int) -> None:
    """Measure one implementation on one dataset and print the result as one JSON line.

    Args:
        implementation: Name of the implementation.
        directory: Directory with the Parquet files and the solution.
        tolerance: Amount tolerance in minor units.
    """
    import pyarrow.parquet as pq
    from baselines import pandas_rapidfuzz, pure_python

    is_remitmatch = implementation.startswith("remitmatch")
    start = time.perf_counter()
    inv_table = pq.read_table(directory / "invoices.parquet")
    pay_table = pq.read_table(directory / "payments.parquet")
    if is_remitmatch:
        inputs: tuple[Any, Any] = (inv_table, pay_table)
    elif implementation == "pure-python":
        inputs = (
            list(
                map(
                    generate.Invoice._make,
                    zip(*(c.to_pylist() for c in inv_table.columns), strict=True),
                )
            ),
            list(
                map(
                    generate.Payment._make,
                    zip(*(c.to_pylist() for c in pay_table.columns), strict=True),
                )
            ),
        )
    else:
        inputs = (
            inv_table.to_pandas(date_as_object=False),
            pay_table.to_pandas(date_as_object=False),
        )
    del inv_table, pay_table
    build_time = time.perf_counter() - start
    gc.collect()
    out: dict[str, Any] = {
        "build_time": build_time,
        "rss_after_load_bytes": _current_rss_bytes(),
        "peak_rss_after_load_bytes": _peak_rss(),
    }

    if is_remitmatch:
        groups, timings, result = _remitmatch(*inputs, tolerance)
    else:
        module = pure_python if implementation == "pure-python" else pandas_rapidfuzz
        groups, timings, result = _timed(module.match, *inputs, tolerance)
    out.update(timings)
    out["peak_rss_bytes"] = _peak_rss()
    rayon = os.environ.get("RAYON_NUM_THREADS")
    # Threads the implementation is allowed to use: rayon's pool for remitmatch (one per
    # logical CPU unless RAYON_NUM_THREADS says otherwise), one for the baselines. The count
    # of operating system threads after the call is informational: the interpreter and the
    # libraries it loads start a few helper threads of their own.
    out["threads"] = (int(rayon) if rayon else os.cpu_count()) if is_remitmatch else 1
    out["rayon_num_threads"] = rayon
    out["os_threads_after_call"] = _os_threads()

    solution = pickle.loads((directory / "solution.pkl").read_bytes())
    accuracy = generate.evaluate(groups, solution)
    out.update(
        precision=accuracy.precision,
        recall=accuracy.recall,
        proposed=accuracy.proposed,
        correct=accuracy.correct,
        expected=accuracy.expected,
    )
    if result is not None:
        from remitmatch import MatchStatus

        auto = [
            (m.payment_ids, m.invoice_ids) for m in result.matches if m.status is MatchStatus.AUTO
        ]
        auto_accuracy = generate.evaluate(auto, solution)
        out.update(auto=auto_accuracy.proposed, auto_precision=auto_accuracy.precision)
    print(_RESULT + json.dumps(out), flush=True)


# --- parent ----------------------------------------------------------------------------------


def _write_dataset(
    directory: Path, noise: generate.Noise, size: int, seed: int, scopes: int, currencies: int
) -> tuple[int, int, int]:
    """Generate a dataset and write it, with its solution, to the directory.

    Args:
        directory: Existing directory to write into.
        noise: Noise profile of the dataset.
        size: Number of payments.
        seed: Seed of the generator.
        scopes: Number of scopes.
        currencies: Number of currencies.

    Returns:
        The number of invoices, payments and solution groups.
    """
    import pyarrow as pa
    import pyarrow.parquet as pq

    data = generate.generate(size, noise, seed=seed, scopes=scopes, currencies=currencies)
    types = {
        "amount_minor": pa.int64(),
        "issue_date": pa.date32(),
        "due_date": pa.date32(),
        "value_date": pa.date32(),
    }
    for name, rows, fields in (
        ("invoices", data.invoices, generate.Invoice._fields),
        ("payments", data.payments, generate.Payment._fields),
    ):
        columns = list(zip(*rows, strict=True))
        table = pa.table(
            {
                f: pa.array(c, types.get(f, pa.string()))
                for f, c in zip(fields, columns, strict=True)
            }
        )
        pq.write_table(table, directory / f"{name}.parquet")
    (directory / "solution.pkl").write_bytes(pickle.dumps(data.solution))
    return len(data.invoices), len(data.payments), len(data.solution)


def _run_child(
    implementation: str, directory: Path, timeout: float, load_limit: float
) -> dict[str, Any]:
    """Run one child process and collect its result.

    The matching call has timeout seconds, counted from the moment the child announces it is
    starting the call; loading the data has its own, much longer limit.

    Args:
        implementation: Name of the implementation.
        directory: Directory with the dataset.
        timeout: Seconds allowed for the matching call.
        load_limit: Seconds allowed for everything before the matching call.

    Returns:
        The child's measurements plus a status: ok, did_not_finish or error.
    """
    load_average = os.getloadavg()[0]
    environment = dict(os.environ)
    if implementation == "remitmatch-1-thread":
        environment["RAYON_NUM_THREADS"] = "1"
    with tempfile.TemporaryFile() as errors:
        child = subprocess.Popen(
            [
                sys.executable,
                str(Path(__file__).resolve()),
                "child",
                implementation,
                str(directory),
            ],
            stdout=subprocess.PIPE,
            env=environment,
            stderr=errors,
        )
        assert child.stdout is not None
        fd = child.stdout.fileno()
        deadline = time.monotonic() + load_limit
        started = False
        stdout = b""
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                child.kill()
                child.wait()
                status = "did_not_finish" if started else "error"
                return {"status": status, "load_average_1m": load_average}
            if not select.select([fd], [], [], min(remaining, 1.0))[0]:
                continue
            chunk = os.read(fd, 65536)
            if not chunk:
                break
            stdout += chunk
            if not started and _START in stdout:
                started = True
                deadline = time.monotonic() + timeout
        child.wait()
        if child.returncode != 0:
            errors.seek(0)
            tail = errors.read().decode(errors="replace")[-2000:]
            return {"status": "error", "error": tail, "load_average_1m": load_average}
    for line in stdout.decode().splitlines():
        if line.startswith(_RESULT):
            run = json.loads(line[len(_RESULT) :])
            return {"status": "ok", "load_average_1m": load_average, **run}
    return {"status": "error", "error": "no result line", "load_average_1m": load_average}


def _machine() -> dict[str, Any]:
    """Describe the machine: CPU model, logical CPUs and memory.

    Returns:
        A dictionary with model, cpu_count and memory_bytes (None when unknown).
    """

    def sysctl(name: str) -> str | None:
        try:
            return subprocess.run(
                ["sysctl", "-n", name], capture_output=True, text=True, check=True
            ).stdout.strip()
        except (OSError, subprocess.CalledProcessError):
            return None

    model, memory = platform.processor() or None, None
    if sys.platform == "darwin":
        model = sysctl("machdep.cpu.brand_string") or model
        size = sysctl("hw.memsize")
        memory = int(size) if size else None
    elif Path("/proc/cpuinfo").exists():
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                model = line.split(":", 1)[1].strip()
                break
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemTotal"):
                memory = int(line.split()[1]) * 1024
                break
    return {"model": model, "cpu_count": os.cpu_count(), "memory_bytes": memory}


def _packages(build: str, commit: str) -> dict[str, Any]:
    """Collect the versions of the packages that take part in the measurement.

    Args:
        build: Build profile of the installed remitmatch, as declared on the command line.
        commit: Git commit the installed remitmatch was built from, as declared.

    Returns:
        Versions by package name; remitmatch also carries its core version and build.
    """
    import remitmatch

    versions: dict[str, Any] = {
        name: importlib.metadata.version(name)
        for name in ("pandas", "numpy", "pyarrow", "rapidfuzz", "matplotlib")
    }
    versions["remitmatch"] = {
        "version": remitmatch.__version__,
        "core_version": remitmatch.version(),
        "build": build,
        "commit": commit,
    }
    return versions


def main(argv: Sequence[str] | None = None) -> int:
    """Run the benchmark.

    Args:
        argv: Command line arguments without the program name; defaults to sys.argv.

    Returns:
        The process exit code.
    """
    args = list(sys.argv[1:] if argv is None else argv)
    if args and args[0] == "child":
        _child(args[1], Path(args[2]), TOLERANCE)
        return 0
    parser = argparse.ArgumentParser(description="Compare remitmatch with two Python baselines.")
    parser.add_argument("--sizes", type=int, nargs="+", default=[10_000, 100_000, 1_000_000])
    parser.add_argument(
        "--implementations", nargs="+", choices=IMPLEMENTATIONS, default=IMPLEMENTATIONS
    )
    parser.add_argument(
        "--datasets", nargs="+", choices=list(generate.DATASETS), default=list(generate.DATASETS)
    )
    parser.add_argument("--scopes", type=int, default=4, help="scopes of the four-scopes layout")
    parser.add_argument("--layouts", nargs="+", choices=list(LAYOUTS), default=list(LAYOUTS))
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--runs", type=int, default=3, help="runs per size up to 100,000 payments")
    parser.add_argument("--timeout", type=float, default=600, help="seconds for the matching call")
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--remitmatch-build", default="unknown", help="build profile to record")
    parser.add_argument("--commit", default="unknown", help="git commit of the build to record")
    parser.add_argument("--note", default="", help="free text recorded in the results file")
    options = parser.parse_args(args)

    layouts = {name: dict(LAYOUTS[name]) for name in options.layouts}
    if "four-scopes" in layouts:
        layouts["four-scopes"]["scopes"] = options.scopes
    meta: dict[str, Any] = {
        "date": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
        "machine": _machine(),
        "os": platform.platform(),
        "python": platform.python_version(),
        "packages": _packages(options.remitmatch_build, options.commit),
        "rayon_num_threads": os.environ.get("RAYON_NUM_THREADS"),
        "seed": options.seed,
        "scopes": options.scopes,
        "layouts": layouts,
        "tolerance_minor": TOLERANCE,
        "sizes": sorted(options.sizes),
        "datasets": options.datasets,
        "implementations": list(options.implementations),
        "runs_per_size": options.runs,
        "runs_up_to_size": RUNS_UP_TO,
        "timeout_seconds": options.timeout,
        "garbage_collector": "disabled during the timed call, in every implementation",
        "timed_region": "matching call plus conversion of the answer to groups of ids",
        "note": options.note,
        "load_average_start": os.getloadavg(),
    }
    results: list[dict[str, Any]] = []
    options.output.parent.mkdir(parents=True, exist_ok=True)

    def save() -> None:
        options.output.write_text(
            json.dumps({"meta": meta, "results": results}, indent=2) + "\n", encoding="utf-8"
        )

    unfinished: set[tuple[str, str, str]] = set()
    work = Path(tempfile.mkdtemp(prefix="remitmatch-bench-"))
    try:
        for layout, shape in layouts.items():
            for dataset in options.datasets:
                for size in meta["sizes"]:
                    directory = work / f"{layout}-{dataset}-{size}"
                    directory.mkdir()
                    print(f"generating {layout} {dataset} {size}", file=sys.stderr, flush=True)
                    n_invoices, n_payments, n_groups = _write_dataset(
                        directory,
                        generate.DATASETS[dataset],
                        size,
                        options.seed,
                        shape["scopes"],
                        shape["currencies"],
                    )
                    for implementation in options.implementations:
                        entry: dict[str, Any] = {
                            "implementation": implementation,
                            "layout": layout,
                            "dataset": dataset,
                            "size": size,
                            "invoices": n_invoices,
                            "payments": n_payments,
                            "solution_groups": n_groups,
                            "runs": [],
                        }
                        if (layout, implementation, dataset) in unfinished:
                            entry.update(status="not_run", best=None)
                        else:
                            n_runs = options.runs if size <= RUNS_UP_TO else 1
                            for _ in range(n_runs):
                                print(
                                    f"{layout} {implementation} {dataset} {size}",
                                    file=sys.stderr,
                                    flush=True,
                                )
                                run = _run_child(
                                    implementation, directory, options.timeout, LOAD_LIMIT_SECONDS
                                )
                                entry["runs"].append(run)
                                if run["status"] != "ok":
                                    break
                            best = pick_best(entry["runs"])
                            entry["best"] = best
                            if best is not None:
                                entry["status"] = "ok"
                            else:
                                entry["status"] = entry["runs"][-1]["status"]
                                unfinished.add((layout, implementation, dataset))
                        results.append(entry)
                        save()
                    shutil.rmtree(directory)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    meta["load_average_end"] = os.getloadavg()
    save()
    return 0


if __name__ == "__main__":
    sys.exit(main())
