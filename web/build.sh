#!/usr/bin/env bash
# Builds the browser version into web/dist (see README.md in this folder).
#
# Needs: the Rust targets wasm32-unknown-unknown and wasm32-wasip1, trunk,
# and wasi-sdk (WASI_SDK_PATH pointing at it) to compile the C engine.
# Extra arguments go to trunk, e.g. `--public-url /straws/`.
set -euo pipefail
cd "$(dirname "$0")/.."

: "${WASI_SDK_PATH:?Set WASI_SDK_PATH to a wasi-sdk folder (see web/README.md)}"

# The engine: nec2c and the solver, built as a WASI program that each
# engine Web Worker runs.
cargo build --release -p straws-core --bin straws-worker --target wasm32-wasip1
mkdir -p web/engine
cp target/wasm32-wasip1/release/straws-worker.wasm web/engine/straws-engine.wasm

# The app itself.
cd web
trunk build --release "$@"
