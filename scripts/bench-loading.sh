#!/bin/sh
set -eu
project_dir="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
bench_dir="$(mktemp -d)"
trap 'rm -rf "$bench_dir"' EXIT HUP INT TERM
rustc --edition=2024 -O "$project_dir/benches/large_library.rs" -o "$bench_dir/large-library"
"$bench_dir/large-library"
