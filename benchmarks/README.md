# Benchmarks

A reproducible comparison of `remitmatch` with two other ways of doing the same job in Python, on the same synthetic data. Every number below comes from [`results/results.json`](results/results.json), written by [`run.py`](run.py), unless it is labelled exploratory or history.

## Summary

On data spread over several scopes, `remitmatch` on all 16 threads of the test machine finishes sooner than the single-threaded Python baselines on one-to-one data (1.36 to 1.71 times faster than pandas + rapidfuzz, 1.96 to 3.37 times faster than pure Python), but that comes from the threads: on one thread it is slower than both (1.87 to 2.47 times slower than pandas + rapidfuzz, 1.12 to 1.40 times slower than pure Python), and with one large scope it is slower than both even on all threads (1.84 to 3.21 and 1.23 to 1.53 times slower). On data with grouped and partial payments it finds more of the true groups (93.1 to 94.9 % against 88.1 to 88.5 %); the target of 20 times over pandas + rapidfuzz is not met in any layout.

- **Threads.** `remitmatch` uses rayon, one thread per core by default (16 here); the baselines use one thread. The results file therefore has a second `remitmatch` entry restricted to one thread (`RAYON_NUM_THREADS=1`). Going from one thread to all threads speeds `remitmatch` up 2.7 to 3.8 times on one-to-one data and 2.8 to 4.8 times on the mixed dataset with four scopes, and by 1.01 to 1.05 times with one scope, where there is a single partition to process.
- **Recall.** On one-to-one data `remitmatch` finds slightly fewer true matches than the baselines: 94.79 to 94.81 % against 94.88 to 94.91 % with four scopes, 94.07 to 94.34 % against 94.88 to 94.91 % with one scope.
- **Memory.** With four scopes `remitmatch` has a higher peak than pandas + rapidfuzz at every size (2,495 MB against 1,647 MB at 1,000,000 payments), a higher one than pure Python at 10,000 and 100,000 payments and a lower one at 1,000,000 (2,726 MB). On one thread it is 2,251 MB at 1,000,000 payments.
- **Mixed data.** With grouped and partial payments `remitmatch` finds 93.88 to 94.90 % of the true groups (four scopes) and the baselines 88.10 to 88.45 %, because they match one payment to one invoice only. On all threads it is 1.26, 0.93 and 1.05 times as fast as pandas + rapidfuzz on that dataset.
- **Build.** The build measured is a release build of `main` at commit `7ccdca0`, after the published 0.1.0a2 (the package version string is still 0.1.0a2). *History, not in the results file:* a first measurement on the published 0.1.0a2 gave 0.035 s, 0.339 s and 3.731 s for `remitmatch` on one-to-one data with four scopes at 10,000, 100,000 and 1,000,000 payments, slower than pandas + rapidfuzz; it is not a current number.
- reconPy was not benchmarked: it cannot express the task (see below).

## Method

**Data.** [`generate.py`](generate.py) is a deterministic generator (`random.Random(seed)`) with a known solution. It mirrors the noise model of the Rust generator used by the accuracy tests (`crates/remitmatch/tests/common/mod.rs`): 15 % of payments that settle an invoice are short by a bank fee of 1 to 300 minor units, 5 % of their references have one typo in the invoice number, 5 % carry no invoice number, 30 % of payer names are abbreviated bank-style variants, 10 % of invoices are unpaid, and unrelated payments add about 5 % of the others. Counting the unrelated payments, which carry no invoice number either, about 9.5 % of all payments have none. Two datasets:

- `one-to-one`: the noise above, no grouped or partial payments. Every implementation can in principle find every match.
- `mixed`: the same noise plus grouped payments (one payment for 2 to 4 invoices) and partial payments (one invoice paid by 2 or 3 payments). A batch that ends with one member is a plain payment, so only a few percent of the invoices end up in a multi-invoice group.

Two layouts (`--layouts`): `four-scopes` has four scopes and two currencies, so eight partitions, and `one-scope` has one scope and one currency, so one partition. Rows of different partitions never belong to one group. Sizes are exactly 10,000, 100,000 and 1,000,000 payments (the number of invoices is in the results file). The runs use seed 1.

