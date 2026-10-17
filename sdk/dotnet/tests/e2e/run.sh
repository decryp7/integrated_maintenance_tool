#!/usr/bin/env bash
# End-to-end gate of the C#/.NET SDK on this machine:
#   1. publish the examples as self-contained single files for this OS (the plugin folder layout),
#   2. run IMT's conformance kit (imt-plugin-check) against each published plugin,
#   3. import the OrderAudit sample data through imt-plugin-host into an imt-core Workspace (tests/e2e-rs).
# Needs .NET 10 and Rust. Usage: sdk/dotnet/tests/e2e/run.sh   (RID=linux-x64 etc. to override)
set -euo pipefail

sdk="$(cd "$(dirname "$0")/../.." && pwd)"
repo="$(cd "$sdk/../.." && pwd)"
export DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 TESTINGPLATFORM_TELEMETRY_OPTOUT=1
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$sdk/artifacts/cargo-target}"

if [[ -z "${RID:-}" ]]; then
  case "$(uname -s)" in
    Darwin) os=osx ;;
    Linux) os=linux ;;
    MINGW* | MSYS* | CYGWIN*) os=win ;;
    *) echo "unknown OS $(uname -s); set RID" >&2; exit 2 ;;
  esac
  case "$(uname -m)" in
    arm64 | aarch64) arch=arm64 ;;
    x86_64 | amd64) arch=x64 ;;
    *) echo "unknown architecture $(uname -m); set RID" >&2; exit 2 ;;
  esac
  RID="$os-$arch"
fi
ext=""
[[ "$RID" == win-* ]] && ext=".exe"

echo "== publish ($RID)"
dotnet publish "$sdk/examples/ToyFormatImporter/src" -c Release -r "$RID"
dotnet publish "$sdk/samples/OrderAudit/src" -c Release -r "$RID"

echo "== imt-plugin-check"
(cd "$repo" && cargo build -p imt-plugin-sdk --bin imt-plugin-check)
check="$CARGO_TARGET_DIR/debug/imt-plugin-check$ext"
for plugin in com.example.toyformat/bin/$RID/toy-format-importer com.example.orderaudit/bin/$RID/order-audit; do
  "$check" "$sdk/artifacts/plugins/$plugin$ext"
done

echo "== workspace import through imt-plugin-host"
cargo test --manifest-path "$sdk/tests/e2e-rs/Cargo.toml"
