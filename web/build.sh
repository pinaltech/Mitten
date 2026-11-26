#!/bin/sh
# Builds Mitten itself for the browser.
#
#   web/build.sh [OUT]      (OUT defaults to web/dist)
#
# Writes OUT/mitten/: the wasm module (pkg/), the loading screen and its
# browser glue (shell/). When OUT has no index.html yet, Mitten's own page
# and default host.js are copied there too; a site embedding Mitten (Pinal
# Photos, say) keeps its own page and host.js and only takes OUT/mitten/.
#
# GPUI's web platform runs Mitten's background work in Web Workers sharing
# the module's memory, which needs a nightly toolchain to rebuild the
# standard library with atomics:
#   rustup toolchain install nightly --component rust-src --target wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version "$(grep -A1 'name = "wasm-bindgen"$' Cargo.lock | sed -n 's/version = "\(.*\)"/\1/p')" --locked
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$root/web/dist}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
cd "$root"
target_dir="${CARGO_TARGET_DIR:-$root/target/wasm}"
RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals
 -C link-arg=--shared-memory -C link-arg=--max-memory=4294967296 -C link-arg=--import-memory
 -C link-arg=--export=__wasm_init_tls -C link-arg=--export=__tls_size
 -C link-arg=--export=__tls_align -C link-arg=--export=__tls_base
 --cfg getrandom_backend="wasm_js"' \
CARGO_TARGET_DIR="$target_dir" \
	cargo +nightly build --locked --profile web -Z build-std=std,panic_abort \
	--target wasm32-unknown-unknown --no-default-features --features native-ui --bin mitten
rm -rf "$out/mitten"
mkdir -p "$out/mitten"
wasm-bindgen --target web --no-typescript --remove-name-section --remove-producers-section \
	--out-dir "$out/mitten/pkg" "$target_dir/wasm32-unknown-unknown/web/mitten.wasm"
# The loading screen's progress bar: servers may compress the module, so
# its size is not always in the response.
wc -c < "$out/mitten/pkg/mitten_bg.wasm" | tr -d ' ' > "$out/mitten/pkg/mitten_bg.size"
cp -R web/shell/. "$out/mitten/"
rm -f "$out/mitten/index.html" "$out/mitten/host.js"
if [ ! -e "$out/index.html" ]; then
	cp web/shell/index.html web/shell/host.js "$out/"
fi
ls -l "$out/mitten/pkg"