**What is compared.** All implementations get the same rows and the same tolerance (300 minor units) and return groups of ids. A proposed group is correct when its payment ids and its invoice ids both equal those of a solution group. Precision is correct groups over proposed groups; recall is correct groups over solution groups. Recall cannot exceed about 95 % because the payments without an invoice number are out of reach for every implementation here (the default weights do not let a payment without a reference reach the suggest threshold).

**What is timed.** Each implementation, layout, dataset and size runs in a fresh child process (never two at once). The child loads the data into the native form of its implementation, which is reported as build time and is not timed: Arrow tables for `remitmatch`, lists of named tuples for pure Python, DataFrames for pandas. Then it times the matching call alone, up to and including the answer as a list of groups of ids, the same shape for all of them. For `remitmatch` that includes turning the result object into groups; the time of the `reconcile` call alone is in the results file as well (`time_call`). The garbage collector is disabled during the timed call in every implementation, as `timeit` does. Best of 3 runs up to 100,000 payments, one run at 1,000,000. A matching call that exceeds 10 minutes would be recorded as "did not finish"; none did.

**Threads.** Every run records the number of threads the implementation may use (`threads`): rayon's pool for `remitmatch`, which is one per logical CPU unless `RAYON_NUM_THREADS` is set, and one for the baselines. The `remitmatch-1-thread` entry runs the child with `RAYON_NUM_THREADS=1`; the variable is recorded in the run. The operating system thread count after the call is recorded as well (`os_threads_after_call`) as information only, because the interpreter and the libraries start helper threads of their own.

**Memory.** Peak resident size of the child process (`ru_maxrss`), which includes the interpreter, the imported libraries (about 150 MB with pandas and pyarrow loaded), the input in its native form and the result. The resident size after loading the data is reported as well, so the memory of the call itself can be read as the difference.

**Machine.** Apple M3 Max, 16 threads, 128 GB, macOS 27.2.1, Python 3.14.0, pandas 3.1.1, numpy 2.5.5, pyarrow 26.0.1, rapidfuzz 3.14.6. `remitmatch` is a **release build of `main` at commit `7ccdca0`, after 0.1.0a2** (`maturin develop --release`; the package version string is still 0.1.0a2). The machine was heavily shared with other work: the load average was 12.6 when the run started and between 9 and 14 when each run started, on 16 threads. That load affects every implementation, and the multi-threaded `remitmatch` possibly more than the single-threaded baselines; how much was not measured. The load of each run is in the results file.

## Results

Four scopes, `one-to-one` (time of the best run; peak memory in MB of 10^6 bytes):

| Payments | Implementation | Threads | Time | Peak memory | Precision | Recall | Precision of `auto` matches (count) |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 10,000 | remitmatch | 16 | 0.016 s | 175 MB | 100.000 % | 94.81 % | 100.000 % (7,863) |
| 10,000 | remitmatch | 1 | 0.043 s | 167 MB | 100.000 % | 94.81 % | 100.000 % (7,863) |
| 10,000 | Pure Python | 1 | 0.031 s | 165 MB | 100.000 % | 94.91 % | n/a |
| 10,000 | pandas + rapidfuzz | 1 | 0.023 s | 164 MB | 100.000 % | 94.91 % | n/a |
| 100,000 | remitmatch | 16 | 0.132 s | 413 MB | 100.000 % | 94.79 % | 100.000 % (78,976) |
| 100,000 | remitmatch | 1 | 0.442 s | 382 MB | 100.000 % | 94.79 % | 100.000 % (78,976) |
| 100,000 | Pure Python | 1 | 0.360 s | 402 MB | 99.999 % | 94.88 % | n/a |
| 100,000 | pandas + rapidfuzz | 1 | 0.179 s | 306 MB | 99.999 % | 94.88 % | n/a |
| 1,000,000 | remitmatch | 16 | 1.445 s | 2,495 MB | 100.000 % | 94.80 % | 100.000 % (790,907) |
| 1,000,000 | remitmatch | 1 | 5.439 s | 2,251 MB | 100.000 % | 94.80 % | 100.000 % (790,907) |
| 1,000,000 | Pure Python | 1 | 4.869 s | 2,726 MB | 99.999 % | 94.91 % | n/a |
| 1,000,000 | pandas + rapidfuzz | 1 | 2.475 s | 1,647 MB | 99.999 % | 94.91 % | n/a |

