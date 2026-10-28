#!/bin/bash
# Build isolated native selection fixtures. No real API calls or clipboard data.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
cargo build -p peek-gpui --offline
swiftc tools/selection-fixture.swift -o /tmp/CrantPeekSelectionFixture
python3 - <<'PY'
from pathlib import Path
import plistlib, shutil
fixture = Path('/tmp/CrantPeek Selection Fixture.app/Contents')
probe = Path('/tmp/CrantPeek Selection Probe.app/Contents')
for folder in (fixture, probe):
    (folder / 'MacOS').mkdir(parents=True, exist_ok=True)
shutil.copy2('/tmp/CrantPeekSelectionFixture', fixture / 'MacOS/Fixture')
(fixture / 'Info.plist').write_bytes(plistlib.dumps({
    'CFBundleExecutable': 'Fixture', 'CFBundleIdentifier': 'dev.crant.selection-fixture',
    'CFBundleName': 'CrantPeek Selection Fixture', 'CFBundlePackageType': 'APPL'
}))
shutil.copy2('target/debug/peek-gpui', probe / 'MacOS/CrantPeek')
info = plistlib.loads(Path('assets/Info.plist').read_bytes())
info['LSBackgroundOnly'] = True
info.pop('LSUIElement', None)
info.pop('LSEnvironment', None)
(probe / 'Info.plist').write_bytes(plistlib.dumps(info))
PY
# Use the same stable identity as the local application; never reset TCC.
codesign --force --deep --sign "${SIGN_IDENTITY:-Crant Peek Dev}" '/tmp/CrantPeek Selection Probe.app'
printf '%s\n' 'Open /tmp/CrantPeek Selection Fixture.app and click one scenario at a time.'
printf '%s\n' 'Each scenario runs five checks; see /tmp/crantpeek-selection-<scenario>.log.'
printf '%s\n' 'All copy tests use a dedicated pasteboard. The keyboard scenario needs real foreground focus.'
