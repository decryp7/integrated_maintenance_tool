#!/usr/bin/env bash
# The CI job `tauri-smoke` (ubuntu leg) in the CI image under Docker, on a developer machine: the same packages, the same
# build and the same `npm run e2e:tauri` under Xvfb, so the real-app smoke can be run before a push without touching the
# host (no window on the desktop, no process left behind). Only the container and the named volumes below are created;
# `--clean` removes the volumes.
#
#   scripts/ci/smoke-linux-docker.sh                  # build the debug app and run every smoke spec
#   scripts/ci/smoke-linux-docker.sh -- --spec e2e-tauri/specs/01-start.e2e.ts
#   scripts/ci/smoke-linux-docker.sh --clean          # remove the cache volumes
set -euo pipefail

IMAGE="${DEBRIEF_SMOKE_IMAGE:-dev.decryptology.net/decryp7/garage:ubuntu-24.04-20261023-r5}"
PREFIX="${DEBRIEF_SMOKE_PREFIX:-debrief-smoke}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VOLUMES=("$PREFIX-target" "$PREFIX-cargo" "$PREFIX-node-modules" "$PREFIX-duckdb")

if [ "${1:-}" = "--clean" ]; then
  docker volume rm "${VOLUMES[@]}" 2>/dev/null || true
  exit 0
fi
[ "${1:-}" = "--" ] && shift

exec docker run --rm --name "$PREFIX-run-$$" --cpus "${DEBRIEF_SMOKE_CPUS:-6}" \
  -v "$ROOT":/work \
  -v "$PREFIX-target":/cache/target \
  -v "$PREFIX-cargo":/root/.cargo \
  -v "$PREFIX-node-modules":/work/app/node_modules \
  -v "$PREFIX-duckdb":/root/.cache/debrief-libduckdb \
  -e CARGO_TARGET_DIR=/cache/target \
  -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" \
  -e WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1 -e WEBKIT_DISABLE_DMABUF_RENDERER=1 \
  -e WEBKIT_DISABLE_COMPOSITING_MODE=1 -e LIBGL_ALWAYS_SOFTWARE=1 -e DEBRIEF_E2E_KEEP_DATA=1 \
  -w /work "$IMAGE" bash -euo pipefail -c '
    export PATH="$HOME/.cargo/bin:$PATH"
    apt-get update -qq
    apt-get install -y -qq libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev patchelf cmake \
      webkit2gtk-driver xvfb dbus dbus-x11 >/dev/null
    command -v cargo >/dev/null || { curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --default-toolchain stable -y; }
    eval "$(bash crates/debrief-store-duck/scripts/fetch-libduckdb.sh "$HOME/.cache/debrief-libduckdb")"
    export LD_LIBRARY_PATH="$DUCKDB_LIB_DIR:${LD_LIBRARY_PATH:-}"
    command -v tauri-driver || cargo install tauri-driver --locked
    cd app
    npm ci --no-audit --no-fund
    npm run e2e:tauri:build
    dbus-run-session -- xvfb-run -a --server-args="-screen 0 1600x1000x24" npm run e2e:tauri -- "$@"
  ' smoke "$@"