Four scopes, `mixed`:

| Payments | Implementation | Threads | Time | Peak memory | Precision | Recall | Precision of `auto` matches (count) |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 10,000 | remitmatch | 16 | 0.018 s | 174 MB | 100.000 % | 94.90 % | 100.000 % (7,240) |
| 10,000 | remitmatch | 1 | 0.052 s | 166 MB | 100.000 % | 94.90 % | 100.000 % (7,240) |
| 10,000 | Pure Python | 1 | 0.036 s | 165 MB | 100.000 % | 88.10 % | n/a |
| 10,000 | pandas + rapidfuzz | 1 | 0.023 s | 165 MB | 100.000 % | 88.10 % | n/a |
| 100,000 | remitmatch | 16 | 0.190 s | 410 MB | 99.994 % | 94.87 % | 99.999 % (72,653) |
| 100,000 | remitmatch | 1 | 0.878 s | 368 MB | 99.994 % | 94.87 % | 99.999 % (72,653) |
| 100,000 | Pure Python | 1 | 0.446 s | 390 MB | 99.957 % | 88.40 % | n/a |
| 100,000 | pandas + rapidfuzz | 1 | 0.177 s | 294 MB | 99.953 % | 88.40 % | n/a |
| 1,000,000 | remitmatch | 16 | 2.214 s | 2,355 MB | 99.993 % | 93.88 % | 100.000 % (726,447) |
| 1,000,000 | remitmatch | 1 | 10.655 s | 2,141 MB | 99.993 % | 93.88 % | 100.000 % (726,447) |
| 1,000,000 | Pure Python | 1 | 6.385 s | 2,613 MB | 99.924 % | 88.45 % | n/a |
| 1,000,000 | pandas + rapidfuzz | 1 | 2.331 s | 1,437 MB | 99.917 % | 88.45 % | n/a |

One scope, `one-to-one`:

| Payments | Implementation | Threads | Time | Peak memory | Precision | Recall | Precision of `auto` matches (count) |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 10,000 | remitmatch | 16 | 0.042 s | 174 MB | 100.000 % | 94.34 % | 100.000 % (7,829) |
| 10,000 | remitmatch | 1 | 0.043 s | 172 MB | 100.000 % | 94.34 % | 100.000 % (7,829) |
| 10,000 | Pure Python | 1 | 0.027 s | 166 MB | 100.000 % | 94.91 % | n/a |
| 10,000 | pandas + rapidfuzz | 1 | 0.023 s | 163 MB | 100.000 % | 94.91 % | n/a |
| 100,000 | remitmatch | 16 | 0.516 s | 387 MB | 100.000 % | 94.19 % | 100.000 % (78,541) |
| 100,000 | remitmatch | 1 | 0.537 s | 386 MB | 100.000 % | 94.19 % | 100.000 % (78,541) |
| 100,000 | Pure Python | 1 | 0.368 s | 403 MB | 99.997 % | 94.88 % | n/a |
| 100,000 | pandas + rapidfuzz | 1 | 0.181 s | 302 MB | 99.997 % | 94.88 % | n/a |
| 1,000,000 | remitmatch | 16 | 8.030 s | 2,461 MB | 100.000 % | 94.07 % | 100.000 % (785,637) |
| 1,000,000 | remitmatch | 1 | 8.363 s | 2,455 MB | 100.000 % | 94.07 % | 100.000 % (785,637) |
| 1,000,000 | Pure Python | 1 | 6.540 s | 2,721 MB | 99.990 % | 94.90 % | n/a |
| 1,000,000 | pandas + rapidfuzz | 1 | 2.502 s | 1,701 MB | 99.989 % | 94.90 % | n/a |

One scope, `mixed`:

