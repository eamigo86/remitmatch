# Benchmarks

A reproducible comparison of `remitmatch` with two other ways of doing the same job in
Python, on the same synthetic data. Every number below comes from
[`results/results.json`](results/results.json), written by [`run.py`](run.py).

## Summary

- On one-to-one data `remitmatch` is faster than both baselines at every size on this
  machine: 2.04, 1.96 and 2.06 times faster than pandas + rapidfuzz at 10,000, 100,000 and
  1,000,000 payments, and 1.83, 2.74 and 3.07 times faster than pure Python. The RFC target of
  at least 20 times over pandas + rapidfuzz is **still not met**: the measured ratio is about 2.
- Its peak memory is higher than that of pandas + rapidfuzz at every size (2.5 GB against
  1.6 GB at 1,000,000 payments) and higher than that of pure Python at 10,000 and 100,000
  payments, but lower than that of pure Python at 1,000,000 (2.5 GB against 2.7 GB).
- The build measured is `main` at commit `7ccdca0`, after the published 0.1.0a2 (the package
  version string is still 0.1.0a2). It includes performance work in the core. The previous
  measurement, on the published 0.1.0a2, was 0.035 s, 0.339 s and 3.731 s for `remitmatch` on
  one-to-one data at 10,000, 100,000 and 1,000,000 payments, which was slower than both
  baselines at 10,000 and 100,000 and slower than pandas + rapidfuzz at 1,000,000. It is kept
  here only as history; it is not a current number and is no longer in the results file.
- Precision is 99.9 % or better for every implementation. On the `mixed` dataset, which
  contains payments that settle several invoices and invoices paid in parts, `remitmatch`
  finds 93.9 to 94.9 % of the true groups and the baselines 88.1 to 88.5 %, because they
  match one payment to one invoice only.
- reconPy was not benchmarked: it cannot express the task (see below).

## Method

**Data.** [`generate.py`](generate.py) is a deterministic generator (`random.Random(seed)`)
with a known solution. It mirrors the noise model of the Rust generator used by the accuracy
tests (`crates/remitmatch/tests/common/mod.rs`): 15 % of payments are short by a bank fee of 1
to 300 minor units, 5 % of references have one typo in the invoice number, 5 % carry no
invoice number, 30 % of payer names are abbreviated bank-style variants, 10 % of invoices are
unpaid, and unrelated payments add about 5 % of the others. Two datasets:

- `one-to-one`: the noise above, no grouped or partial payments. Every implementation can in
  principle find every match.
- `mixed`: the same noise plus grouped payments (one payment for 2 to 4 invoices) and partial
  payments (one invoice paid by 2 or 3 payments). A batch that ends with one member is a plain
  payment, so only a few percent of the invoices end up in a multi-invoice group.

Sizes are exactly 10,000, 100,000 and 1,000,000 payments (the number of invoices is in the
results file). Customers are spread over four scopes and each has one of two currencies, so
the data forms eight partitions, and rows of different partitions never belong to one group.
The runs use seed 1. `--scopes` changes the number of scopes; the single-scope case is not
measured here.

**What is compared.** All implementations get the same rows and the same tolerance (300 minor
units) and return groups of ids. A proposed group is correct when its payment ids and its
invoice ids both equal those of a solution group. Precision is correct groups over proposed
groups; recall is correct groups over solution groups. Recall cannot exceed about 95 %
because the 5 % of payments without an invoice number are out of reach for every
implementation here (the default weights do not let a payment without a reference reach the
suggest threshold).

**What is timed.** Each implementation, dataset and size runs in a fresh child process (never
two at once). The child loads the data into the native form of its implementation, which is
reported as build time and is not timed: Arrow tables for `remitmatch`, lists of named tuples
for pure Python, DataFrames for pandas. Then it times the matching call alone, up to and
including the answer as a list of groups of ids, the same shape for all three. For
`remitmatch` that includes turning the result object into groups; the time of the `reconcile`
call alone is in the results file as well (`time_call`). The garbage collector is disabled
during the timed call in every implementation, as `timeit` does. Best of 3 runs up to 100,000
payments, one run at 1,000,000. A matching call that exceeds 10 minutes would be recorded as
"did not finish"; none did.

