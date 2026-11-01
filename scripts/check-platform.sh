#!/bin/sh
# Pass a Rust target triple to check the portable core and network adapters.
# Native UI checks use the host's window-system toolchain.
set -eu
project_dir="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
if [ "$#" -gt 0 ]; then
    target="$1"
    shift
    case "$target" in
        *-apple-darwin) features="remote,apple-photos" ;;
        *) features="remote" ;;
    esac
    cargo check --locked --manifest-path "$project_dir/Cargo.toml" --no-default-features --features "$features" --lib --target "$target" "$@"
else
    cargo check --locked --manifest-path "$project_dir/Cargo.toml" --all-targets "$@"
fi
