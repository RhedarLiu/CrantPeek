#!/bin/bash
# Creates a self-signed code signing certificate for local development.
#
# Why this exists: TCC (Accessibility, Input Monitoring, Screen Recording)
# matches an ad-hoc signed app by its cdhash, so *every* rebuild is a new
# identity and the user has to remove and re-add the app in System Settings.
# A certificate gives the app a designated requirement based on the
# certificate and bundle id instead of a hash, so a grant survives rebuilds.
# Shipped apps use a Developer ID; this is the local-development equivalent.
#
# Idempotent: does nothing when the identity already exists.
set -euo pipefail

NAME="${SIGN_IDENTITY:-Crant Peek Dev}"
KEYCHAIN="${KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}"
PASSWORD="${P12_PASSWORD:-crantpeek-local}"

if /usr/bin/security find-identity -v -p codesigning "$KEYCHAIN" 2>/dev/null | grep -qF "$NAME"; then
  echo "signing identity already present: $NAME"
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

cat > "$WORK/cert.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $NAME
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
EOF

/usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -keyout "$WORK/key.pem" -out "$WORK/cert.pem" -config "$WORK/cert.cnf" 2>/dev/null
/usr/bin/openssl pkcs12 -export -inkey "$WORK/key.pem" -in "$WORK/cert.pem" \
  -out "$WORK/identity.p12" -passout "pass:$PASSWORD" -name "$NAME" 2>/dev/null

# -T /usr/bin/codesign lets codesign use the key without a per-use prompt.
/usr/bin/security import "$WORK/identity.p12" -k "$KEYCHAIN" -P "$PASSWORD" \
  -T /usr/bin/codesign -T /usr/bin/security >/dev/null

# codesign accepts the identity without this, but macOS only treats the
# signature as valid once the certificate is trusted for code signing. Best
# effort: a locked keychain must not fail the setup.
/usr/bin/security add-trusted-cert -r trustRoot -p codeSign -k "$KEYCHAIN" \
  "$WORK/cert.pem" >/dev/null 2>&1 || echo "note: trust could not be set automatically" >&2

echo "created signing identity: $NAME"
if /usr/bin/security find-identity -v -p codesigning "$KEYCHAIN" 2>/dev/null | grep -qF "$NAME"; then
  echo "identity is valid for code signing"
else
  echo "note: find-identity does not list it as valid yet; codesign still accepts it." >&2
fi
