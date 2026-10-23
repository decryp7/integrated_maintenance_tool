#!/usr/bin/env bash
# Real-app smoke for macOS (audit D3): the built Debrief.app must start every sidecar and quit within 10 s.
#   1. every executable under Contents/MacOS other than the main one is a sidecar with a known self-test; one without
#      an entry here fails the job (add its self-test below, so a new sidecar can never ship unstarted);
#   2. the main executable is launched with throw-away data folders, must still be running after a settle time (a crash
#      at start fails), is asked to quit and must be gone within QUIT_LIMIT seconds. Nothing is left running.
# WebDriver cannot drive a WKWebView on macOS, so this is the macOS leg of `real-app smoke`; Linux and Windows run the
# WebDriver suite (app/e2e-tauri).
#   scripts/ci/macos-smoke.sh path/to/Debrief.app
# SMOKE_QUIT=term asks with SIGTERM instead of the Apple event "quit" (used by the script's own test, where the app is a stub).
set -euo pipefail
app="${1:?usage: macos-smoke.sh path/to/Debrief.app}"
[ -d "$app" ] || { echo "::error::no .app at $app"; exit 1; }
macos="$app/Contents/MacOS"
plist="$app/Contents/Info.plist"
QUIT_LIMIT="${QUIT_LIMIT:-10}"
SETTLE="${SETTLE:-6}"

main_exe=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$plist")
bundle_id=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$plist")
[ -x "$macos/$main_exe" ] || { echo "::error::the main executable $macos/$main_exe is missing"; exit 1; }

scratch=$(mktemp -d "${TMPDIR:-/tmp}/debrief-smoke-XXXXXX")
pid=""
cleanup() {
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then kill -KILL "$pid" 2>/dev/null || true; fi
  rm -rf "$scratch"
}
trap cleanup EXIT

# Runs one sidecar self-test with a time limit; prints its output.
run_limited() {
  local limit="$1"; shift
  "$@" > "$scratch/out" 2>&1 &
  local p=$! waited=0
  while kill -0 "$p" 2>/dev/null; do
    if [ "$waited" -ge $((limit * 10)) ]; then kill -KILL "$p" 2>/dev/null || true; wait "$p" 2>/dev/null || true; cat "$scratch/out"; echo "::error::$1 did not finish within ${limit}s"; return 1; fi
    sleep 0.1; waited=$((waited + 1))
  done
  local rc=0; wait "$p" || rc=$?
  cat "$scratch/out"
  [ "$rc" -eq 0 ] || { echo "::error::$1 failed with exit code $rc"; return 1; }
}

echo "== sidecars =="
found=0
while IFS= read -r exe; do
  name=$(basename "$exe")
  [ "$name" = "$main_exe" ] && continue
  found=$((found + 1))
  case "$name" in
    debrief-store-worker) args=(--self-test) ;;
    debrief-plugins) args=(--manifests) ;;
    debrief-ai-worker) args=(--list-builds) ;;
    *) echo "::error::sidecar $name has no self-test in scripts/ci/macos-smoke.sh: add one (every sidecar must be started by the smoke)"; exit 1 ;;
  esac
  echo "-- $name ${args[*]}"
  run_limited 40 "$exe" "${args[@]}" | head -c 4000
  [ "${PIPESTATUS[0]}" -eq 0 ] || exit 1
done < <(find "$macos" -maxdepth 1 -type f -perm -u+x | sort)
[ "$found" -gt 0 ] || { echo "::error::no sidecar executables found in $macos"; exit 1; }
echo "$found sidecar(s) started"

echo "== launch and quit =="
export DEBRIEF_DATA_DIR="$scratch/data"
mkdir -p "$DEBRIEF_DATA_DIR"
"$macos/$main_exe" > "$scratch/app.log" 2>&1 &
pid=$!
sleep "$SETTLE"
if ! kill -0 "$pid" 2>/dev/null; then
  rc=0; wait "$pid" || rc=$?
  tail -n 60 "$scratch/app.log" || true
  echo "::error::$main_exe exited by itself within ${SETTLE}s (exit code $rc): it must stay running until quit"
  pid=""
  exit 1
fi
echo "running (pid $pid); asking it to quit"
start=$SECONDS
if [ "${SMOKE_QUIT:-event}" = term ]; then
  kill -TERM "$pid"
else
  osascript -e "tell application id \"$bundle_id\" to quit" || kill -TERM "$pid"
fi
while kill -0 "$pid" 2>/dev/null; do
  if [ $((SECONDS - start)) -ge "$QUIT_LIMIT" ]; then
    tail -n 60 "$scratch/app.log" || true
    echo "::error::$main_exe was still running ${QUIT_LIMIT}s after the quit request (the exit watchdog should end it within 8 s)"
    exit 1
  fi
  sleep 0.2
done
wait "$pid" 2>/dev/null || true
echo "quit after $((SECONDS - start))s"
pid=""
# Nothing left behind: no helper of this app still running.
if pgrep -f "$macos/" >/dev/null 2>&1; then
  pgrep -fl "$macos/" || true
  pkill -KILL -f "$macos/" || true
  echo "::error::processes of $app were left running after the quit"
  exit 1
fi
echo "macOS smoke OK: $app"