| Payments | Implementation | Threads | Time | Peak memory | Precision | Recall | Precision of `auto` matches (count) |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 10,000 | remitmatch | 16 | 0.081 s | 171 MB | 100.000 % | 94.44 % | 100.000 % (7,210) |
| 10,000 | remitmatch | 1 | 0.086 s | 169 MB | 100.000 % | 94.44 % | 100.000 % (7,210) |
| 10,000 | Pure Python | 1 | 0.036 s | 166 MB | 99.897 % | 88.10 % | n/a |
| 10,000 | pandas + rapidfuzz | 1 | 0.024 s | 162 MB | 99.897 % | 88.10 % | n/a |
| 100,000 | remitmatch | 16 | 1.029 s | 382 MB | 99.995 % | 93.30 % | 100.000 % (72,271) |
| 100,000 | remitmatch | 1 | 1.065 s | 382 MB | 99.995 % | 93.30 % | 100.000 % (72,271) |
| 100,000 | Pure Python | 1 | 0.511 s | 386 MB | 99.747 % | 88.40 % | n/a |
| 100,000 | pandas + rapidfuzz | 1 | 0.180 s | 309 MB | 99.717 % | 88.40 % | n/a |
| 1,000,000 | remitmatch | 16 | 20.167 s | 2,390 MB | 99.996 % | 93.08 % | 100.000 % (721,732) |
| 1,000,000 | remitmatch | 1 | 20.275 s | 2,368 MB | 99.996 % | 93.08 % | 100.000 % (721,732) |
| 1,000,000 | Pure Python | 1 | 13.126 s | 2,624 MB | 99.393 % | 88.44 % | n/a |
| 1,000,000 | pandas + rapidfuzz | 1 | 2.625 s | 1,855 MB | 99.341 % | 88.43 % | n/a |

Time of each baseline divided by the time of `remitmatch` (above 1 means `remitmatch` is faster), with all threads and with one thread:

| Layout | Dataset | Payments | pure Python / remitmatch (all threads) | pandas / remitmatch (all threads) | pure Python / remitmatch (1 thread) | pandas / remitmatch (1 thread) | remitmatch 1 thread / all threads |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| four-scopes | one-to-one | 10,000 | 1.96 | 1.47 | 0.71 | 0.54 | 2.74 |
| four-scopes | one-to-one | 100,000 | 2.73 | 1.36 | 0.81 | 0.41 | 3.36 |
| four-scopes | one-to-one | 1,000,000 | 3.37 | 1.71 | 0.90 | 0.45 | 3.76 |
| four-scopes | mixed | 10,000 | 1.99 | 1.26 | 0.70 | 0.44 | 2.85 |
| four-scopes | mixed | 100,000 | 2.34 | 0.93 | 0.51 | 0.20 | 4.61 |
| four-scopes | mixed | 1,000,000 | 2.88 | 1.05 | 0.60 | 0.22 | 4.81 |
| one-scope | one-to-one | 10,000 | 0.65 | 0.54 | 0.63 | 0.52 | 1.04 |
| one-scope | one-to-one | 100,000 | 0.71 | 0.35 | 0.69 | 0.34 | 1.04 |
| one-scope | one-to-one | 1,000,000 | 0.81 | 0.31 | 0.78 | 0.30 | 1.04 |
| one-scope | mixed | 10,000 | 0.44 | 0.29 | 0.42 | 0.28 | 1.05 |
| one-scope | mixed | 100,000 | 0.50 | 0.18 | 0.48 | 0.17 | 1.03 |
| one-scope | mixed | 1,000,000 | 0.65 | 0.13 | 0.65 | 0.13 | 1.01 |

Everything measured, including the build time, the time of the `reconcile` call alone, the memory after loading and the time of every run:

