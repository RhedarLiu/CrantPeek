#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"

# The GPUI shell is the application. The overrides exist so an alternative
# bundle can be built side by side (for example while comparing UI crates)
# without touching this one:
#   PACKAGE=... BIN=... APP_NAME=... BUNDLE_ID=... EXEC_NAME=... tools/package-macos.sh
PACKAGE="${PACKAGE:-peek-gpui}"
BIN="${BIN:-peek-gpui}"
APP_NAME="${APP_NAME:-Crant Peek.app}"
BUNDLE_ID="${BUNDLE_ID:-dev.crant.peek}"
EXEC_NAME="${EXEC_NAME:-CrantPeek}"

# Signing identity. Ad-hoc has no stable identity: its designated requirement
# is just `cdhash H"..."`, so TCC matches by hash and every rebuild loses the
# app's Accessibility, Input Monitoring and Screen Recording grants. A
# certificate's requirement is `identifier "..." and certificate leaf = H"..."`,
# which survives rebuilds.
#
# Local builds must keep a stable identity. A sandbox may report no identities
# even when the login keychain contains one; never silently replace a signed
# development app with ad-hoc signing. CI or SIGN_IDENTITY=- opts in explicitly.
if [[ -z "${SIGN_IDENTITY:-}" ]]; then
  IDENTITIES="$(/usr/bin/security find-identity -p codesigning 2>/dev/null)" || {
    echo "Cannot read signing identities. Run packaging with access to the login keychain." >&2
    exit 1
  }
  if [[ "$IDENTITIES" == *'"Crant Peek Dev"'* ]]; then
    SIGN_IDENTITY="Crant Peek Dev"
  elif [[ "${CI:-}" == "true" ]]; then
    SIGN_IDENTITY="-"
  else
    echo "Crant Peek Dev signing identity is unavailable. Run packaging outside the sandbox, or set up tools/dev-signing-identity.sh once." >&2
    echo "For an intentional ad-hoc build only, set SIGN_IDENTITY=- (permissions will need granting again)." >&2
    exit 1
  fi
fi

"$CARGO" build --release -p "$PACKAGE"
APP="$ROOT/dist/$APP_NAME"
# Never delete an existing app: install files in place; no unrelated path is modified.
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/target/release/$BIN" "$APP/Contents/MacOS/$EXEC_NAME"
cp "$ROOT/assets/Info.plist" "$APP/Contents/Info.plist"
cp "$ROOT/LICENSE" "$APP/Contents/Resources/LICENSE"
if [[ "$BUNDLE_ID" != "dev.crant.peek" || "$EXEC_NAME" != "CrantPeek" ]]; then
  /usr/bin/plutil -replace CFBundleIdentifier -string "$BUNDLE_ID" "$APP/Contents/Info.plist"
  /usr/bin/plutil -replace CFBundleExecutable -string "$EXEC_NAME" "$APP/Contents/Info.plist"
fi
if [[ -f "$ROOT/local-assets/ecdict.pkd" ]]; then
  cp "$ROOT/local-assets/ecdict.pkd" "$APP/Contents/Resources/ecdict.pkd"
  cp "$ROOT/assets/ECDICT-LICENSE" "$APP/Contents/Resources/ECDICT-LICENSE"
fi
if [[ "$SIGN_IDENTITY" == "Developer ID Application:"* ]]; then
  /usr/bin/codesign --force --deep --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP"
else
  /usr/bin/codesign --force --deep --sign "$SIGN_IDENTITY" "$APP"
fi
if [[ "$SIGN_IDENTITY" == "-" ]]; then
  printf '\nBuilt local test app: %s (bundle id %s)\nSigned ad-hoc: macOS will ask for permissions again after every rebuild.\nRun tools/dev-signing-identity.sh once to stop that.\nNot notarized; not ready for public distribution.\n' "$APP" "$BUNDLE_ID"
else
  printf '\nBuilt local test app: %s (bundle id %s)\nSigned with: %s (stable development identity)\nNot notarized; not ready for public distribution.\n' "$APP" "$BUNDLE_ID" "$SIGN_IDENTITY"
fi

# A stable requirement is necessary but the certificate must also be trusted.
# Verify without silently modifying the user's keychain trust settings.
if [[ "$SIGN_IDENTITY" != "-" ]] && ! /usr/bin/codesign --verify --deep --strict "$APP"; then
  echo "Signature verification failed. Check the development certificate's code-signing trust before relying on persistent permissions." >&2
fi