**Memory.** Peak resident size of the child process (`ru_maxrss`), which includes the
interpreter, the imported libraries (about 150 MB with pandas and pyarrow loaded), the input
in its native form and the result. The resident size after loading the data is reported as
well, so the memory of the call itself can be read as the difference.

**Machine.** Apple M3 Max, 16 threads, 128 GB, macOS 27.2.1, Python 3.14.0, pandas 3.1.1,
numpy 2.5.5, pyarrow 26.0.1, rapidfuzz 3.14.6. `remitmatch` is a **release build of `main` at
commit `7ccdca0`, after 0.1.0a2** (`maturin develop --release`; the package version string is
still 0.1.0a2), using rayon's default of one thread per core. The machine was heavily
shared with other work: the load average was 13.6 before the run started (it stayed between
12 and 18 during the four minutes I waited for it to drop) and between 12.7 and 20.9 when
each run started, on 16 threads. That load affects every implementation, and the 16-thread
`remitmatch` possibly more than the single-threaded baselines; how much was not measured. The
load average of each run is in the results file.

## Results

`one-to-one` (time of the best run; peak memory in MB of 10^6 bytes):

| Payments | Implementation | Time | Peak memory | Precision | Recall | Precision of `auto` matches |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 10,000 | remitmatch | 0.016 s | 175 MB | 100.000 % | 94.81 % | 100.000 % (7,863 matches) |
| 10,000 | Pure Python | 0.029 s | 170 MB | 100.000 % | 94.91 % | n/a |
| 10,000 | pandas + rapidfuzz | 0.032 s | 163 MB | 100.000 % | 94.91 % | n/a |
| 100,000 | remitmatch | 0.136 s | 415 MB | 100.000 % | 94.79 % | 100.000 % (78,976 matches) |
| 100,000 | Pure Python | 0.374 s | 402 MB | 99.999 % | 94.88 % | n/a |
| 100,000 | pandas + rapidfuzz | 0.268 s | 295 MB | 99.999 % | 94.88 % | n/a |
| 1,000,000 | remitmatch | 1.594 s | 2,510 MB | 100.000 % | 94.80 % | 100.000 % (790,907 matches) |
| 1,000,000 | Pure Python | 4.896 s | 2,709 MB | 99.999 % | 94.91 % | n/a |
| 1,000,000 | pandas + rapidfuzz | 3.290 s | 1,575 MB | 99.999 % | 94.91 % | n/a |

`mixed`:

| Payments | Implementation | Time | Peak memory | Precision | Recall | Precision of `auto` matches |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 10,000 | remitmatch | 0.018 s | 173 MB | 100.000 % | 94.90 % | 100.000 % (7,240 matches) |
| 10,000 | Pure Python | 0.035 s | 168 MB | 100.000 % | 88.10 % | n/a |
| 10,000 | pandas + rapidfuzz | 0.031 s | 160 MB | 100.000 % | 88.10 % | n/a |
| 100,000 | remitmatch | 0.198 s | 403 MB | 99.994 % | 94.87 % | 99.999 % (72,653 matches) |
| 100,000 | Pure Python | 0.448 s | 386 MB | 99.957 % | 88.40 % | n/a |
| 100,000 | pandas + rapidfuzz | 0.252 s | 293 MB | 99.953 % | 88.40 % | n/a |
| 1,000,000 | remitmatch | 2.247 s | 2,407 MB | 99.993 % | 93.88 % | 100.000 % (726,447 matches) |
| 1,000,000 | Pure Python | 6.467 s | 2,612 MB | 99.924 % | 88.45 % | n/a |
| 1,000,000 | pandas + rapidfuzz | 3.077 s | 1,352 MB | 99.917 % | 88.45 % | n/a |

Time of each baseline divided by the time of `remitmatch` (above 1 means `remitmatch` is
faster):

