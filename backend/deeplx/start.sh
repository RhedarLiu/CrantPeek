#!/usr/bin/env bash
set -euo pipefail
SERVICE_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SERVICE_DIR"

if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
  echo "This package is for Linux x86_64. Use the matching official binary for other platforms." >&2
  exit 1
fi
BINARY="$SERVICE_DIR/deeplx_linux_amd64"
if [[ ! -f "$BINARY" ]]; then
  echo "The DeepLX executable is missing. Upload the complete backend folder." >&2
  exit 1
fi
[[ ! -f settings.env ]] || source settings.env

# An empty token disables upstream authentication. Do not read legacy token
# files or inherit TOKEN from the process manager when using the public service.
export TOKEN="${DEEPLX_TOKEN:-}"
export IP="${DEEPLX_BIND:-127.0.0.1}"
export PORT="${DEEPLX_PORT:-1188}"
export PROXY="${DEEPLX_PROXY:-}"

chmod u+x "$BINARY"
echo "Starting DeepLX at $IP:$PORT under MCSManager."
# Replace the shell so MCSManager controls the real foreground process.
exec "$BINARY"