| Layout | Dataset | Payments | Implementation | Threads | Time | reconcile call only | Build time | Memory after load | Peak memory | Precision | Recall | Precision of auto | Times of the runs |
| --- | --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| four-scopes | one-to-one | 10,000 | remitmatch | 16 | 0.016 s | 0.015 s | 0.027 s | 151 MB | 175 MB | 100.000 % | 94.81 % | 100.000 % | 0.017, 0.016, 0.017 |
| four-scopes | one-to-one | 10,000 | Pure Python | 1 | 0.031 s | n/a | 0.048 s | 158 MB | 165 MB | 100.000 % | 94.91 % | n/a | 0.031, 0.031, 0.031 |
| four-scopes | one-to-one | 10,000 | pandas + rapidfuzz | 1 | 0.023 s | n/a | 0.029 s | 153 MB | 164 MB | 100.000 % | 94.91 % | n/a | 0.031, 0.024, 0.023 |
| four-scopes | one-to-one | 10,000 | remitmatch | 1 | 0.043 s | 0.042 s | 0.028 s | 148 MB | 167 MB | 100.000 % | 94.81 % | 100.000 % | 0.044, 0.044, 0.043 |
| four-scopes | one-to-one | 100,000 | remitmatch | 16 | 0.132 s | 0.120 s | 0.033 s | 195 MB | 413 MB | 100.000 % | 94.79 % | 100.000 % | 0.134, 0.137, 0.132 |
| four-scopes | one-to-one | 100,000 | Pure Python | 1 | 0.360 s | n/a | 0.253 s | 307 MB | 402 MB | 99.999 % | 94.88 % | n/a | 0.379, 0.372, 0.360 |
| four-scopes | one-to-one | 100,000 | pandas + rapidfuzz | 1 | 0.179 s | n/a | 0.035 s | 199 MB | 306 MB | 99.999 % | 94.88 % | n/a | 0.179, 0.180, 0.181 |
| four-scopes | one-to-one | 100,000 | remitmatch | 1 | 0.442 s | 0.430 s | 0.034 s | 194 MB | 382 MB | 100.000 % | 94.79 % | 100.000 % | 0.442, 0.462, 0.451 |
| four-scopes | one-to-one | 1,000,000 | remitmatch | 16 | 1.445 s | 1.332 s | 0.086 s | 516 MB | 2,495 MB | 100.000 % | 94.80 % | 100.000 % | 1.445 |
| four-scopes | one-to-one | 1,000,000 | Pure Python | 1 | 4.869 s | n/a | 2.731 s | 1,764 MB | 2,726 MB | 99.999 % | 94.91 % | n/a | 4.869 |
| four-scopes | one-to-one | 1,000,000 | pandas + rapidfuzz | 1 | 2.475 s | n/a | 0.093 s | 558 MB | 1,647 MB | 99.999 % | 94.91 % | n/a | 2.475 |
| four-scopes | one-to-one | 1,000,000 | remitmatch | 1 | 5.439 s | 5.326 s | 0.082 s | 531 MB | 2,251 MB | 100.000 % | 94.80 % | 100.000 % | 5.439 |
| four-scopes | mixed | 10,000 | remitmatch | 16 | 0.018 s | 0.017 s | 0.029 s | 149 MB | 174 MB | 100.000 % | 94.90 % | 100.000 % | 0.020, 0.018, 0.018 |
| four-scopes | mixed | 10,000 | Pure Python | 1 | 0.036 s | n/a | 0.048 s | 158 MB | 165 MB | 100.000 % | 88.10 % | n/a | 0.038, 0.037, 0.036 |
| four-scopes | mixed | 10,000 | pandas + rapidfuzz | 1 | 0.023 s | n/a | 0.030 s | 152 MB | 165 MB | 100.000 % | 88.10 % | n/a | 0.024, 0.024, 0.023 |
| four-scopes | mixed | 10,000 | remitmatch | 1 | 0.052 s | 0.051 s | 0.027 s | 149 MB | 166 MB | 100.000 % | 94.90 % | 100.000 % | 0.053, 0.053, 0.052 |
| four-scopes | mixed | 100,000 | remitmatch | 16 | 0.190 s | 0.179 s | 0.035 s | 192 MB | 410 MB | 99.994 % | 94.87 % | 99.999 % | 0.190, 0.205, 0.191 |
| four-scopes | mixed | 100,000 | Pure Python | 1 | 0.446 s | n/a | 0.246 s | 306 MB | 390 MB | 99.957 % | 88.40 % | n/a | 0.448, 0.460, 0.446 |
| four-scopes | mixed | 100,000 | pandas + rapidfuzz | 1 | 0.177 s | n/a | 0.036 s | 198 MB | 294 MB | 99.953 % | 88.40 % | n/a | 0.181, 0.179, 0.177 |
| four-scopes | mixed | 100,000 | remitmatch | 1 | 0.878 s | 0.868 s | 0.034 s | 191 MB | 368 MB | 99.994 % | 94.87 % | 99.999 % | 0.878, 0.879, 0.881 |
| four-scopes | mixed | 1,000,000 | remitmatch | 16 | 2.214 s | 2.092 s | 0.086 s | 507 MB | 2,355 MB | 99.993 % | 93.88 % | 100.000 % | 2.214 |
| four-scopes | mixed | 1,000,000 | Pure Python | 1 | 6.385 s | n/a | 2.615 s | 1,704 MB | 2,613 MB | 99.924 % | 88.45 % | n/a | 6.385 |
| four-scopes | mixed | 1,000,000 | pandas + rapidfuzz | 1 | 2.331 s | n/a | 0.100 s | 549 MB | 1,437 MB | 99.917 % | 88.45 % | n/a | 2.331 |
| four-scopes | mixed | 1,000,000 | remitmatch | 1 | 10.655 s | 10.544 s | 0.083 s | 520 MB | 2,141 MB | 99.993 % | 93.88 % | 100.000 % | 10.655 |
| one-scope | one-to-one | 10,000 | remitmatch | 16 | 0.042 s | 0.040 s | 0.027 s | 149 MB | 174 MB | 100.000 % | 94.34 % | 100.000 % | 0.042, 0.042, 0.042 |
| one-scope | one-to-one | 10,000 | Pure Python | 1 | 0.027 s | n/a | 0.047 s | 158 MB | 166 MB | 100.000 % | 94.91 % | n/a | 0.027, 0.030, 0.031 |
| one-scope | one-to-one | 10,000 | pandas + rapidfuzz | 1 | 0.023 s | n/a | 0.029 s | 150 MB | 163 MB | 100.000 % | 94.91 % | n/a | 0.024, 0.023, 0.023 |
| one-scope | one-to-one | 10,000 | remitmatch | 1 | 0.043 s | 0.042 s | 0.028 s | 148 MB | 172 MB | 100.000 % | 94.34 % | 100.000 % | 0.043, 0.045, 0.047 |
| one-scope | one-to-one | 100,000 | remitmatch | 16 | 0.516 s | 0.505 s | 0.035 s | 194 MB | 387 MB | 100.000 % | 94.19 % | 100.000 % | 0.603, 0.524, 0.516 |
| one-scope | one-to-one | 100,000 | Pure Python | 1 | 0.368 s | n/a | 0.255 s | 309 MB | 403 MB | 99.997 % | 94.88 % | n/a | 0.380, 0.368, 0.406 |
| one-scope | one-to-one | 100,000 | pandas + rapidfuzz | 1 | 0.181 s | n/a | 0.036 s | 196 MB | 302 MB | 99.997 % | 94.88 % | n/a | 0.183, 0.181, 0.184 |
| one-scope | one-to-one | 100,000 | remitmatch | 1 | 0.537 s | 0.526 s | 0.034 s | 193 MB | 386 MB | 100.000 % | 94.19 % | 100.000 % | 0.537, 0.561, 0.559 |
| one-scope | one-to-one | 1,000,000 | remitmatch | 16 | 8.030 s | 7.924 s | 0.085 s | 523 MB | 2,461 MB | 100.000 % | 94.07 % | 100.000 % | 8.030 |
| one-scope | one-to-one | 1,000,000 | Pure Python | 1 | 6.540 s | n/a | 2.718 s | 1,758 MB | 2,721 MB | 99.990 % | 94.90 % | n/a | 6.540 |
| one-scope | one-to-one | 1,000,000 | pandas + rapidfuzz | 1 | 2.502 s | n/a | 0.097 s | 562 MB | 1,701 MB | 99.989 % | 94.90 % | n/a | 2.502 |
| one-scope | one-to-one | 1,000,000 | remitmatch | 1 | 8.363 s | 8.244 s | 0.085 s | 524 MB | 2,455 MB | 100.000 % | 94.07 % | 100.000 % | 8.363 |
| one-scope | mixed | 10,000 | remitmatch | 16 | 0.081 s | 0.080 s | 0.029 s | 149 MB | 171 MB | 100.000 % | 94.44 % | 100.000 % | 0.082, 0.082, 0.081 |
| one-scope | mixed | 10,000 | Pure Python | 1 | 0.036 s | n/a | 0.049 s | 159 MB | 166 MB | 99.897 % | 88.10 % | n/a | 0.039, 0.036, 0.038 |
| one-scope | mixed | 10,000 | pandas + rapidfuzz | 1 | 0.024 s | n/a | 0.029 s | 150 MB | 162 MB | 99.897 % | 88.10 % | n/a | 0.024, 0.024, 0.025 |
| one-scope | mixed | 10,000 | remitmatch | 1 | 0.086 s | 0.085 s | 0.028 s | 148 MB | 169 MB | 100.000 % | 94.44 % | 100.000 % | 0.086, 0.088, 0.086 |
| one-scope | mixed | 100,000 | remitmatch | 16 | 1.029 s | 1.019 s | 0.034 s | 195 MB | 382 MB | 99.995 % | 93.30 % | 100.000 % | 1.031, 1.030, 1.029 |
| one-scope | mixed | 100,000 | Pure Python | 1 | 0.511 s | n/a | 0.240 s | 302 MB | 386 MB | 99.747 % | 88.40 % | n/a | 0.522, 0.511, 0.532 |
| one-scope | mixed | 100,000 | pandas + rapidfuzz | 1 | 0.180 s | n/a | 0.036 s | 199 MB | 309 MB | 99.717 % | 88.40 % | n/a | 0.180, 0.186, 0.185 |
| one-scope | mixed | 100,000 | remitmatch | 1 | 1.065 s | 1.055 s | 0.034 s | 195 MB | 382 MB | 99.995 % | 93.30 % | 100.000 % | 1.065, 1.082, 1.066 |
| one-scope | mixed | 1,000,000 | remitmatch | 16 | 20.167 s | 20.056 s | 0.085 s | 531 MB | 2,390 MB | 99.996 % | 93.08 % | 100.000 % | 20.167 |
| one-scope | mixed | 1,000,000 | Pure Python | 1 | 13.126 s | n/a | 2.583 s | 1,701 MB | 2,624 MB | 99.393 % | 88.44 % | n/a | 13.126 |
| one-scope | mixed | 1,000,000 | pandas + rapidfuzz | 1 | 2.625 s | n/a | 0.094 s | 530 MB | 1,855 MB | 99.341 % | 88.43 % | n/a | 2.625 |
| one-scope | mixed | 1,000,000 | remitmatch | 1 | 20.275 s | 20.167 s | 0.087 s | 515 MB | 2,368 MB | 99.996 % | 93.08 % | 100.000 % | 20.275 |