| Dataset | Payments | to pure Python | to pandas + rapidfuzz |
| --- | ---: | ---: | ---: |
| one-to-one | 10,000 | 1.83 | 2.04 |
| one-to-one | 100,000 | 2.74 | 1.96 |
| one-to-one | 1,000,000 | 3.07 | 2.06 |
| mixed | 10,000 | 1.89 | 1.68 |
| mixed | 100,000 | 2.26 | 1.27 |
| mixed | 1,000,000 | 2.88 | 1.37 |

Everything measured, including the build time, the time of the `reconcile` call alone, the
memory after loading and the time of every run:

| Dataset | Payments | Implementation | Time | reconcile call only | Build time | Memory after load | Peak memory | Precision | Recall | Precision of auto | Times of the runs |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| one-to-one | 10,000 | remitmatch | 0.016 s | 0.015 s | 0.029 s | 148 MB | 175 MB | 100.000 % | 94.81 % | 100.000 % | 0.018, 0.017, 0.016 |
| one-to-one | 10,000 | Pure Python | 0.029 s | n/a | 0.048 s | 162 MB | 170 MB | 100.000 % | 94.91 % | n/a | 0.029, 0.029, 0.029 |
| one-to-one | 10,000 | pandas + rapidfuzz | 0.032 s | n/a | 0.030 s | 152 MB | 163 MB | 100.000 % | 94.91 % | n/a | 0.044, 0.032, 0.033 |
| one-to-one | 100,000 | remitmatch | 0.136 s | 0.125 s | 0.035 s | 194 MB | 415 MB | 100.000 % | 94.79 % | 100.000 % | 0.158, 0.136, 0.151 |
| one-to-one | 100,000 | Pure Python | 0.374 s | n/a | 0.256 s | 305 MB | 402 MB | 99.999 % | 94.88 % | n/a | 0.400, 0.394, 0.374 |
| one-to-one | 100,000 | pandas + rapidfuzz | 0.268 s | n/a | 0.038 s | 200 MB | 295 MB | 99.999 % | 94.88 % | n/a | 0.273, 0.268, 0.275 |
| one-to-one | 1,000,000 | remitmatch | 1.594 s | 1.466 s | 0.095 s | 519 MB | 2,510 MB | 100.000 % | 94.80 % | 100.000 % | 1.594 |
| one-to-one | 1,000,000 | Pure Python | 4.896 s | n/a | 2.749 s | 1,747 MB | 2,709 MB | 99.999 % | 94.91 % | n/a | 4.896 |
| one-to-one | 1,000,000 | pandas + rapidfuzz | 3.290 s | n/a | 0.098 s | 569 MB | 1,575 MB | 99.999 % | 94.91 % | n/a | 3.290 |
| mixed | 10,000 | remitmatch | 0.018 s | 0.017 s | 0.027 s | 148 MB | 173 MB | 100.000 % | 94.90 % | 100.000 % | 0.018, 0.019, 0.018 |
| mixed | 10,000 | Pure Python | 0.035 s | n/a | 0.047 s | 161 MB | 168 MB | 100.000 % | 88.10 % | n/a | 0.038, 0.036, 0.035 |
| mixed | 10,000 | pandas + rapidfuzz | 0.031 s | n/a | 0.029 s | 150 MB | 160 MB | 100.000 % | 88.10 % | n/a | 0.031, 0.031, 0.031 |
| mixed | 100,000 | remitmatch | 0.198 s | 0.187 s | 0.034 s | 193 MB | 403 MB | 99.994 % | 94.87 % | 99.999 % | 0.205, 0.198, 0.203 |
| mixed | 100,000 | Pure Python | 0.448 s | n/a | 0.247 s | 302 MB | 386 MB | 99.957 % | 88.40 % | n/a | 0.448, 0.456, 0.453 |
| mixed | 100,000 | pandas + rapidfuzz | 0.252 s | n/a | 0.038 s | 198 MB | 293 MB | 99.953 % | 88.40 % | n/a | 0.253, 0.254, 0.252 |
| mixed | 1,000,000 | remitmatch | 2.247 s | 2.136 s | 0.087 s | 503 MB | 2,407 MB | 99.993 % | 93.88 % | 100.000 % | 2.247 |
| mixed | 1,000,000 | Pure Python | 6.467 s | n/a | 2.638 s | 1,703 MB | 2,612 MB | 99.924 % | 88.45 % | n/a | 6.467 |
| mixed | 1,000,000 | pandas + rapidfuzz | 3.077 s | n/a | 0.093 s | 547 MB | 1,352 MB | 99.917 % | 88.45 % | n/a | 3.077 |

