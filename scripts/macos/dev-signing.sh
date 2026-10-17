#!/usr/bin/env bash
# Creates a self-signed code-signing identity ("IMT Local Dev") in the login
# keychain, once. Local builds signed with it keep the same identity across
# rebuilds, so macOS remembers privacy grants (e.g. removable-volume access)
# instead of prompting after every build. Not for distribution.
#
# Usage: scripts/macos/dev-signing.sh [--remove]
set -euo pipefail

NAME="IMT Local Dev"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"

[[ "$(uname)" == "Darwin" ]] || { echo "macOS only"; exit 0; }

has_identity() { security find-identity -v -p codesigning "$KEYCHAIN" | grep -q "\"$NAME\""; }

if [[ "${1:-}" == "--remove" ]]; then
  while security find-certificate -c "$NAME" "$KEYCHAIN" >/dev/null 2>&1; do
    security delete-identity -c "$NAME" "$KEYCHAIN" >/dev/null 2>&1 \
      || security delete-certificate -c "$NAME" "$KEYCHAIN"
  done
  echo "Removed \"$NAME\"."
  exit 0
fi

if has_identity; then
  echo "Signing identity \"$NAME\" already present."
  exit 0
fi

command -v openssl >/dev/null || { echo "openssl is required"; exit 1; }
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
PASS="$(openssl rand -hex 16)"

openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -keyout "$TMP/key.pem" -out "$TMP/cert.pem" -subj "/CN=$NAME" \
  -addext "basicConstraints=critical,CA:false" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "extendedKeyUsage=critical,codeSigning" 2>/dev/null

# macOS `security` only reads PKCS#12 with legacy (SHA1/3DES) encryption.
openssl pkcs12 -export -inkey "$TMP/key.pem" -in "$TMP/cert.pem" -name "$NAME" \
  -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1 \
  -passout "pass:$PASS" -out "$TMP/imt.p12"

# -T lets codesign use the key without a keychain prompt.
security import "$TMP/imt.p12" -k "$KEYCHAIN" -P "$PASS" -T /usr/bin/codesign >/dev/null

echo "Trusting the certificate for code signing (macOS asks for your password once)..."
security add-trusted-cert -r trustRoot -p codeSign -k "$KEYCHAIN" "$TMP/cert.pem"

if has_identity; then
  echo "Created signing identity \"$NAME\"."
else
  echo "Certificate imported but not yet valid for code signing; check Keychain Access." >&2
  exit 1
fi
