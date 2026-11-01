#!/bin/sh
set -eu
project_dir="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
build_dir="${CARGO_TARGET_DIR:-$project_dir/../DesertUI/target}"
cargo build --locked --manifest-path "$project_dir/Cargo.toml" --target-dir "$build_dir"
# Validate what Foundation actually sees for the executable before packaging.
"$build_dir/debug/desert-photos" --verify-photo-permissions
app_dir="$project_dir/target/GPUI Photos.app"
mkdir -p "$app_dir/Contents/MacOS"
cp "$build_dir/debug/desert-photos" "$app_dir/Contents/MacOS/desert-photos"
# One canonical plist supplies both the Mach-O section and the app bundle.
cp "$project_dir/native/Info.plist" "$app_dir/Contents/Info.plist"
"$app_dir/Contents/MacOS/desert-photos" --verify-photo-permissions
printf '%s\n' "$app_dir"
