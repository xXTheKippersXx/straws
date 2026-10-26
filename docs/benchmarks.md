# Benchmarks

Speed is the project's top priority, so it's measured, not guessed. Re-run this after any
performance work and add a new dated section; keep the old ones for comparison.

## How to run

```
cargo build --release
target/release/straws-cli bench benchmarks --engine-dir C:\4nec2\exe --repeat 3
```

The decks in `benchmarks/` are made by `benchmarks/generate.py`. Each measurement is the median of
`--repeat` runs, wall-clock, including starting the engine and reading its results:

- **nec2c, 1 process**: the whole deck in one nec2c worker, plus parsing the output.
- **nec2c, split sweep**: the same deck through `Engine::sweep`, which splits the frequency sweep
  across worker processes (up to one per thread) when the estimated work makes it worthwhile.
- **4nec2 engine**: 4nec2's `nec2dxs` build for that size, run directly on the same deck. This is
  only the engine; 4nec2 itself adds its own time on top (writing the deck, reading results,
  redrawing), so the real gap to 4nec2 is larger than shown.
- **Speed-up**: 4nec2 engine time divided by the faster nec2c time.


## 2026-10-28: testing protocol run 2 (persistent workers, binary results)

Release build, dev machine, nothing else running, `--repeat 3`. Every row is at or better than the 2026-10-26 figures.

| Deck | Segments | Freqs | nec2c, 1 process (ms) | nec2c, split sweep (ms) | 4nec2 engine (ms) | Speed-up |
|---|---:|---:|---:|---:|---:|---:|
| 01 small dipole sweep, free space | 21 | 101 | 29.5 | 22.5 | 150 | 6.7× |
| 02 small dipole sweep, Sommerfeld ground | 21 | 101 | 59.0 | 54.2 | 7,460 | 138× |
| 03 7-element Yagi sweep, free space | 315 | 51 | 378 | 204 | 9,195 | 45× |
| 04 7-element Yagi sweep, Sommerfeld ground | 315 | 51 | 876 | 552 | 19,783 | 36× |
| 05 7-element Yagi, 1° full-sphere pattern | 315 | 1 | 344 | 124 | 8,485 | 69× |
| 06 wire grid + dipole, free space | 1,533 | 3 | 453 | 424 | 15,350 | 36× |
| 07 wire grid + dipole, Sommerfeld ground | 1,533 | 1 | 348 | 370 | 8,435 | 24× |
| 08 large wire grid + dipole, free space | 2,985 | 1 | 756 | 761 | 26,910 | 36× |

