#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUTPUT="$ROOT/dist/官网部署"

# Retain both existing upload layouts, excluding obsolete local binaries.
mkdir -p "$OUTPUT/public"
rsync -a --exclude '/downloads/' "$ROOT/website/public/" "$OUTPUT/"
rsync -a --exclude '/downloads/' "$ROOT/website/public/" "$OUTPUT/public/"
cp "$ROOT/website/nginx.conf" "$OUTPUT/nginx.conf"
cp "$ROOT/website/部署说明.md" "$OUTPUT/部署说明.md"

if [[ -d "$OUTPUT/downloads" || -d "$OUTPUT/public/downloads" ]]; then
  echo "Legacy downloads directory found; move it out before deploying." >&2
  exit 1
fi
printf 'Website ready: %s\nDownloads use the latest mirrored release.\n' "$OUTPUT"
