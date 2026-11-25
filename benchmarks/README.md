# Benchmarks

A reproducible comparison of `remitmatch` with two other ways of doing the same job in
Python, on the same synthetic data. Every number below comes from
[`results/results.json`](results/results.json), written by [`run.py`](run.py).

## Summary

- On one-to-one data `remitmatch` is **not faster** than the two baselines on this machine: it
  is 1.3 to 1.5 times slower than pandas + rapidfuzz at every size, and slower than pure
  Python at 10,000 and 100,000 payments (1.4 and 1.15 times) but faster at 1,000,000
  (3.7 s against 4.2 s). The RFC target of at least 20 times over pandas + rapidfuzz is
  **not met**: the measured ratio is 0.78, 0.68 and 0.76 at 10,000, 100,000 and 1,000,000
  payments.
- Its peak memory is the highest at 100,000 and 1,000,000 payments (3.3 GB against 1.6 GB for
  pandas + rapidfuzz at 1,000,000).
- Precision is 99.99 % or better for every implementation. On the `mixed` dataset, which
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
numpy 2.5.5, pyarrow 26.0.1, rapidfuzz 3.14.6. `remitmatch` is a **release build of version
0.1.0a2** (`maturin develop --release`), using rayon's default of one thread per core. The
machine was shared with other work during the run: the load average was between 3.8 and 5.3
when each run started (it was already about 5 before the benchmark began, and it includes
the benchmark itself), and the memory pressure was low. The load
average of each run is in the results file.

## Results

`one-to-one` (time of the best run; peak memory in MB of 10^6 bytes):

| Payments | Implementation | Time | Peak memory | Precision | Recall | Precision of `auto` matches |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 10,000 | remitmatch | 0.035 s | 181 MB | 100.000 % | 94.81 % | 100.000 % (7,863 matches) |
| 10,000 | Pure Python | 0.025 s | 166 MB | 100.000 % | 94.91 % | n/a |
| 10,000 | pandas + rapidfuzz | 0.027 s | 161 MB | 100.000 % | 94.91 % | n/a |
| 100,000 | remitmatch | 0.339 s | 499 MB | 100.000 % | 94.79 % | 100.000 % (78,976 matches) |
| 100,000 | Pure Python | 0.295 s | 402 MB | 99.999 % | 94.88 % | n/a |
| 100,000 | pandas + rapidfuzz | 0.229 s | 291 MB | 99.999 % | 94.88 % | n/a |
| 1,000,000 | remitmatch | 3.731 s | 3,260 MB | 100.000 % | 94.80 % | 100.000 % (790,907 matches) |
| 1,000,000 | Pure Python | 4.229 s | 2,717 MB | 99.999 % | 94.91 % | n/a |
| 1,000,000 | pandas + rapidfuzz | 2.824 s | 1,573 MB | 99.999 % | 94.91 % | n/a |

`mixed`:

| Payments | Implementation | Time | Peak memory | Precision | Recall | Precision of `auto` matches |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 10,000 | remitmatch | 0.035 s | 178 MB | 100.000 % | 94.90 % | 100.000 % (7,240 matches) |
| 10,000 | Pure Python | 0.031 s | 165 MB | 100.000 % | 88.10 % | n/a |
| 10,000 | pandas + rapidfuzz | 0.027 s | 163 MB | 100.000 % | 88.10 % | n/a |
| 100,000 | remitmatch | 0.394 s | 488 MB | 99.994 % | 94.87 % | 99.999 % (72,653 matches) |
| 100,000 | Pure Python | 0.365 s | 388 MB | 99.957 % | 88.40 % | n/a |
| 100,000 | pandas + rapidfuzz | 0.225 s | 288 MB | 99.953 % | 88.40 % | n/a |
| 1,000,000 | remitmatch | 4.244 s | 3,102 MB | 99.993 % | 93.88 % | 100.000 % (726,447 matches) |
| 1,000,000 | Pure Python | 5.481 s | 2,627 MB | 99.924 % | 88.45 % | n/a |
| 1,000,000 | pandas + rapidfuzz | 2.700 s | 1,338 MB | 99.917 % | 88.45 % | n/a |

Ratio of the time of `remitmatch` to the time of each baseline (above 1 means `remitmatch`
is slower):

| Dataset | Payments | to pure Python | to pandas + rapidfuzz |
| --- | ---: | ---: | ---: |
| one-to-one | 10,000 | 1.40 | 1.28 |
| one-to-one | 100,000 | 1.15 | 1.48 |
| one-to-one | 1,000,000 | 0.88 | 1.32 |
| mixed | 10,000 | 1.15 | 1.30 |
| mixed | 100,000 | 1.08 | 1.75 |
| mixed | 1,000,000 | 0.77 | 1.57 |

Everything measured, including the build time, the time of the `reconcile` call alone, the
memory after loading and the time of every run:

