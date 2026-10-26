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
# Default visibility, which clang does not give WebAssembly, so that the runtime is exported.
export CFLAGS_wasm32_wasip1="--sysroot=$WASI_SDK/share/wasi-sysroot -fvisibility=default"
# The engine exports A0's runtime (its functions, memory and table), which compiled programs link
# against (playground/compiled.js).
# The C library's functions in the runtime's symbol table are hidden in wasi-libc: exported by name.
libc_exports=$(sed -n '/a0_rt_symbols\[\]/,/{NULL, NULL}/p' crates/a0-runtime/c/symbols.c |
  grep -o 'SYM([a-z_0-9]*)' | sed 's/SYM(\(.*\))/\1/' | grep -v '^a0_' |
  sed 's/^/-C link-arg=--export-if-defined=/' | tr '\n' ' ')
RUSTFLAGS="-C link-arg=--export-dynamic -C link-arg=--export-table -C link-arg=--growable-table $libc_exports" \
  cargo build --release --locked -p a0-playground --target wasm32-wasip1
target="${CARGO_TARGET_DIR:-target}"
out="$target/playground"
rm -rf "$out"
mkdir -p "$out/examples"
cp playground/index.html playground/style.css playground/app.js playground/worker.js \
   playground/wasi.js playground/compiled.js playground/examples.json "$out/"
cp "$target/wasm32-wasip1/release/a0-playground.wasm" "$out/"
cp -r examples/calc examples/lispy "$out/examples/"
echo "built $out"