Build time is the time to read the Parquet file into the native form of each implementation. It is not part of the timed call, but it is a real cost for the pure Python baseline (2.7 s at 1,000,000 payments, against 0.10 s for Arrow tables).

## The implementations

- **`remitmatch`**: `reconcile(invoices, payments, MatchConfig(amount_tolerance_minor=300))` with both inputs as Arrow tables, default thresholds and weights. Every proposed match counts (`auto`, `suggested` and `ambiguous`), and the tables also give the precision of the `auto` matches alone. It finds one-to-one, one-to-many and many-to-one matches and returns a per-signal explanation of each; none of that is used in the one-to-one comparison. It runs on all threads, or on one in the `remitmatch-1-thread` entry.
- **Pure Python** ([`baselines/pure_python.py`](baselines/pure_python.py)), standard library only, one thread. Exact pass: invoice-like numbers (the prefix `INV` followed by digits, after removing spaces, dashes and slashes) are extracted from the reference and looked up in a dictionary; the pair is kept when scope and currency agree and the amounts differ by at most the tolerance. Fuzzy pass, for payments that mention such a number but found no exact match: candidates come from an amount index (`bisect`) inside the 30-day date window, and each is scored with the four signals of `remitmatch` (reference, amount, payer name, date; the same default weights and the same 0.60 threshold) using `difflib`. Greedy assignment, best score first.
- **pandas + rapidfuzz** ([`baselines/pandas_rapidfuzz.py`](baselines/pandas_rapidfuzz.py)), one thread: the same two passes, vectorized. References and numbers are normalized and the invoice-like numbers are extracted with `pyarrow.compute` (the few references that name several numbers are split with `re`), then merged with the invoices. The fuzzy pass builds the candidates with an amount-window join (sorted composite keys and `numpy.searchsorted`, no row loop), cuts them by the date window, and scores the reference and the payer name with `rapidfuzz.process.cpdist`. The greedy assignment is vectorized by repeatedly accepting the pairs that are the best choice of both their payment and their invoice, which gives the same result as sorting and accepting one by one.