| Dataset | Payments | Implementation | Time | reconcile call only | Build time | Memory after load | Peak memory | Precision | Recall | Precision of auto | Times of the runs |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| one-to-one | 10,000 | remitmatch | 0.035 s | 0.032 s | 0.024 s | 149 MB | 181 MB | 100.000 % | 94.81 % | 100.000 % | 0.038, 0.035, 0.035 |
| one-to-one | 10,000 | Pure Python | 0.025 s | n/a | 0.043 s | 159 MB | 166 MB | 100.000 % | 94.91 % | n/a | 0.025, 0.025, 0.025 |
| one-to-one | 10,000 | pandas + rapidfuzz | 0.027 s | n/a | 0.025 s | 151 MB | 161 MB | 100.000 % | 94.91 % | n/a | 0.027, 0.028, 0.029 |
| one-to-one | 100,000 | remitmatch | 0.339 s | 0.314 s | 0.030 s | 195 MB | 499 MB | 100.000 % | 94.79 % | 100.000 % | 0.358, 0.339, 0.344 |
| one-to-one | 100,000 | Pure Python | 0.295 s | n/a | 0.229 s | 306 MB | 402 MB | 99.999 % | 94.88 % | n/a | 0.319, 0.295, 0.321 |
| one-to-one | 100,000 | pandas + rapidfuzz | 0.229 s | n/a | 0.031 s | 195 MB | 291 MB | 99.999 % | 94.88 % | n/a | 0.233, 0.247, 0.229 |
| one-to-one | 1,000,000 | remitmatch | 3.731 s | 3.436 s | 0.080 s | 533 MB | 3,260 MB | 100.000 % | 94.80 % | 100.000 % | 3.731 |
| one-to-one | 1,000,000 | Pure Python | 4.229 s | n/a | 2.578 s | 1,752 MB | 2,717 MB | 99.999 % | 94.91 % | n/a | 4.229 |
| one-to-one | 1,000,000 | pandas + rapidfuzz | 2.824 s | n/a | 0.092 s | 567 MB | 1,573 MB | 99.999 % | 94.91 % | n/a | 2.824 |
| mixed | 10,000 | remitmatch | 0.035 s | 0.033 s | 0.024 s | 149 MB | 178 MB | 100.000 % | 94.90 % | 100.000 % | 0.037, 0.037, 0.035 |
| mixed | 10,000 | Pure Python | 0.031 s | n/a | 0.042 s | 158 MB | 165 MB | 100.000 % | 88.10 % | n/a | 0.031, 0.031, 0.032 |
| mixed | 10,000 | pandas + rapidfuzz | 0.027 s | n/a | 0.026 s | 153 MB | 163 MB | 100.000 % | 88.10 % | n/a | 0.028, 0.030, 0.027 |
| mixed | 100,000 | remitmatch | 0.394 s | 0.368 s | 0.030 s | 195 MB | 488 MB | 99.994 % | 94.87 % | 99.999 % | 0.417, 0.401, 0.394 |
| mixed | 100,000 | Pure Python | 0.365 s | n/a | 0.226 s | 303 MB | 388 MB | 99.957 % | 88.40 % | n/a | 0.365, 0.402, 0.371 |
| mixed | 100,000 | pandas + rapidfuzz | 0.225 s | n/a | 0.031 s | 197 MB | 288 MB | 99.953 % | 88.40 % | n/a | 0.244, 0.232, 0.225 |
| mixed | 1,000,000 | remitmatch | 4.244 s | 3.985 s | 0.078 s | 517 MB | 3,102 MB | 99.993 % | 93.88 % | 100.000 % | 4.244 |
| mixed | 1,000,000 | Pure Python | 5.481 s | n/a | 2.504 s | 1,713 MB | 2,627 MB | 99.924 % | 88.45 % | n/a | 5.481 |
| mixed | 1,000,000 | pandas + rapidfuzz | 2.700 s | n/a | 0.085 s | 526 MB | 1,338 MB | 99.917 % | 88.45 % | n/a | 2.700 |

Build time is the time to read the Parquet file into the native form of each implementation.
It is not part of the timed call, but it is a real cost for the pure Python baseline
(2.5 s at 1,000,000 payments, against 0.08 s for Arrow tables).

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
- The data comes from the generator that the accuracy tests of `remitmatch` use, with the same
  noise model, and the default weights of `remitmatch` were tuned on it. Real bank statements
  are different, so these numbers say nothing about accuracy in production.
- The data has references that contain the invoice number, which is what makes an exact
  dictionary lookup so effective. Data where references are noisier would cost the baselines
  more than the engine.
- Disabling the garbage collector favours the baselines, because they build many Python
  objects. In exploratory runs, with the collector on and the input already in memory, the
  pure Python baseline took about three times longer at 1,000,000 payments and the pandas
  baseline about as much (the cost comes from the collector scanning the millions of live
  objects while the answer is built). Those exploratory numbers are not in the results file.
- The 1,000,000 payment case is a single run. The spread of the three runs at 10,000 and
  100,000 payments is between 0 and 10 percent of the best run (the file has every run).
- One machine, shared with other work. The numbers will differ elsewhere, and the ratios
  between implementations will too. Four scopes and two currencies give eight partitions that
  `remitmatch` processes in parallel; with a single large scope the parallelism disappears and
  the README of the project documents a quadratic cost inside one scope. That case is not
  measured here.
- The `remitmatch` memory is the highest at 100,000 and 1,000,000 payments. It has not been
  analysed further in this slice.

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
uv run --no-sync python benchmarks/run.py --remitmatch-build release
uv run --no-sync python benchmarks/plot.py
uv run --no-sync maturin develop          # back to the debug build
```

`run.py` takes `--sizes`, `--implementations`, `--datasets`, `--scopes`, `--seed`, `--runs`,
`--timeout`, `--output`, `--remitmatch-build` and `--note`. For example, a quick run:

```bash
uv run --no-sync python benchmarks/run.py --sizes 10000 --output /tmp/results.json
```

The full run takes about two minutes on the machine above. The results file records the
machine, the operating system, the versions of Python and of every package, the seeds, the scopes, the tolerance, the
build profile declared for `remitmatch` and the load average. Run the tests of the generator,
the baselines and the runner with `uv run --group bench pytest benchmarks`; they are skipped
when the `bench` dependency group is not installed.
