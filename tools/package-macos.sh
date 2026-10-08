#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"
"$CARGO" build --release -p peek-app
APP="$ROOT/dist/Crant Peek.app"
# Never delete an existing app: install files in place; no unrelated path is modified.
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/target/release/peek-app" "$APP/Contents/MacOS/CrantPeek"
cp "$ROOT/assets/Info.plist" "$APP/Contents/Info.plist"
cp "$ROOT/LICENSE" "$APP/Contents/Resources/LICENSE"
if [[ -f "$ROOT/local-assets/ecdict.pkd" ]]; then
  cp "$ROOT/local-assets/ecdict.pkd" "$APP/Contents/Resources/ecdict.pkd"
  cp "$ROOT/assets/ECDICT-LICENSE" "$APP/Contents/Resources/ECDICT-LICENSE"
fi
# Ad-hoc signature for local testing, not notarization or a trusted distribution signature.
/usr/bin/codesign --force --deep --sign - "$APP"
printf '\nBuilt local test app: %s\nNot notarized; not ready for public distribution.\n' "$APP"
