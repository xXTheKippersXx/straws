# nec2c in straws

This folder holds **nec2c 1.3.1** by Neoklis Kyriazis (5B4AZ), a C translation of the
NEC-2 FORTRAN code. Neoklis put nec2c in the public domain ("nec2c is Public Domain, same as
the original FORTRAN source"; see `README`, section 7) and asked that software built on it
stay open. straws is GPL-3.0-or-later.

## Where it came from

The upstream tarball as packaged by Debian:

| File | SHA-256 |
|---|---|
| `nec2c_1.3.1.orig.tar.bz2` | `271bd2ea6037f896912139a66e3abb75981c72315bab5221fcf384a1736d0337` |

Downloaded from `https://deb.debian.org/debian/pool/main/n/nec2c/` on 2026-10-18; the checksum
matches Debian's `nec2c_1.3.1-3.dsc`. Debian's only patch is to the build system, which we
don't use. Build files (autotools) and the GPL `COPYING` file that came in the tarball are not
copied here; `README`, `AUTHORS`, `ChangeLog` and `NEC2-bug.txt` are kept as they were.

## Changes from upstream

Every change to the C source is marked with a `straws:` comment.

1. **`src/main.c`, signal handling.** Windows has no `sigaction()`, so on Windows the same
   handlers are registered with `signal()`. Other platforms are unchanged.
2. **`src/main.c`, buffer overflow fix.** `main()` read comment cards with `load_line()`, which
   fills up to `LINE_LEN` (132) characters, into an 81-byte buffer, so a long `CM` line could
   overwrite memory. The buffer is now `LINE_LEN + 2` bytes.
3. **`src/calculations.c`, `zint()`: thin-wire losses.** The FORTRAN returns after each of its
   three formulas (thin, medium and thick wires, by skin depth). The C had no `return`s, so every
   wire fell through to the thick-wire formula and `LD 5` losses were wrong for thin wires, low
   frequencies and poor conductors. Found by validating against 4nec2's engine.
4. **`src/somnec.c`, `evlua()`: Sommerfeld contour.** One branch skipped both tail integrations,
   leaving the Sommerfeld integral incomplete (up to 5% impedance error on 80 m beams over real
   ground). Restored the FORTRAN's decision order. Found and fixed in
   [xnec2c](https://github.com/KJ7LNW/xnec2c) (commit 17b272d).
5. **`src/somnec.c`, `gshank()`: convergence test.** A misplaced bracket
   (`fabs(creal(a1)+fabs(cimag(a1)))`) could pass the convergence test too early. Found and fixed
   in xnec2c (commit 5d49608).

6. **`src/matrix.c`, fast solver.** `factr()` and `solve()` call `nec_factr()`/`nec_solve()`
   in `../src/solver.rs` (faer's multi-threaded LU). The originals are kept as
   `factr_classic()`/`solve_classic()`; `NEC_SOLVER=classic` selects them.
7. **`src/matrix.c` and friends, parallel matrix fill.** `cmset()` splits the observation
   segments into blocks filled on separate threads (`fill_part()`, threads started from Rust).
   The scratch globals the fill uses (`dataj`, `segj`, `incom`, `gwav`, `tmi`, `tmh`, and
   `intrp()`'s cache) are `_Thread_local` (`NEC_TLS` in `shared.h`). After the threads finish,
   the calling thread runs `trio()` over every segment, because `tbf()` (used later for currents)
   writes into `segj`'s buffers assuming a sequential fill has already grown them; missing this
   caused heap corruption on grids with big junctions. Threads per worker: `NEC_SOLVER_THREADS`.

8. **`src/radiation.c`, parallel far-field pattern.** `rdpat()` first computes the field at
   every pattern point on several threads (`pattern_part()`; `ffld()` only reads shared data),
   then its original loop prints and accumulates from those values. Patterns under 256 points
   stay on one thread.
9. **`src/geometry.c`, `datagn()`: patch strips.** NEC lets more `SC` cards follow an `SP` card,
   each adding a patch joined to the one before (a strip). The Fortran clears the "an SC may
   follow" flag on any card except `ATST(10)`, 1-based, which is `SC`; the C kept the 10 with its
   0-based table, so it tested for `GH` and refused every strip with "PATCH DATA ERROR". Found by
   validating 4nec2's patch models (coffee-can feeds, W2IMU horn).
10. **`src/somnec.c`, `somnec()`: a cache of Sommerfeld tables.** The tables depend only on the
   ground and the frequency, and working them out takes about 13 ms a frequency (85% of a small
   model's run over real ground). When `STRAWS_SOMNEC_CACHE` names a folder, each frequency's
   tables are written there (`som-<bits of εr>-<σ>-<MHz>.bin`, about 17 KB, tagged and keyed by
   the exact inputs) and later runs read them back; the original code is `somnec_work()`. The
   output is bit-for-bit the same (test `cached_sommerfeld_tables_change_nothing`). The engine
   points workers at `straws-sommerfeld` in the temp folder and trims it to 32 MB, oldest first.
11. **`src/radiation.c`, `rdpat()`: binary far-field patterns.** When `STRAWS_BINARY_PATTERN`
   names a file, each far-field point is written there as 12 doubles (θ, φ, vertical, horizontal
   and total gain, axial ratio, tilt, sense 0-3, |Eθ|, ∠Eθ, |Eφ|, ∠Eφ) instead of a text line,
   and the table ends with `STRAWS BINARY PATTERN n POINTS`. Printing and re-reading the text took
   about as long as working the pattern out, and the text keeps only 0.01 dB. Ground-wave
   (`RP 1`) patterns stay text. `Engine::run` asks for it; `run_text` (validation against 4nec2)
   doesn't. Test `binary_patterns_match_the_text`.
12. **Persistent workers (`main.c`, `misc.c`, `calculations.c`, `radiation.c`, `nec2c.h`).** A
   worker can run deck after deck in one process: `straws_run()` runs one (stderr to the job's
   `stderr.txt`, the binary pattern path for that run) and returns its exit code; `stop()`
   `longjmp`s back to it instead of calling `exit()` (as do `main()`'s argument and file errors,
   which now call `stop()`); `straws_reset()` frees every global buffer and zeroes every global
   struct, as a fresh process has them, and rewinds `getopt`. `main()`'s working buffers are
   `static` and freed at the start of the next run (so a run that stops with an error doesn't
   leak them). Per-run caches restart by `straws_run_count`: `intrp()`'s interpolation cache and
   `rdpat()`'s binary file (closed by `rdpat_finish()` so it's complete when read). A run's output
   is bit-for-bit what a fresh process writes (test `persistent_workers_match_fresh_processes`);
   memory stays flat over hundreds of runs (`persistent_worker_memory`). The signal handlers
   still exit: a crash ends the worker and the engine starts another.
13. **`src/main.c`: binary currents.** With a binary file (see item 11), the full current table
   (the default `PT` printing) goes there as 7 doubles a segment (segment, tag, centre x/y/z in
   wavelengths, real and imaginary current; `straws_current_record()`), marked in the text by
   `STRAWS BINARY CURRENTS n SEGMENTS`. The short receiving-pattern forms stay text.

Not changed in the C, but handled by the deck reader: a second field on the `GE` card made nec2c
write to a plot file that wasn't open and crash, so the reader drops that field (it isn't part of
NEC-2).

Build-time changes (in `../build.rs`, no source edits):

- `main` is renamed to `nec2c_main` (`-Dmain=nec2c_main`) so a worker process can call it.
- The engine is always compiled with optimisation, even in debug builds of the app.
- On Windows it is compiled with clang-cl, because Microsoft's C compiler has no C99 complex
  numbers. `compat/win/` supplies what the Windows C runtime lacks: `<complex.h>` (written for
  clang's native `_Complex` type), `<unistd.h>` and `<sys/times.h>` stand-ins, `getopt()`, and
  the complex multiply/divide helpers clang calls. `_CRT_DECLARE_NONSTDC_NAMES=0` stops the
  Windows headers defining `min`/`max` macros that clash with nec2c's own `min()`.

## Why it runs in a separate process

nec2c keeps its working data in global and `static` variables and calls `exit()` on any input
error, so it can't run two solves at once in one process and must not run inside the app. Every
solve runs in a short-lived worker process (see `../src/worker.rs` and `../src/engine.rs`).

## Known behaviour worth remembering

- A wire with zero length makes nec2c run forever instead of reporting an error. The deck reader
  rejects zero-length wires before they reach the engine.
- nec2c prints results to five significant figures (4nec2's NEC-2D engines print six) and gains
  to 0.01 dB. Reading results straight from memory instead of from the text output would give
  full precision; that's a candidate for later.
- No Numerical Green's Function support (`GF`/`WG` cards); the deck reader says so clearly.