Build time is the time to read the Parquet file into the native form of each implementation.
It is not part of the timed call, but it is a real cost for the pure Python baseline
(2.7 s at 1,000,000 payments, against 0.10 s for Arrow tables).

## The implementations

- **`remitmatch`**: `reconcile(invoices, payments, MatchConfig(amount_tolerance_minor=300))`
  with both inputs as Arrow tables, default thresholds and weights. Every proposed match
  counts (`auto`, `suggested` and `ambiguous`), and the tables also give the precision of the
  `auto` matches alone. It finds one-to-one, one-to-many and many-to-one matches and returns a
  per-signal explanation of each; none of that is used in the one-to-one comparison.
- **Pure Python** ([`baselines/pure_python.py`](baselines/pure_python.py)), standard library
  only. Exact pass: invoice-like numbers (`INV` followed by digits, after removing spaces,
  dashes and slashes) are extracted from the reference and looked up in a dictionary; the pair
  is kept when scope and currency agree and the amounts differ by at most the tolerance.
  Fuzzy pass, for payments that mention such a number but found no exact match: candidates
  come from an amount index (`bisect`) inside the 30-day date window, and each is scored with
  the four signals of `remitmatch` (reference, amount, payer name, date; the same default
  weights and the same 0.60 threshold) using `difflib`. Greedy assignment, best score first.
- **pandas + rapidfuzz** ([`baselines/pandas_rapidfuzz.py`](baselines/pandas_rapidfuzz.py)):
  the same two passes, vectorized. The exact pass is a regular expression, an `explode` and a
  merge. The fuzzy pass builds the candidates with an amount-window join (sorted composite
  keys and `numpy.searchsorted`, no row loop), cuts them by the date window, and scores the
  reference and the payer name with `rapidfuzz.process.cpdist`. The greedy assignment is
  vectorized by repeatedly accepting the pairs that are the best choice of both their payment
  and their invoice, which gives the same result as sorting and accepting one by one.

What the baselines do not do:

- They match one payment to one invoice. They cannot find a payment that settles several
  invoices or several payments that settle one invoice, which is why they lose recall on the
  `mixed` dataset.
- They never flag a match as ambiguous and give no per-signal explanation.
- They assume the invoice numbers start with `INV`, which makes the exact pass a single
  regular expression. `remitmatch` does not need to know the prefix: it searches the
  references for the open invoice numbers it was given.
- They were written for this benchmark, by the author of `remitmatch`.

## Fairness notes

- The baselines were written to be fast, not to be slow. Their recall and precision are
  within 0.2 points of `remitmatch` on one-to-one data, so they do the same job on this data.
  Where a choice was open, the faster correct formulation was kept (see below).
- The data comes from `generate.py`, a Python generator that mirrors the noise model of the
  generator used by the accuracy tests of `remitmatch`. The default weights of `remitmatch`
  have not been tuned on either. Real bank statements are different, so these numbers say
  nothing about accuracy in production.
- The data has references that contain the invoice number, which is what makes an exact
  dictionary lookup so effective. Data where references are noisier would cost the baselines
  more than the engine.
- Disabling the garbage collector favours the baselines, because they build many Python
  objects. In exploratory runs, with the collector on and the input already in memory, the
  pure Python baseline took about three times longer at 1,000,000 payments and the pandas
  baseline about as much (the cost comes from the collector scanning the millions of live
  objects while the answer is built). Those exploratory numbers are not in the results file.
- The 1,000,000 payment case is a single run. The three runs at 10,000 and
  100,000 payments differ by up to 16 percent from the best one, and by 37 percent in one case
  (pandas + rapidfuzz at 10,000 payments, 0.044 s against 0.032 s), which the machine load
  explains in part (the file has every run).
