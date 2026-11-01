# STRAWS in a browser

The same app as the desktop version, compiled to WebAssembly, at
<https://xxthekippersxx.github.io/straws/>. Nothing to install: it runs in
any recent Chrome, Edge, Firefox or Safari (it needs WebGPU or WebGL 2).
Files never leave the computer: opening one reads it into the page, and
saving one downloads it.

## What's in it

Everything that doesn't need the desktop: building and editing models (3D
drawing, wire table, deck text, variables and sliders, templates and the
design library), sweeps, the frequency slider, 3D patterns and cuts, SWR,
Smith chart, matching, feedlines, feeds and arrays, near fields, comparison
snapshots, projects, imports and exports.

Not yet (they say "Needs the desktop app"): the optimiser, the convergence,
tolerance and sensitivity checks, and the estimate for radials on or under
the ground. They run thousands of engine solves, or several in a row, from
code that waits for each result; a browser page can't wait like that.

It's slower than the desktop app (the engine runs about three times
slower in WebAssembly, and without the desktop's parallel solver), but
small and medium models still sweep in well under a second.

## How it works

- The app (`crates/straws-gui`) builds for `wasm32-unknown-unknown` with
  eframe's web runner. `crates/straws-gui/src/files.rs` turns file dialogs
  into uploads and downloads; `compute_web.rs` and `web.rs` run engine jobs.
- The engine is nec2c with its Rust solver, built as a WASI program for
  `wasm32-wasip1` (the `straws-worker` binary, compiled with wasi-sdk's
  clang). `engine-worker.js` runs it in Web Workers, the browser's version
  of the desktop's worker processes: a fresh instance per deck, so nec2c's
  global state and `exit()` calls stay contained, with a small in-memory
  file system for its files. Sweeps are split across the workers (one per
  core, up to 8).
- Sommerfeld ground tables, which the desktop caches in files, are kept in
  each worker's memory and passed on to the others, so a sweep run again
  after an edit skips working them out.

## Building

Needs the Rust targets `wasm32-unknown-unknown` and `wasm32-wasip1`,
[trunk](https://trunkrs.dev/) and [wasi-sdk](https://github.com/WebAssembly/wasi-sdk)
(set `WASI_SDK_PATH` to its folder):

```bash
rustup target add wasm32-unknown-unknown wasm32-wasip1
WASI_SDK_PATH=/path/to/wasi-sdk bash web/build.sh
```

The site is then in `web/dist`. Serve it with any static web server, for
example `python -m http.server -d web/dist`, and open the address it
prints. (Opening `index.html` straight from the disk doesn't work:
browsers don't run Web Workers from files.)

`.github/workflows/web.yml` builds it and publishes it to GitHub Pages on
every push to `main`.
