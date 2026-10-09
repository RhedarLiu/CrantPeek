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
# Signing identity. Ad-hoc has no stable identity: its designated requirement
# is just `cdhash H"..."`, so TCC matches by hash and every rebuild loses the
# app's Accessibility, Input Monitoring and Screen Recording grants. A
# certificate's requirement is `identifier "..." and certificate leaf = H"..."`,
# which survives rebuilds.
#
# Locally the dev certificate is picked up automatically; CI has none, so it
# stays ad-hoc. Override with SIGN_IDENTITY=... or SIGN_IDENTITY=- to force
# ad-hoc.
if [[ -z "${SIGN_IDENTITY:-}" ]] && /usr/bin/security find-identity -p codesigning 2>/dev/null | grep -qF "Crant Peek Dev"; then
  SIGN_IDENTITY="Crant Peek Dev"
fi
SIGN_IDENTITY="${SIGN_IDENTITY:--}"
/usr/bin/codesign --force --deep --sign "$SIGN_IDENTITY" "$APP"
if [[ "$SIGN_IDENTITY" == "-" ]]; then
  printf '\nBuilt local test app: %s (bundle id %s)\nSigned ad-hoc: macOS will ask for permissions again after every rebuild.\nRun tools/dev-signing-identity.sh once to stop that.\nNot notarized; not ready for public distribution.\n' "$APP" "$BUNDLE_ID"
else
  printf '\nBuilt local test app: %s (bundle id %s)\nSigned with: %s (permissions survive rebuilds)\nNot notarized; not ready for public distribution.\n' "$APP" "$BUNDLE_ID" "$SIGN_IDENTITY"
fi
