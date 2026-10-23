#!/usr/bin/env bash
# Builds the plugins binary (crates/debrief-plugins: serves every built-in plugin, each in its own sandboxed
# process, ADR 0017) for the target platform and puts it where Tauri bundles it as a sidecar:
#   app/src-tauri/binaries/debrief-plugins-<target-triple>[.exe]
# Release builds do this for both sidecars (AI worker and plugins) through app/scripts/build-sidecars.mjs:
#   (cd app && npx tauri build --config src-tauri/tauri.sidecars.conf.json)
# This script is for building the plugins alone.
# Pure Rust: no C toolchain needed. At run time Debrief finds it next to its executable (DEBRIEF_PLUGINS overrides
# the path, for development).
#
# Usage: scripts/build-plugins.sh [--release|--debug] [--target <triple>]
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
profile=release
target=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --release) profile=release ;;
    --debug) profile=debug ;;
    --target) target="${2:?--target needs a triple}"; shift ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

host="$(rustc -vV | sed -n 's/^host: //p')"
target="${target:-$host}"
args=(build -p debrief-plugins --bin debrief-plugins --target "$target")
[[ $profile == release ]] && args+=(--release)
echo "cargo ${args[*]}"
(cd "$root" && cargo "${args[@]}")

target_dir="${CARGO_TARGET_DIR:-$root/target}"
ext=""
[[ "$target" == *-windows-* ]] && ext=".exe"
out_dir="$root/app/src-tauri/binaries"
mkdir -p "$out_dir"
out="$out_dir/debrief-plugins-$target$ext"
cp "$target_dir/$target/$profile/debrief-plugins$ext" "$out"
chmod +x "$out"
echo "plugins: $out"