What the baselines do not do:

- They match one payment to one invoice. They cannot find a payment that settles several invoices or several payments that settle one invoice, which is why they lose recall on the `mixed` dataset.
- They never flag a match as ambiguous and give no per-signal explanation.
- They assume the invoice numbers start with `INV` (the constant `INVOICE_PREFIX` in each module), which makes the exact pass a single regular expression. `remitmatch` does not need to know the prefix: it searches the references for the open invoice numbers it was given. With another prefix, for example `FAC`, both baselines find nothing (a test checks it). The choice favours the baselines. *Exploratory measurement, not in the results file:* a regular expression that accepts any three capital letters as a prefix made pure Python about 36 % slower and pandas + rapidfuzz about 3 % slower at 100,000 payments one-to-one, and made no measurable difference at 1,000,000 payments, with the same matches.
- They were written for this benchmark, by the author of `remitmatch`.

## Fairness notes

- The baselines were written to be fast, not to be slow. They run on one thread, and `remitmatch` on all threads is the default; the one-thread entry and the one-scope layout are there to show how much of the result is parallelism.
- Their precision is within 0.2 points of `remitmatch` on one-to-one data, and their recall is slightly higher there (94.88 to 94.91 % against 94.79 to 94.81 % with four scopes), so on this data they do the same job.
- The data comes from `generate.py`, a Python generator that mirrors the noise model of the generator used by the accuracy tests of `remitmatch`. The default weights of `remitmatch` have not been tuned on either. Real bank statements are different, so these numbers say nothing about accuracy in production.
- The data has references that contain the invoice number, which is what makes an exact dictionary lookup so effective. Data where references are noisier would cost the baselines more than the engine.
- Disabling the garbage collector favours the baselines, because they build many Python objects. *Exploratory, not in the results file, measured on an earlier version of the baselines:* with the collector on and the input already in memory, pure Python took about three times longer at 1,000,000 payments.
- The 1,000,000 payment case is a single run. The three runs at 10,000 and 100,000 payments differ by up to 17 percent from the best one, and by 35 percent in one case (pandas + rapidfuzz at 10,000 payments, four scopes, one-to-one), which the machine load explains in part (the file has every run).
- One machine, shared with other work. The numbers will differ elsewhere, and the ratios between implementations will too.
- The `remitmatch` memory is higher than that of pandas + rapidfuzz at every size. It has not been analysed further in this slice.

