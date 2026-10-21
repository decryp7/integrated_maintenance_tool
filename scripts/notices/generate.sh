#!/usr/bin/env bash
# Regenerates THIRD_PARTY_NOTICES.md (Rust crates via cargo-about, npm production deps via
# package-lock.json) into app/src-tauri/resources/ and the bundled About-dialog sources.
# Usage: scripts/notices/generate.sh        (needs cargo, node and `cargo install cargo-about --locked --features cli`)
set -euo pipefail
cd "$(dirname "$0")/../.."
root=$PWD
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if ! command -v cargo-about >/dev/null; then
  echo "installing cargo-about..." >&2
  cargo install cargo-about --locked --features cli
fi
if [ ! -d app/node_modules ]; then
  (cd app && npm ci --omit=dev --ignore-scripts)
fi

rust=()
for manifest in app/src-tauri/Cargo.toml crates/imt-ai-worker/Cargo.toml crates/imt-plugins/Cargo.toml; do
  [ -f "$manifest" ] || continue
  out="$tmp/$(echo "$manifest" | tr '/' '_').json"
  cargo about generate -c scripts/notices/about.toml --format json --manifest-path "$manifest" -o "$out" 2>"$tmp/err.log" \
    || { cat "$tmp/err.log" >&2; echo "cargo-about failed for $manifest" >&2; exit 1; }
  rust+=("$out")
done

node scripts/notices/generate.mjs --rust "${rust[@]}" --app app --out app/src-tauri/resources
echo "wrote app/src-tauri/resources/THIRD_PARTY_NOTICES.md"
