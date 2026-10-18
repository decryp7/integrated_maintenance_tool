#!/usr/bin/env bash
# Exports every docs/diagrams/*.drawio to docs/diagrams/svg/<name>.svg with the draw.io desktop CLI,
# for docs/design.md (Markdown viewers can't render .drawio). Run after editing a diagram:
#   scripts/export-diagrams.sh
# DRAWIO overrides the draw.io executable (default: found on PATH or in the usual install folders).
set -euo pipefail
cd "$(dirname "$0")/.."

drawio="${DRAWIO:-}"
if [[ -z "$drawio" ]]; then
  for c in drawio draw.io "/c/Program Files/draw.io/draw.io.exe" "/Applications/draw.io.app/Contents/MacOS/draw.io"; do
    if command -v "$c" >/dev/null 2>&1 || [[ -x "$c" ]]; then drawio="$c"; break; fi
  done
fi
[[ -n "$drawio" ]] || { echo "draw.io desktop not found; install it or set DRAWIO" >&2; exit 1; }

mkdir -p docs/diagrams/svg
for f in docs/diagrams/*.drawio; do
  name="$(basename "$f" .drawio)"
  # Fonts are Arial and Consolas/Menlo (installed on Windows and macOS), so nothing is embedded.
  "$drawio" --export --format svg --border 16 --output "docs/diagrams/svg/$name.svg" "$f" --no-sandbox >/dev/null
  # draw.io adds a PNG copy of every HTML label (inside <switch>, after its <foreignObject>) for viewers
  # without foreignObject support. Browsers, Gitea and GitHub render the HTML, so drop the copies:
  # they make each file megabytes larger.
  node -e '
    const fs = require("fs"), f = process.argv[1];
    const svg = fs.readFileSync(f, "utf8").replace(/(<\/foreignObject>)<image [^>]*xlink:href="data:image\/png;base64,[^"]*"[^>]*\/>/g, "$1");
    fs.writeFileSync(f, svg);
  ' "docs/diagrams/svg/$name.svg"
  echo "exported $name.svg"
done