### What was tried to make the baselines faster

*Exploratory, not in the results file.*

- pandas + rapidfuzz: normalizing the references and extracting the numbers with `pyarrow.compute`, and factorizing scope and currency separately instead of concatenating strings, made the baseline 1.37 to 1.55 times faster than the version that used the pandas string methods, with identical output (compared on 10,000, 100,000 and 1,000,000 payments, both datasets, four scopes). The faster version is the one measured here. Scoring with all threads (`workers=-1`) instead of one changed the total by 2 % on the earlier version, because `cpdist` took 0.01 s of it, so the default was kept.
- Pure Python: an upper-bound check that skips candidates which cannot reach the threshold before running `difflib` made no measurable difference at 1,000,000 payments (4.21 s with and without), so it was removed. Names are normalized lazily and once per distinct name, and `SequenceMatcher` objects keep the sequence they index fixed per payment.

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

Needs a Rust toolchain and [uv](https://docs.astral.sh/uv/). Run the commands from the repository root. The `--no-sync` flags keep `uv` from rebuilding the extension in debug mode after the release build.

```bash
uv sync --group bench
uv run --no-sync maturin develop --release
uv run --no-sync python benchmarks/run.py --remitmatch-build release --commit "$(git rev-parse --short HEAD)"
uv run --no-sync python benchmarks/plot.py
uv run --no-sync maturin develop          # back to the debug build
```

`run.py` takes `--sizes`, `--implementations`, `--datasets`, `--layouts`, `--scopes`, `--seed`, `--runs`, `--timeout`, `--output`, `--remitmatch-build`, `--commit` and `--note`. For example, a quick run:

```bash
uv run --no-sync python benchmarks/run.py --sizes 10000 --layouts one-scope --output /tmp/results.json
```

The full run takes about half an hour on the machine above. The results file records the machine, the operating system, the versions of Python and of every package, the seeds, the layouts, the tolerance, the build profile and commit declared for `remitmatch`, the threads of every run and the load average. Run the tests of the generator, the baselines and the runner with `uv run --group bench pytest benchmarks`; they are skipped when the `bench` dependency group is not installed.
