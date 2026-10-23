#!/usr/bin/env bash
# Checks the built Debrief.app (and the copy inside the DMG): sealed ad-hoc (bundle.macOS.signingIdentity "-" in
# tauri.macos.conf.json). A linker-signed-only bundle has no sealed resources and Gatekeeper reports a quarantined
# download as "damaged" (docs/releasing.md). Every nested Mach-O (sidecars, libduckdb) must be signed and verified.
#   scripts/ci/macos-verify-bundle.sh [path/to/Debrief.app]   (also checks the .dmg next to it, when there is one)
set -euo pipefail
cd "$(dirname "$0")/../.."
dir="${CARGO_TARGET_DIR:-target}"
bundle="$dir/dist/bundle"
app="${1:-$(find "$bundle/macos" -maxdepth 1 -name '*.app' | head -1)}"
[ -d "${app:-}" ] || { echo "::error::no .app bundle found (looked in $bundle/macos)"; exit 1; }

check_app() {
  local app="$1" sig bin
  codesign --verify --deep --strict --verbose=2 "$app"
  sig=$(codesign -dvvv "$app" 2>&1)
  echo "$sig"
  if echo "$sig" | grep -q 'flags=.*linker-signed'; then
    echo "::error::$app is only linker-signed: set bundle.macOS.signingIdentity to \"-\" (ad-hoc seal)"
    return 1
  fi
  if ! echo "$sig" | grep -Eq 'flags=0x[0-9a-f]+\(.*adhoc'; then
    echo "$sig" | grep -q 'Authority=' || { echo "::error::$app is neither ad-hoc nor identity signed"; return 1; }
    echo "::notice::$app is signed with a real identity"
  fi
  if echo "$sig" | grep -q 'Sealed Resources=none'; then
    echo "::error::$app has no sealed resources"
    return 1
  fi
  echo "$sig" | grep -q 'Sealed Resources' || { echo "::error::$app has no sealed resources"; return 1; }
  # Every Mach-O inside the seal must verify on its own and must not be linker-signed only.
  while IFS= read -r bin; do
    file -b "$bin" | grep -q 'Mach-O' || continue
    codesign --verify --strict "$bin" || { echo "::error::$bin does not verify"; return 1; }
    if codesign -dv "$bin" 2>&1 | grep -q 'flags=.*linker-signed'; then
      echo "::error::$bin is only linker-signed"
      return 1
    fi
    echo "verified: ${bin#"$app"/}"
  done < <(find "$app/Contents" -type f \( -perm -u+x -o -name '*.dylib' \))
  echo "bundle OK: $app"
}

check_app "$app"

dmg=$(find "$bundle/dmg" -maxdepth 1 -name '*.dmg' 2>/dev/null | head -1 || true)
if [ -n "$dmg" ]; then
  mnt=$(mktemp -d)
  trap 'hdiutil detach "$mnt" -quiet || true' EXIT
  hdiutil attach "$dmg" -nobrowse -readonly -mountpoint "$mnt" -quiet
  inner=$(find "$mnt" -maxdepth 1 -name '*.app' | head -1)
  [ -d "$inner" ] || { echo "::error::no .app inside $dmg"; exit 1; }
  check_app "$inner"
fi
