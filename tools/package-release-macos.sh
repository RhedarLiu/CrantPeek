#!/bin/bash
# Requires the user's own Developer ID and a preconfigured notarytool profile.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
: "${SIGN_IDENTITY:?Set SIGN_IDENTITY to your Developer ID Application identity}"
: "${NOTARY_PROFILE:?Set NOTARY_PROFILE to your notarytool keychain profile}"
[[ "$SIGN_IDENTITY" == "Developer ID Application:"* ]] || { echo 'A Developer ID Application identity is required.' >&2; exit 1; }
export SIGN_IDENTITY
bash tools/package-macos.sh
APP="$ROOT/dist/Crant Peek.app"
ditto -c -k --sequesterRsrc --keepParent "$APP" dist/Crant-Peek-notary.zip
xcrun notarytool submit dist/Crant-Peek-notary.zip --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple "$APP"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cp -R "$APP" "$WORK/Crant Peek.app"
ln -s /Applications "$WORK/Applications"
hdiutil create -volname 'Crant Peek' -srcfolder "$WORK" -ov -format UDZO dist/Crant-Peek-macOS.dmg
codesign --sign "$SIGN_IDENTITY" --timestamp dist/Crant-Peek-macOS.dmg
xcrun notarytool submit dist/Crant-Peek-macOS.dmg --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple dist/Crant-Peek-macOS.dmg
codesign --verify --deep --strict "$APP"
echo 'Built signed and notarized dist/Crant-Peek-macOS.dmg'
