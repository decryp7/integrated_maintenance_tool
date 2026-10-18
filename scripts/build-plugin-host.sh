#!/usr/bin/env bash
# Builds the plugin host (crates/imt-plugin-host: serves built-in plugins out of process when they run
# isolated) for the target platform and puts it where Tauri bundles it as a sidecar:
#   app/src-tauri/binaries/imt-plugin-host-<target-triple>[.exe]
# Release builds do this for both sidecars (AI worker and plugin host) through app/scripts/build-sidecars.mjs:
#   (cd app && npx tauri build --config src-tauri/tauri.sidecars.conf.json)
# This script is for building the plugin host alone.
# Pure Rust: no C toolchain needed. At run time IMT finds it next to its executable (IMT_PLUGIN_HOST
# overrides the path, for development).
#
# Usage: scripts/build-plugin-host.sh [--release|--debug] [--target <triple>]
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
args=(build -p imt-plugin-host --bin imt-plugin-host --target "$target")
[[ $profile == release ]] && args+=(--release)
echo "cargo ${args[*]}"
(cd "$root" && cargo "${args[@]}")

target_dir="${CARGO_TARGET_DIR:-$root/target}"
ext=""
[[ "$target" == *-windows-* ]] && ext=".exe"
out_dir="$root/app/src-tauri/binaries"
mkdir -p "$out_dir"
out="$out_dir/imt-plugin-host-$target$ext"
cp "$target_dir/$target/$profile/imt-plugin-host$ext" "$out"
chmod +x "$out"
echo "plugin host: $out"
