#!/usr/bin/env python3
"""Copy existing release packages into the static site's download directory."""

import argparse
import hashlib
from pathlib import Path
import os
import shutil
import zipfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--macos", type=Path, required=True, help="macOS app ZIP")
    parser.add_argument("--windows-portable", type=Path, required=True, help="Windows portable ZIP")
    parser.add_argument("--windows-installer", type=Path, required=True, help="Windows installer EXE")
    args = parser.parse_args()
    files = [
        (args.macos, "Crant-Peek-macOS.zip"),
        (args.windows_portable, "Crant-Peek-Windows.zip"),
        (args.windows_installer, "Crant-Peek-Windows-Setup.exe"),
    ]
    # Validate all inputs before replacing any deployed package.
    for source, name in files:
        if not source.is_file():
            parser.error(f"Missing package: {source}")
        if name.endswith(".zip"):
            try:
                with zipfile.ZipFile(source) as archive:
                    bad = archive.testzip()
                    if bad:
                        parser.error(f"Corrupt package: {source}: {bad}")
                    paths = archive.namelist()
                    if name == "Crant-Peek-macOS.zip":
                        valid = any(".app/Contents/MacOS/" in p for p in paths)
                    else:
                        valid = any(Path(p).name == "CrantPeek.exe" for p in paths)
                    if not valid:
                        parser.error(f"Application not found in package: {source}")
            except zipfile.BadZipFile:
                parser.error(f"Invalid ZIP: {source}")
        else:
            with source.open("rb") as executable:
                if executable.read(2) != b"MZ":
                    parser.error(f"Invalid Windows executable: {source}")

    output = Path(__file__).resolve().parent / "public" / "downloads"
    output.mkdir(parents=True, exist_ok=True)
    checksums = []
    for source, name in files:
        target = output / name
        temporary = output / (name + ".new")
        shutil.copyfile(source, temporary)
        temporary.chmod(0o644)
        digest = hashlib.sha256(temporary.read_bytes()).hexdigest()
        os.replace(temporary, target)
        checksums.append(f"{digest}  {name}")
        print(f"{name}: {target.stat().st_size:,} bytes")
    (output / "SHA256SUMS.txt").write_text("\n".join(checksums) + "\n")
    print(f"Ready to upload: {output}")


if __name__ == "__main__":
    main()
