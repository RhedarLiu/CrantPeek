#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"

# Defaults package the egui shell, which is what CI and the existing bundle use.
# Override to package the GPUI shell into its own bundle, so macOS permissions
# can be granted to each separately while both exist:
#   PACKAGE=peek-gpui BIN=peek-gpui APP_NAME="Crant Peek GPUI.app" \
#   BUNDLE_ID=dev.crant.peek.gpui EXEC_NAME=CrantPeekGPUI tools/package-macos.sh
PACKAGE="${PACKAGE:-peek-app}"
BIN="${BIN:-peek-app}"
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
# Ad-hoc signature for local testing, not notarization or a trusted distribution signature.
/usr/bin/codesign --force --deep --sign - "$APP"
printf '\nBuilt local test app: %s (bundle id %s)\nNot notarized; not ready for public distribution.\n' "$APP" "$BUNDLE_ID"
