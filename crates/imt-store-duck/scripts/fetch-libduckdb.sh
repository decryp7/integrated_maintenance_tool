#!/usr/bin/env sh
# Downloads the official prebuilt libduckdb for this platform, verifies its pinned SHA-256 and
# unpacks it into a cache directory, so imt-store-duck links DuckDB without compiling its C++ sources
# (the `bundled` feature needs several GB of RAM and many minutes).
#
#   eval "$(crates/imt-store-duck/scripts/fetch-libduckdb.sh [cache-dir])"
#
# prints `export DUCKDB_LIB_DIR=... DUCKDB_INCLUDE_DIR=...` (and PATH on Windows, where the
# executable needs duckdb.dll next to it or on PATH). The version must match the `duckdb` crate:
# duckdb-rs 1.10506.x = DuckDB 1.5.6. Checksums are the GitHub release API digests of v1.5.6.
set -eu

VERSION=1.5.6
CACHE=${1:-${CARGO_TARGET_DIR:-target}/libduckdb}

case "$(uname -s)-$(uname -m)" in
  MINGW*-x86_64 | MSYS*-x86_64 | CYGWIN*-x86_64)
    ASSET=libduckdb-windows-amd64.zip; SHA=44cf59583f9951d2cb09b1bf115a63ecb2d8901e363903029d86c7d8683fe96a ;;
  MINGW*-aarch64 | MSYS*-aarch64)
    ASSET=libduckdb-windows-arm64.zip; SHA=c63285647e94c46d309b60d5dd1ff3cb2cdf9ca03e415a818ff7d8dbf838e0e8 ;;
  Linux-x86_64)
    ASSET=libduckdb-linux-amd64.zip; SHA=b845005f5132a7d8180057c35e14a7626632258782f871a90861b19c1c03841b ;;
  Linux-aarch64)
    ASSET=libduckdb-linux-arm64.zip; SHA=b72ed9f05003f5e9d2015f7ceada6416b377d9dd33169cdde3c9e33897856eee ;;
  Darwin-*)
    ASSET=libduckdb-osx-universal.zip; SHA=e0bc007d9b0094c0970ac1847a8601d10aad07cbd2910ce586ec810f77b638d6 ;;
  *) echo "no pinned libduckdb for $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

DIR="$CACHE/$VERSION/${ASSET%.zip}"
if [ ! -f "$DIR/.verified" ]; then
  mkdir -p "$CACHE/$VERSION"
  ZIP="$CACHE/$VERSION/$ASSET"
  curl -fsSL --retry 3 -o "$ZIP" "https://github.com/duckdb/duckdb/releases/download/v$VERSION/$ASSET"
  GOT=$(sha256sum "$ZIP" 2>/dev/null | cut -d' ' -f1 || shasum -a 256 "$ZIP" | cut -d' ' -f1)
  if [ "$GOT" != "$SHA" ]; then
    rm -f "$ZIP"
    echo "SHA-256 mismatch for $ASSET: expected $SHA, got $GOT" >&2
    exit 1
  fi
  rm -rf "$DIR"
  mkdir -p "$DIR"
  # A fresh, empty directory; only the library, import library and headers are used.
  unzip -q -o "$ZIP" -d "$DIR"
  touch "$DIR/.verified"
fi

ABS=$(cd "$DIR" && pwd)
# Cargo build scripts on Windows need a native path (C:/...), the shell's PATH a POSIX one.
NATIVE=$ABS
command -v cygpath >/dev/null 2>&1 && NATIVE=$(cygpath -m "$ABS")
echo "export DUCKDB_LIB_DIR='$NATIVE' DUCKDB_INCLUDE_DIR='$NATIVE'"
case "$ASSET" in
  libduckdb-windows-*) echo "export PATH='$ABS':\"\$PATH\"" ;;
  libduckdb-linux-*) echo "export LD_LIBRARY_PATH='$ABS':\"\${LD_LIBRARY_PATH:-}\"" ;;
  libduckdb-osx-*) echo "export DYLD_LIBRARY_PATH='$ABS':\"\${DYLD_LIBRARY_PATH:-}\"" ;;
esac
