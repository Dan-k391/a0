#!/bin/sh
# Builds the playground into target/playground (or $CARGO_TARGET_DIR/playground): the engine,
# crates/a0-playground, for wasm32-wasip1, and the page with its examples. Needs wasi-sdk
# (WASI_SDK=/path/to/wasi-sdk) and the Rust target (rustup target add wasm32-wasip1). Serve the
# directory with any static web server, and test it with `node playground/test.mjs <directory>`.
set -eu
cd "$(dirname "$0")/.."
: "${WASI_SDK:?set WASI_SDK to the directory of wasi-sdk}"
export CC_wasm32_wasip1="$WASI_SDK/bin/clang"
export AR_wasm32_wasip1="$WASI_SDK/bin/llvm-ar"
export CFLAGS_wasm32_wasip1="--sysroot=$WASI_SDK/share/wasi-sysroot"
cargo build --release --locked -p a0-playground --target wasm32-wasip1
target="${CARGO_TARGET_DIR:-target}"
out="$target/playground"
rm -rf "$out"
mkdir -p "$out/examples"
cp playground/index.html playground/style.css playground/app.js playground/worker.js \
   playground/wasi.js playground/examples.json "$out/"
cp "$target/wasm32-wasip1/release/a0-playground.wasm" "$out/"
cp -r examples/calc examples/lispy "$out/examples/"
echo "built $out"
