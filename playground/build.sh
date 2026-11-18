#!/bin/sh
# Build the playground into site/: the toolchain as WebAssembly, the
# examples, and the icon. Serve site/ with any static file server.
set -eu
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/bailiwick_playground.wasm site/bailiwick.wasm
mkdir -p site/examples
for e in clinic-sovereign storefront edge-site; do cp "../examples/$e.bwk" site/examples/; done
cp ../assets/icon-square.svg site/icon.svg
