"""Copy formal release assets to the download backend, never log credentials.

Contracts: docs.gitcode.com/docs/apis/{post-api-v-5-repos-owner-repo-releases,
get-api-v-5-repos-owner-repo-releases-tag-upload-url}/
"""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

API = "https://api.gitcode.com/api/v5/repos/CrantAI/CrantPeek"
NAMES = ("Crant-Peek-macOS.zip", "Crant-Peek-Windows.zip",
         "Crant-Peek-Windows-Setup.exe", "SHA256SUMS.txt")
DOMAINS = ("gitcode.com", "gitcode.net", "atomgit.com", "myhuaweicloud.com")


class SyncError(RuntimeError):
    pass


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def public_url(raw):
    parsed = urllib.parse.urlsplit(raw)
    if (parsed.scheme != "https" or parsed.username or parsed.password
            or "access_token" in urllib.parse.parse_qs(parsed.query)
            or not any(parsed.hostname == d or (parsed.hostname or "").endswith("." + d)
                       for d in DOMAINS)):
        raise SyncError("Unexpected attachment host or private URL")
    return raw


class PublicRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        public_url(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def api(path, token, method="GET", body=None, **params):
    query = urllib.parse.urlencode({"access_token": token, **params})
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(API + path + "?" + query, data=data,
                                     method=method, headers={"Content-Type": "application/json"})
    try:
        # Prevent a redirect from forwarding the API token elsewhere.
        with urllib.request.build_opener(NoRedirect).open(request, timeout=60) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        raise SyncError(f"Download API {method} {path}: HTTP {error.code}; check repository permissions and mirrored tag") from None
    except (urllib.error.URLError, ValueError):
        raise SyncError("Download API unavailable or returned invalid metadata") from None


def digest_url(url):
    digest = hashlib.sha256()
    total = 0
    try:
        with urllib.request.build_opener(PublicRedirect).open(public_url(url), timeout=120) as response:
            while chunk := response.read(1024 * 1024):
                total += len(chunk)
                if total > 100 * 1024 * 1024:
                    raise SyncError("Attachment exceeds verification size limit")
                digest.update(chunk)
    except (urllib.error.URLError, ValueError):
        raise SyncError("Cannot verify public attachment download") from None
    return digest.hexdigest()


def upload(path, metadata):
    url = public_url(metadata["url"])
    # The storage signature may require headers beyond the documentation's
    # examples. Forward the returned upload headers, never our API credential.
    headers = metadata["headers"]
    if any(k.lower() in {"host", "cookie", "proxy-authorization"} for k in headers):
        raise SyncError("Unexpected storage upload header")
    print("Uploading:", path.name, "bytes:", path.stat().st_size,
          "host:", urllib.parse.urlsplit(url).hostname,
          "header names:", ",".join(sorted(headers)), flush=True)
    def quoted(value):
        if not isinstance(value, str) or any(c in value for c in "\r\n\0"):
            raise SyncError("Invalid signed upload configuration")
        return '"' + value.replace('\\', '\\\\').replace('"', '\\"') + '"'
    # IPv4 avoids unreachable IPv6 storage routes. Stream the file rather than
    # buffering it; config arrives via stdin so signatures do not enter argv.
    config = "url = " + quoted(url) + "\n" + "".join(
        "header = " + quoted(key + ": " + value) + "\n" for key, value in headers.items())
    with tempfile.TemporaryDirectory() as directory:
        result = subprocess.run([
            "curl", "--config", "-", "--silent", "--ipv4", "--globoff", "--proto", "=https",
            "--connect-timeout", "20", "--max-time", "240", "--request", "PUT",
            "--header", "Expect:", "--upload-file", str(path), "--output", str(Path(directory) / "response"),
            "--write-out", "%{http_code}|%{size_upload}|%{time_connect}",
        ], input=config, capture_output=True, text=True)
    parts = result.stdout.strip().split("|")
    code = parts[0]
    if result.returncode or not re.fullmatch(r"2\d\d", code):
        metrics = f", bytes sent {parts[1]}, connect seconds {parts[2]}" if len(parts) == 3 and all(re.fullmatch(r"[0-9.]+", p) for p in parts) else ""
        raise SyncError(f"Upload failed for {path.name}: curl exit {result.returncode}, HTTP {code if code.isdigit() else 'unknown'}{metrics}")


def gh(*args):
    result = subprocess.run(["gh", *args], capture_output=True, text=True)
    if result.returncode:
        raise SyncError("GitHub release lookup or asset download failed")
    return result.stdout


def synchronize():
    token = os.environ.get("GITCODE_TOKEN", "")
    if not token:
        raise SyncError("Configure GitHub Actions secret GITCODE_TOKEN, then rerun Sync download source")
    tag = os.environ.get("RELEASE_TAG", "")
    if not re.fullmatch(r"v\d+\.\d+\.\d+", tag):
        raise SyncError("Only a vX.Y.Z formal release tag can be synchronized")
    release = json.loads(gh("release", "view", tag, "--json", "name,body,isDraft,isPrerelease,assets"))
    if release["isDraft"] or release["isPrerelease"]:
        raise SyncError("Draft and preview releases cannot be synchronized")
    if not set(NAMES).issubset(a["name"] for a in release["assets"]):
        raise SyncError("All three executable packages and checksums are required")
    repo = os.environ["GH_REPO"]
    # Resolve the actual commit, including annotated tags. Never create a mirror
    # release pointing to whichever commit happens to be on its default branch.
    commit = json.loads(gh("api", f"repos/{repo}/commits/{tag}"))["sha"]
    path = "/releases/tags/" + tag
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        gh("release", "download", tag, "--dir", directory, *sum((["--pattern", n] for n in NAMES), []))
        hashes = {n: hashlib.sha256((root / n).read_bytes()).hexdigest() for n in NAMES}
        sums = dict(line.split(None, 1)[::-1] for line in (root / NAMES[-1]).read_text().splitlines())
        sums = {name.lstrip(" *"): value for name, value in sums.items()}
        if any(sums.get(n) != hashes[n] for n in NAMES[:-1]):
            raise SyncError("Source package checksum mismatch; nothing was uploaded")
        # The list endpoint distinguishes no release from auth/network failure.
        releases = api("/releases", token)
        current = tuple(map(int, tag[1:].split(".")))
        for item in releases:
            other = item.get("tag_name", "")
            if (not item.get("prerelease", True) and re.fullmatch(r"v\d+\.\d+\.\d+", other)
                    and tuple(map(int, other[1:].split("."))) > current):
                raise SyncError("A newer download release exists; refusing to replace latest with an older version")
        existing = next((r for r in releases if r.get("tag_name") == tag), None)
        if existing is None:
            api("/releases", token, "POST", {
                "tag_name": tag, "name": release["name"], "body": release["body"],
                "target_commitish": commit, "release_status": "pre",
            })
        metadata = api(path, token)
        for name in NAMES:
            attached = next((a for a in metadata.get("assets", []) if a.get("name") == name), None)
            if attached:
                if digest_url(attached["browser_download_url"]) != hashes[name]:
                    raise SyncError(f"Existing {name} differs; refusing to overwrite a published file")
            else:
                upload(root / name, api(f"/releases/{tag}/upload_url", token, file_name=name))
            print("Verified existing or uploaded:", name)
        # Wait for storage callbacks and verify every actual public file before
        # exposing the new release as latest to website visitors.
        for attempt in range(12):
            metadata = api(path, token)
            assets = {a.get("name"): a for a in metadata.get("assets", [])}
            if all(n in assets for n in NAMES):
                break
            if attempt == 11:
                raise SyncError("Attachments are not all visible yet; rerun synchronization")
            time.sleep(5)
        for name in NAMES:
            if digest_url(assets[name]["browser_download_url"]) != hashes[name]:
                raise SyncError(f"Public download checksum mismatch: {name}")
        api(f"/releases/{tag}", token, "PATCH", {
            "name": release["name"], "body": release["body"], "release_status": "latest",
        })
        print("Public downloads verified and published:", tag)


if __name__ == "__main__":
    try:
        synchronize()
    except (SyncError, KeyError, TypeError, json.JSONDecodeError) as error:
        # Never print raw request objects, signed URLs, API bodies or tracebacks.
        print("::error::" + (str(error) if isinstance(error, SyncError) else "Invalid release/upload metadata"))
        raise SystemExit(1)
