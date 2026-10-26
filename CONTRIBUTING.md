# Contributing

Thanks for helping. A few things keep the project healthy.

## Layout
- `crates/straws-core`: the library. Deck reading and editing (`deck/`), the
  engine wrapper (`engine.rs`, `worker.rs`), results (`output.rs`), analysis
  (`analysis/`: plans, patterns, checks, matching, feedlines, near fields),
  the optimiser, templates, exports, and nec2c itself (`nec2c/`, C).
- `crates/straws-gui`: the desktop app (egui, egui_dock, wgpu). `app.rs` is the
  window; each view is in `views/`.
- `crates/straws-cli`: the command-line tool (validation, benchmarks).
- `tests/decks` and `tests/references`: regression decks and results from
  4nec2's engine.

## Rules
- Keep `cargo fmt` and `cargo clippy --workspace --all-targets -- -D warnings`
  clean, and `cargo test --workspace` passing. CI checks all three on Windows,
  Linux and macOS.
- Anything that changes engine results must keep the regression suite passing.
  A feature isn't done until its numbers match the reference.
- nec2c changes are marked `straws:` in the source and listed in
  `crates/straws-core/nec2c/README.straws.md`.
- Don't add 4nec2's example models or other decks we can't redistribute; write
  your own test decks.
- Australian English in UI text, comments and docs. Messages for users say what
  is wrong and how to fix it, in plain words.
- Speed matters: measure before and after (`straws-cli bench`), and record results
  in `docs/benchmarks.md`.

## Running tests
```
cargo test --workspace
cargo test --release -p straws-core --test regression -- --ignored --nocapture
```
The ignored tests time the optimiser and write a sample report.
