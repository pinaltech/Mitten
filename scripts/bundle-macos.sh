#!/bin/sh
set -eu
project_dir="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
build_dir="${CARGO_TARGET_DIR:-$project_dir/target}"
cargo build --locked --manifest-path "$project_dir/Cargo.toml" --target-dir "$build_dir"
# Validate what Foundation actually sees for the executable before packaging.
"$build_dir/debug/mitten" --verify-photo-permissions
app_dir="$project_dir/target/Mitten.app"
mkdir -p "$app_dir/Contents/MacOS" "$app_dir/Contents/Resources"
# Replace the executable inode so macOS does not reuse a stale code-signature
# cache entry when rebuilding an already launched development bundle.
cp "$build_dir/debug/mitten" "$app_dir/Contents/MacOS/.mitten-new"
mv -f "$app_dir/Contents/MacOS/.mitten-new" "$app_dir/Contents/MacOS/mitten"
# One canonical plist supplies both the Mach-O section and the app bundle.
cp "$project_dir/native/Info.plist" "$app_dir/Contents/Info.plist"
cp "$project_dir/assets/brand/AppIcon.icns" "$app_dir/Contents/Resources/AppIcon.icns"
"$app_dir/Contents/MacOS/mitten" --verify-photo-permissions
printf '%s\n' "$app_dir"
