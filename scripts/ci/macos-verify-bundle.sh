#!/usr/bin/env bash
# Checks the built Debrief.app (and the copy inside the DMG): sealed ad-hoc (bundle.macOS.signingIdentity "-" in
# tauri.macos.conf.json). A linker-signed-only bundle has no sealed resources and Gatekeeper reports a quarantined
# download as "damaged" (docs/releasing.md). Every nested Mach-O (sidecars, libduckdb) must be signed and verified.
# Output is captured and matched in bash (never `cmd | grep -q` under pipefail: an early grep exit can fail the pipe).
#   scripts/ci/macos-verify-bundle.sh [path/to/Debrief.app]   (also checks the .dmg next to it, when there is one)
set -euo pipefail
cd "$(dirname "$0")/../.."
dir="${CARGO_TARGET_DIR:-target}"
bundle="$dir/dist/bundle"
app="${1:-$(find "$bundle/macos" -maxdepth 1 -name '*.app' -print -quit 2>/dev/null)}"
[ -d "${app:-}" ] || { echo "::error::no .app bundle found (looked in $bundle/macos)"; exit 1; }

# Per-line patterns over codesign's output (a bracket expression without newline keeps a match on one line).
NL=$'\n'
LINKER_SIGNED="flags=[^${NL}]*linker-signed"
ADHOC="flags=0x[0-9a-f]+\\([^${NL}]*adhoc"

# The Team ID of a signed Mach-O ("" when it has none).
team_of() { codesign -dv "$1" 2>&1 | sed -n 's/^TeamIdentifier=//p' | sed 's/^not set$//'; }

# Really loads the code from the built app: a library-validation or run-path failure aborts the process.
load_check() {
  local app="$1" macos="$1/Contents/MacOS" out
  out=$("$macos/debrief-store-worker" --self-test 2>&1) || { echo "$out"; echo "::error::the store worker of $app cannot open a store"; return 1; }
  echo "$out"
  out=$("$macos/debrief-plugins" --manifests 2>&1) || { echo "$out" | head -5; echo "::error::debrief-plugins of $app does not start"; return 1; }
  out=$("$macos/debrief-ai-worker" --list-builds 2>&1) || true
  [ -x "$macos/debrief-ai-worker" ] || { echo "::error::debrief-ai-worker is missing from $app"; return 1; }
}

check_app() {
  local app="$1" sig bin
  codesign --verify --deep --strict --verbose=2 "$app"
  sig=$(codesign -dvvv "$app" 2>&1)
  echo "$sig"
  if [[ $sig =~ $LINKER_SIGNED ]]; then
    echo "::error::$app is only linker-signed: set bundle.macOS.signingIdentity to \"-\" (ad-hoc seal)"
    return 1
  fi
  if ! [[ $sig =~ $ADHOC ]]; then
    [[ $sig == *Authority=* ]] || { echo "::error::$app is neither ad-hoc nor identity signed"; return 1; }
    echo "::notice::$app is signed with a real identity"
  fi
  if [[ $sig == *'Sealed Resources=none'* ]]; then
    echo "::error::$app has no sealed resources"
    return 1
  fi
  [[ $sig == *'Sealed Resources'* ]] || { echo "::error::$app has no sealed resources"; return 1; }
  # Every Mach-O inside the seal must verify on its own, must not be linker-signed only and must carry the main
  # executable's Team ID (none, ad hoc): dyld's library validation refuses a library of another team ("different Team
  # IDs"), which made the store worker abort at start with DuckDB's own Developer ID signature on libduckdb.
  local main_team bin_team
  main_team=$(team_of "$(find "$app/Contents/MacOS" -type f -perm -u+x -print -quit)")
  while IFS= read -r bin; do
    [[ $(file -b "$bin") == *Mach-O* ]] || continue
    codesign --verify --strict "$bin" || { echo "::error::$bin does not verify"; return 1; }
    bin_sig=$(codesign -dv "$bin" 2>&1 || true)
    if [[ $bin_sig =~ $LINKER_SIGNED ]]; then
      echo "::error::$bin is only linker-signed"
      return 1
    fi
    bin_team=$(team_of "$bin")
    if [ "$bin_team" != "$main_team" ]; then
      echo "::error::$bin has Team ID '$bin_team', the app has '$main_team': library validation will refuse it (re-sign it ad hoc)"
      return 1
    fi
    echo "verified: ${bin#"$app"/}"
  done < <(find "$app/Contents" -type f \( -perm -u+x -o -name '*.dylib' \))
  load_check "$app"
  echo "bundle OK: $app"
}

check_app "$app"

dmg=$(find "$bundle/dmg" -maxdepth 1 -name '*.dmg' -print -quit 2>/dev/null || true)
if [ -n "$dmg" ]; then
  mnt=$(mktemp -d)
  trap 'hdiutil detach "$mnt" -quiet || true' EXIT
  hdiutil attach "$dmg" -nobrowse -readonly -mountpoint "$mnt" -quiet
  inner=$(find "$mnt" -maxdepth 1 -name '*.app' -print -quit)
  [ -d "$inner" ] || { echo "::error::no .app inside $dmg"; exit 1; }
  check_app "$inner"
fi