- One machine, shared with other work. The numbers will differ elsewhere, and the ratios
  between implementations will too. Four scopes and two currencies give eight partitions that
  `remitmatch` processes in parallel; with a single large scope the parallelism disappears and
  the README of the project documents a quadratic cost inside one scope. That case is not
  measured here.
- The `remitmatch` memory is higher than that of pandas + rapidfuzz at every size. It has not
  been analysed further in this slice.

### What was tried to make the baselines faster

- Pure Python: an upper-bound check that skips candidates which cannot reach the threshold
  before running `difflib` made no measurable difference at 1,000,000 payments (4.21 s with
  and without), so it was removed. Names are normalized lazily and once per distinct name,
  and `SequenceMatcher` objects keep the sequence they index fixed per payment.
- pandas + rapidfuzz: scoring with all threads
  (`workers=-1`) instead of one changed the total from 3.03 s to 2.96 s, because `cpdist`
  takes 0.01 s of it, so the default was kept. Normalizing the references with `pyarrow.compute`
  instead of the pandas string methods saves about 0.3 s per million strings, but it leaves
  pandas, so the pandas methods were kept. Day numbers are computed for the candidate rows
  only, not for whole columns.
- Both: the garbage collector, as described above.

(Those timings were taken in exploratory runs while developing, on the same machine.)

## Why reconPy is not in the comparison

The RFC lists reconPy (version 1.0.5, a single module on PyPI) as a baseline. It was installed
in a throwaway environment outside this repository and its code and documentation were read.
It does not fit this task:

- Its only function, `recon(left_df, right_df, left_exact_match_cols, right_exact_match_cols,
  left_compare_cols, right_compare_cols, weight, tolerance_percentage, ...)`, first joins the
  two tables on **exact-match columns** (an outer `pandas.merge`), then compares the
  **compare columns** with a tolerance given as a **percentage**, measured against the
  right-hand value, and finally assigns pairs greedily with a Python loop over `iterrows`.
- There is no absolute tolerance. A fee of 300 minor units is 42.9 % of a payment of 700 and
  0.006 % of a payment of 4,999,700. On a two-row example, where both payments are short by
  exactly 300 units, a 30 % tolerance matched only the large pair, because the small pair is
  42.9 % off. No single percentage accepts "up to 300 units" at every amount.
- The only keys that can serve as exact-match columns are scope and currency, so the merge
  produces every payment-invoice pair within a key. On the datasets of this benchmark that is
  13 million rows at 10,000 payments, 1.3 billion at 100,000 and 132 billion at 1,000,000
  (sum over the eight partitions of payments times invoices), before any comparison runs. A
  reference that contains an invoice number cannot be an exact key, because it is free text
  around the number.
- It modifies the frames it is given (it adds id columns) and compares strings with
  `fuzzywuzzy` through `DataFrame.apply(axis=1)`.

A benchmark would have needed an adapter that re-implements the candidate search outside
reconPy, which would measure the adapter and not the library, so the dependency was not added.

## Reproduce

Needs a Rust toolchain and [uv](https://docs.astral.sh/uv/). Run the commands from the
repository root. The `--no-sync` flags keep `uv` from rebuilding the extension in debug mode
after the release build.

```bash
uv sync --group bench
uv run --no-sync maturin develop --release
uv run --no-sync python benchmarks/run.py --remitmatch-build release --commit "$(git rev-parse --short HEAD)"
uv run --no-sync python benchmarks/plot.py
uv run --no-sync maturin develop          # back to the debug build
```

`run.py` takes `--sizes`, `--implementations`, `--datasets`, `--scopes`, `--seed`, `--runs`,
`--timeout`, `--output`, `--remitmatch-build`, `--commit` and `--note`. For example, a quick run:

```bash
uv run --no-sync python benchmarks/run.py --sizes 10000 --output /tmp/results.json
```

The full run takes about two minutes on the machine above. The results file records the
machine, the operating system, the versions of Python and of every package, the seeds, the scopes, the tolerance, the
build profile declared for `remitmatch` and the load average. Run the tests of the generator,
the baselines and the runner with `uv run --group bench pytest benchmarks`; they are skipped
when the `bench` dependency group is not installed.
