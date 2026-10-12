#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUTPUT="${1:-$ROOT/dist/macos-icon}"
SOURCE="$ROOT/assets/CrantPeek.icon"
CACHE="${MACOS_ICON_CACHE:-$ROOT/local-assets/macos-icon}"
DIGEST="$(python3 - "$SOURCE" <<'PY'
import hashlib, pathlib, sys
root = pathlib.Path(sys.argv[1])
digest = hashlib.sha256()
for path in sorted(root.rglob('*')):
    if path.is_file():
        digest.update(str(path.relative_to(root)).encode())
        digest.update(path.read_bytes())
print(digest.hexdigest())
PY
)"
mkdir -p "$OUTPUT"
if xcrun --find actool >/dev/null 2>&1; then
  xcrun actool "$SOURCE" --compile "$OUTPUT" \
    --output-format human-readable-text --notices --warnings --errors \
    --output-partial-info-plist "$OUTPUT/icon-info.plist" \
    --app-icon CrantPeek --include-all-app-icons --enable-on-demand-resources NO \
    --development-region en --target-device mac \
    --minimum-deployment-target 26.0 --platform macosx
  test -s "$OUTPUT/Assets.car"
  printf '%s\n' "$DIGEST" > "$OUTPUT/source.sha256"
elif [[ -f "$CACHE/source.sha256" && "$(cat "$CACHE/source.sha256")" == "$DIGEST" && -s "$CACHE/Assets.car" ]]; then
  cp "$CACHE/Assets.car" "$CACHE/icon-info.plist" "$CACHE/source.sha256" "$OUTPUT/"
  if [[ -f "$CACHE/CrantPeek.icns" ]]; then cp "$CACHE/CrantPeek.icns" "$OUTPUT/"; fi
else
  # Command Line Tools omit actool. CI supplies a source-matched native cache.
  rm -f "$OUTPUT/Assets.car" "$OUTPUT/icon-info.plist" "$OUTPUT/source.sha256"
  echo "No Xcode icon compiler or matching native cache; using the legacy icon."
fi
