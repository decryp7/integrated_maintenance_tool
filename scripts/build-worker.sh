#!/usr/bin/env bash
# Builds the AI inference worker (crates/imt-ai-worker, llama.cpp) for the target platform and puts it where
# Tauri bundles it as a sidecar:
#   app/src-tauri/binaries/imt-ai-worker-<target-triple>[.exe]
# The sidecar is declared in app/src-tauri/tauri.ai.conf.json (`bundle.externalBin`), merged at bundle time:
#   (cd app && npx tauri build --config src-tauri/tauri.ai.conf.json)
# It is not in tauri.conf.json because tauri-build refuses to compile the app when a declared sidecar is
# missing, which would break every `cargo build`/`cargo test` that did not build the worker first.
#
# GPU backend per target: Metal on macOS, Vulkan on Windows and Linux (needs the Vulkan SDK at build time:
# VULKAN_SDK set, glslc on PATH). `--cpu` builds without a GPU backend.
#
# Usage: scripts/build-worker.sh [--release|--debug] [--target <triple>] [--cpu]
# Needs cmake and a C/C++ toolchain. Compiling llama.cpp takes a few minutes.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
profile=release
target=""
cpu=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --release) profile=release ;;
    --debug) profile=debug ;;
    --target) target="${2:?--target needs a triple}"; shift ;;
    --cpu) cpu=1 ;;
    -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

host="$(rustc -vV | sed -n 's/^host: //p')"
target="${target:-$host}"

feature=""
if [[ $cpu -eq 0 ]]; then
  case "$target" in
    *-apple-darwin) feature=metal ;;
    *-windows-*|*-linux-*) feature=vulkan ;;
  esac
fi

if ! command -v cmake >/dev/null 2>&1; then
  echo "cmake is required to build llama.cpp (install it, e.g. brew install cmake / apt install cmake)" >&2
  exit 1
fi
if [[ "$feature" == vulkan && -z "${VULKAN_SDK:-}" ]] && ! command -v glslc >/dev/null 2>&1; then
  echo "the Vulkan backend needs the Vulkan SDK (set VULKAN_SDK, glslc on PATH), or pass --cpu" >&2
  exit 1
fi

args=(build -p imt-ai-worker --target "$target")
[[ $profile == release ]] && args+=(--release)
[[ -n "$feature" ]] && args+=(--features "$feature")
echo "cargo ${args[*]}"
(cd "$root" && cargo "${args[@]}")

target_dir="${CARGO_TARGET_DIR:-$root/target}"
ext=""
[[ "$target" == *-windows-* ]] && ext=".exe"
built="$target_dir/$target/$profile/imt-ai-worker$ext"
out_dir="$root/app/src-tauri/binaries"
mkdir -p "$out_dir"
out="$out_dir/imt-ai-worker-$target$ext"
cp "$built" "$out"
chmod +x "$out"
echo "worker: $out (${feature:-cpu})"
