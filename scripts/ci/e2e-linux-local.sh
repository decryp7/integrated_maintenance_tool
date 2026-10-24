#!/usr/bin/env bash
# Runs Debrief's Playwright e2e in the CI image (same as the `e2e` job in .gitea/workflows/build.yml), so Linux fonts,
# metrics and WebKit are tested before a push or a tag. Usage:
#   scripts/ci/e2e-linux-local.sh [playwright args...]     e.g. e2e/search-field.e2e.ts --project=webkit --repeat-each=3
# Environment: E2E_CPUS (default 6), E2E_WORKERS (default 2), E2E_PLATFORM (default: the image's, linux/amd64).
# Creates one container and one named volume (debrief-e2e-linux-<worktree>) and removes the container on exit; the volume
# keeps node_modules between runs (Linux binaries never touch the checkout's node_modules). Remove it with
#   docker volume rm debrief-e2e-linux-<worktree>
set -euo pipefail

IMAGE="${E2E_IMAGE:-dev.decryptology.net/decryp7/garage:ubuntu-24.04-20261023-r5}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
NAME="debrief-e2e-linux-$(basename "$ROOT" | tr -c 'a-zA-Z0-9_.-' '-')"
CPUS="${E2E_CPUS:-6}"
WORKERS="${E2E_WORKERS:-2}"
PORT="${DEBRIEF_E2E_PORT:-4355}"

docker volume create "$NAME" >/dev/null
cleanup() { docker rm -f "$NAME" >/dev/null 2>&1 || true; }
trap cleanup EXIT
cleanup

docker run -d --name "$NAME" --cpus "$CPUS" ${E2E_PLATFORM:+--platform "$E2E_PLATFORM"} -v "$NAME:/work" "$IMAGE" sleep infinity >/dev/null

# Ship the sources (never node_modules, builds or reports) into the volume.
(cd "$ROOT" && COPYFILE_DISABLE=1 tar --exclude=node_modules --exclude=dist --exclude=.angular --exclude=playwright-report \
  --exclude=test-results --exclude=src-tauri -cf - app docs) | docker exec -i "$NAME" tar -xf - -C /work

# Install only when the lockfile changed (the volume keeps node_modules).
docker exec "$NAME" bash -ec '
  cd /work/app
  if [ ! -f node_modules/.lock-hash ] || ! sha256sum -c node_modules/.lock-hash >/dev/null 2>&1; then
    npm ci
    sha256sum package-lock.json > node_modules/.lock-hash
  fi'

# Same environment as the job; browsers come preinstalled in the image (PLAYWRIGHT_BROWSERS_PATH).
set +e
docker exec -e CI=true -e DEBRIEF_E2E_PORT="$PORT" -e DEBRIEF_E2E_SKIP_PROD="${DEBRIEF_E2E_SKIP_PROD:-1}" -e DEBRIEF_E2E_PROD_PORT="$((PORT + 1))" \
  "$NAME" bash -c "cd /work/app && npx playwright test --retries=0 --workers=$WORKERS $*"
status=$?
set -e
rm -rf "$ROOT/app/playwright-report.linux"
docker cp "$NAME:/work/app/playwright-report" "$ROOT/app/playwright-report.linux" >/dev/null 2>&1 || true
exit $status
