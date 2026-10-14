"""Emergency local transfer; the API token stays in Actions, requests use CMS encryption."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import urllib.request

spec = importlib.util.spec_from_file_location("download_sync", Path(__file__).with_name("sync-downloads.py"))
sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sync)


def validate_request(item):
    if (item.get("name") not in sync.NAMES or
            not re.fullmatch(r"[0-9a-f]{64}", item.get("sha256", ""))):
        raise sync.SyncError("Invalid transfer file identity")
    metadata = item["upload"]
    sync.public_url(metadata["url"])
    headers = metadata["headers"]
    if not isinstance(headers, dict) or any(
            not isinstance(k, str) or not isinstance(v, str) or
            k.lower() in {"host", "cookie", "proxy-authorization"} or
            any(c in k + v for c in "\r\n\0") for k, v in headers.items()):
        raise sync.SyncError("Invalid transfer upload headers")


def encrypt(payload, certificate, output):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        cert = root / "public.pem"
        cert.write_text(certificate)
        data = root / "requests.json"
        data.write_text(json.dumps(payload))
        result = subprocess.run(["openssl", "cms", "-encrypt", "-binary", "-aes256",
                                 "-outform", "DER", "-in", str(data), "-out", str(output),
                                 str(cert)], capture_output=True)
        if result.returncode:
            raise sync.SyncError("Cannot encrypt temporary upload requests")


def prepare(output):
    tag, token = os.environ["RELEASE_TAG"], os.environ.get("GITCODE_TOKEN", "")
    certificate = os.environ.get("UPLOAD_CERTIFICATE", "")
    if not token or not re.fullmatch(r"v\d+\.\d+\.\d+", tag) or not certificate:
        raise sync.SyncError("Formal release, API credential and public certificate are required")
    release = json.loads(sync.gh("release", "view", tag, "--json", "isDraft,isPrerelease,assets"))
    if release["isDraft"] or release["isPrerelease"] or not set(sync.NAMES).issubset(
            a["name"] for a in release["assets"]):
        raise sync.SyncError("Only complete formal releases can be transferred")
    metadata = sync.api("/releases/tags/" + tag, token)
    payload = {"destination": sync.API, "tag": tag, "files": []}
    with tempfile.TemporaryDirectory() as directory:
        sync.gh("release", "download", tag, "--dir", directory,
                *sum((["--pattern", name] for name in sync.NAMES), []))
        root = Path(directory)
        hashes = {name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in sync.NAMES}
        sums = {name.lstrip(" *"): value for value, name in
                (line.split(None, 1) for line in (root / "SHA256SUMS.txt").read_text().splitlines())}
        if any(sums.get(name) != hashes[name] for name in sync.NAMES[:-1]):
            raise sync.SyncError("Source package checksum mismatch")
        for name in sync.NAMES:
            existing = next((a for a in metadata.get("assets", []) if a.get("name") == name), None)
            if existing:
                if sync.digest_url(existing["browser_download_url"]) != hashes[name]:
                    raise sync.SyncError("An existing attachment differs from the source package")
                continue
            item = {"name": name, "sha256": hashes[name],
                    "upload": sync.api(f"/releases/{tag}/upload_url", token, file_name=name)}
            validate_request(item)
            payload["files"].append(item)
        # Never export the repository API token, including inside ciphertext.
        if token in json.dumps(payload):
            raise sync.SyncError("API credential appeared in transfer metadata")
        output.parent.mkdir(parents=True, exist_ok=True)
        encrypt(payload, certificate, output)
        print("Encrypted transfer prepared:", ", ".join(x["name"] for x in payload["files"]))


def upload(requests, private_key, packages):
    result = subprocess.run(["openssl", "cms", "-decrypt", "-binary", "-inform", "DER",
                             "-in", str(requests), "-inkey", str(private_key)], capture_output=True)
    if result.returncode:
        raise sync.SyncError("Cannot decrypt the temporary upload requests")
    payload = json.loads(result.stdout)
    if payload.get("destination") != sync.API or not re.fullmatch(r"v\d+\.\d+\.\d+", payload.get("tag", "")):
        raise sync.SyncError("Unexpected transfer destination")
    # Validate all source files before making any write request.
    for item in payload["files"]:
        validate_request(item)
        path = packages / item["name"]
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != item["sha256"]:
            raise sync.SyncError("Local package differs from the formal release")
    with urllib.request.build_opener(sync.PublicRedirect).open(
            sync.API + "/releases/tags/" + payload["tag"], timeout=60) as response:
        metadata = json.load(response)
    for item in payload["files"]:
        existing = next((a for a in metadata.get("assets", []) if a.get("name") == item["name"]), None)
        if existing:
            if sync.digest_url(existing["browser_download_url"]) != item["sha256"]:
                raise sync.SyncError("Existing attachment differs; refusing to overwrite")
        else:
            sync.upload(packages / item["name"], item["upload"])
        print("Transferred:", item["name"], flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prep = commands.add_parser("prepare")
    prep.add_argument("--output", type=Path, required=True)
    client = commands.add_parser("upload")
    client.add_argument("--requests", type=Path, required=True)
    client.add_argument("--private-key", type=Path, required=True)
    client.add_argument("--packages", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.command == "prepare":
            prepare(args.output)
        else:
            upload(args.requests, args.private_key, args.packages)
    except (sync.SyncError, KeyError, TypeError, ValueError, OSError):
        # Exception values can contain signed storage metadata; do not print them.
        print("Transfer failed; check source hashes, certificate and upload connectivity")
        raise SystemExit(1)