Also: a trivial engine run 2.4-3.2 ms; a slider update (315-segment Yagi, Sommerfeld, 5° pattern) 32-36 ms; the optimiser 84 / 140 designs a second (fine-tuning / whole range); the app's memory settles at about 180 MB and stays flat over 45 slider sweeps, and no worker processes are left after closing. The Feeds view's designers: phasing lines 26 ms, tuned over 51 frequencies 37 ms, exact L networks 26 ms, a hybrid 7 ms (290 ms tuned over a sweep, so it's held while a slider is dragged).

## 2026-10-26: phase 7, persistent workers

Workers now stay running and take job after job (`worker::SERVER_FLAG`; nec2c jumps back
instead of exiting and resets its globals; see `crates/straws-core/nec2c/README.straws.md`
item 12), so a run no longer pays for starting a process:

| | New process each run (ms) | Persistent workers (ms) |
|---|---:|---:|
| A trivial run through the engine | 16 | 3.1 |
| Slider update, 315-segment Yagi over Sommerfeld ground | 47 | 34-40 |
| Geometry-only run of that Yagi | 24 | 12 |
| Optimiser (`optimiser_speed`), fine-tuning / whole range, designs a second | 55 / 89 | 82 / 126 |

The benchmark set (medians of 3, nec2c one process / split / 4nec2's engine, ms): 01 31 / 31 /
124; 02 65 / 65 / 6,509; 03 400 / 241 / 6,313; 04 965 / 621 / 15,555; 05 344 / 133 / 6,411;
06 495 / 450 / 13,409; 07 365 / 365 / 7,236; 08 816 / 832 / 25,667. Small and split runs gain
most; big single solves are unchanged (the solve itself dominates).

Over this phase so far the optimiser went from 21 / 34 to 82 / 126 designs a second.

## 2026-10-26: phase 7, workers started without a console

Every engine run started its worker with `CREATE_NO_WINDOW`, and Windows still made a hidden
console (and a conhost.exe process) for each one. Starting workers with `DETACHED_PROCESS` (no
console at all; their output goes to files anyway) saves about 13 ms a run:

| | Before (ms) | Now (ms) |
|---|---:|---:|
| A trivial run through the engine (start, solve nothing, read back, clean up) | 30 | 16.5 |
| Slider update, 315-segment Yagi over Sommerfeld ground (1 frequency, 5° pattern) | 65 | 51-60 |
| Geometry-only run of that Yagi | 38 | 24-29 |
| Optimiser (`optimiser_speed`), fine-tuning / whole range, designs a second | 41 / 62 | 49 / 72 |

Then job folders were reused instead of made and deleted for every run (that cost about 9 ms on
Windows): slider update 46-47 ms, geometry run 23-25 ms, optimiser 55 / 89 designs a second.

(`slider_update_speed`, an ignored test, measures the slider path.) Correction to an earlier
note: a worker run costs about 9.5 ms when started from an existing console, not "8 ms of
start-up"; the 2026-10-25 PowerShell figure had mangled arguments and measured an error exit.
What's left per run (about 16 ms) is mostly process creation and the job folder (a fresh
folder costs about 4.5 ms more than reusing one); persistent workers would remove most of it.

## 2026-10-26: phase 7, far-field patterns handed over in binary

The app's sweeps ask for a 5° full-sphere pattern at every frequency (2,701 points each). For
the 7-element Yagi (row 03's deck with that pattern, 51 frequencies, one process) the pattern
added about 820 ms: roughly 440 ms working it out, 380 ms nec2c printing 138,000 lines (18 MB)
and 125 ms reading them back. Patterns now go to a binary file (`Engine::run`; the benchmark
table below still times the text path, as 4nec2 has no other):

| Yagi sweep, 51 frequencies, 5° pattern each | Before (ms) | Now (ms) |
|---|---:|---:|
| one process | 1,220-1,480 | 710-740 |
| split across processes | | 533 |

Gains are now full precision (text: 0.01 dB).

## 2026-10-25: phase 7, Sommerfeld tables cached between runs

Same machine. Working out NEC's Sommerfeld ground tables took about 13 ms a frequency, 85% of
a small model's run over real ground, and they depend only on the ground and the frequency.
Workers now keep them in files (`straws-sommerfeld` in the temp folder, 17 KB a frequency,
trimmed to 32 MB) and later runs read them back: output identical, see
`crates/straws-core/nec2c/README.straws.md` item 10. Medians of 3 runs, so the tables were
cached for 2 of them (a first sweep at new frequencies still works them out). The app
remembers which grounds and frequencies are cached and doesn't split those sweeps across
processes (the split no longer pays). The 4nec2 engine timings vary from day to day on this
laptop (row 02: 6.8 s today, 9.0 s before).

| Deck | Segments | Freqs | nec2c, 1 process (ms) | nec2c, split sweep (ms) | 4nec2 engine (ms) | Speed-up | Before (1 process / split, ms) |
|---|---:|---:|---:|---:|---:|---:|---:|
| 01 small dipole sweep, free space | 21 | 101 | 54 | 55 | 110 | 2.0× | 51 / 50 |
| 02 small dipole sweep, Sommerfeld ground | 21 | 101 | 88 | 94 | 6,834 | 78× | 1,438 / 316 |
| 03 7-element Yagi sweep, free space | 315 | 51 | 406 | 321 | 6,196 | 19× | 466 / 297 |
| 04 7-element Yagi sweep, Sommerfeld ground | 315 | 51 | 956 | 682 | 15,491 | 23× | 1,670 / 809 |
| 05 7-element Yagi, 1° full-sphere pattern | 315 | 1 | 404 | 366 | 6,418 | 18× | 375 |
| 06 wire grid + dipole, free space | 1,533 | 3 | 507 | 496 | 13,218 | 27× | 575 / 480 |
| 07 wire grid + dipole, Sommerfeld ground | 1,533 | 1 | 389 | 396 | 7,227 | 19× | 404 / 398 |
| 08 large wire grid + dipole, free space | 2,985 | 1 | 842 | 831 | 25,573 | 31× | 835 / 821 |

Optimiser (`optimiser_speed`, the 55-segment Yagi over Sommerfeld ground, 7 frequencies a
design): fine-tuning 21 → 41 designs a second, whole-range search 34 → 62.

Big models: the double biquad demo (1,995 segments, free space) takes about 90 ms to fill and
130 ms to factorise per frequency, close to this CPU's peak arithmetic rate, so further gains
there need fewer solves (e.g. reusing the factorised reflector block while only the element
changes) rather than faster ones. (The worker start-up figure measured that day was wrong; see
2026-10-26.)

## 2026-10-18 (later): phase 2.5, fast solver and parallel matrix fill

Same machine. nec2c's matrix factorisation now uses faer's blocked, multi-threaded LU
(`crates/straws-core/src/solver.rs`), and the wire part of the matrix fill runs on all cores
(`cmset()` in `nec2c/src/matrix.c`). Parallel sweeps share the cores between their workers.
4nec2 figures are from the baseline run below (its engine hasn't changed).

| Deck | Segments | Freqs | nec2c, 1 process (ms) | nec2c, split sweep (ms) | 4nec2 engine (ms) | Speed-up | Baseline best (ms) |
|---|---:|---:|---:|---:|---:|---:|---:|
| 01 small dipole sweep, free space | 21 | 101 | 51 | 50 | 216 | 4.3× | 53 |
| 02 small dipole sweep, Sommerfeld ground | 21 | 101 | 1,438 | 316 | 8,984 | 28× | 345 |
| 03 7-element Yagi sweep, free space | 315 | 51 | 466 | 297 | 12,784 | 43× | 411 |
| 04 7-element Yagi sweep, Sommerfeld ground | 315 | 51 | 1,670 | 809 | 26,233 | 32× | 1,048 |
| 05 7-element Yagi, 1° full-sphere pattern | 315 | 1 | 761 | 765 | 16,933 | 22× | 774 |
| 06 wire grid + dipole, free space | 1,533 | 3 | 575 | 480 | 20,120 | 42× | 3,157 |
| 07 wire grid + dipole, Sommerfeld ground | 1,533 | 1 | 404 | 398 | 10,502 | 26× | 3,673 |
| 08 large wire grid + dipole, free space | 2,985 | 1 | 835 | 821 | 30,500 | 37× | 14,609 |

Inside one 2,985-segment solve: factorisation 12.1 s → 0.41 s, matrix fill 1.7 s → 0.26 s.
Accuracy is unchanged: the 19 regression decks and 710 of 4nec2's models still match 4nec2's
engine. nec2c's original solver is still there for comparison (`NEC_SOLVER=classic`).

Afterwards the far-field pattern was parallelised too: row 05 went from 761 ms to 375 ms. Most of
what's left there is writing and re-reading the 65,341-point pattern as text; reading results
straight from memory is the next step. Sommerfeld ground set-up at each frequency (about 14 ms
each, row 02) is the other remaining cost.

## 2026-10-18: phase 1 baseline

Dev machine: Lenovo IdeaPad Slim 3, Ryzen 5 7530U (6 cores / 12 threads), 16 GB, Windows 11.
nec2c 1.3.1 with this project's fixes, compiled with clang-cl 19 at `-O2`. Mains power.

| Deck | Segments | Freqs | nec2c, 1 process (ms) | nec2c, split sweep (ms) | 4nec2 engine (ms) | Speed-up |
|---|---:|---:|---:|---:|---:|---:|
| 01 small dipole sweep, free space | 21 | 101 | 53 | 55 | 216 | 4.1× |
| 02 small dipole sweep, Sommerfeld ground | 21 | 101 | 1,458 | 345 | 8,984 | 26× |
| 03 7-element Yagi sweep, free space | 315 | 51 | 1,601 | 411 | 12,784 | 31× |
| 04 7-element Yagi sweep, Sommerfeld ground | 315 | 51 | 4,648 | 1,048 | 26,233 | 25× |
| 05 7-element Yagi, 1° full-sphere pattern | 315 | 1 | 812 | 774 | 16,933 | 22× |
| 06 wire grid + dipole, free space | 1,533 | 3 | 7,256 | 3,157 | 20,120 | 6.4× |
| 07 wire grid + dipole, Sommerfeld ground | 1,533 | 1 | 3,675 | 3,673 | 10,502 | 2.9× |
| 08 large wire grid + dipole, free space | 2,985 | 1 | 14,646 | 14,609 | 30,500 | 2.1× |

Row 01's split-sweep figure is from a re-run after adding the "is splitting worth it?" rule; before
the rule it split into 12 processes and took 110 ms. The other rows are from the first run (the
re-run gave the same within run-to-run noise).

### What this shows

- **Sweeps are 25–31× faster** than 4nec2's engine on everyday models, from nec2c being quicker
  per frequency and from spreading the sweep over all cores.
- **Tiny models**: starting a worker process costs about 15–25 ms on Windows, so a sweep only
  splits when each process gets at least about 60 ms of estimated work. The estimate
  (`engine::estimated_run_ms`) is fitted to this table: per frequency about
  0.00013·N² + 6×10⁻⁷·N³ ms for N unknowns, plus 14 + 0.00046·N² ms over Sommerfeld ground.
- **Big single-frequency models** (rows 07, 08) are only 2–3× faster: one matrix solve (∝ N³) in
  one process dominates, and nec2c's solver is plain textbook code.
- **Patterns**: in row 05 the 65,341-point pattern takes about 0.75 s of the 0.8 s; the solve is
  about 30 ms.

### Next speed-ups, in order of expected payoff

1. ~~Faster matrix solve~~ (done in phase 2.5).
2. ~~Parallel matrix fill~~ (done in phase 2.5).
3. **Patterns**: compute pattern points in parallel and read results straight from memory
   instead of writing and parsing text (row 05, and the live 3D pattern view).
4. **Persistent workers** that stay running between solves, removing the 15–25 ms start-up per
   job. This matters most for the live sliders, where every edit is a fresh small solve.
5. ~~Reuse Sommerfeld ground tables~~ between runs at the same frequency and ground (done
   2026-10-25).
