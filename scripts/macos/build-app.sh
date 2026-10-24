#!/usr/bin/env bash
# Local release build of the Debrief app. On macOS, signs with the "Debrief Local Dev"
# identity when present (see dev-signing.sh) so privacy grants survive rebuilds;
# otherwise builds unsigned. Extra arguments go to `tauri build`.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
NAME="Debrief Local Dev"

if [[ "$(uname)" == "Darwin" && -z "${APPLE_SIGNING_IDENTITY:-}" ]] \
  && [[ $(security find-identity -v -p codesigning) == *"\"$NAME\""* ]]; then
  export APPLE_SIGNING_IDENTITY="$NAME"
  echo "Signing with \"$NAME\"."
fi

cd "$ROOT/app"
npx tauri build "$@"
