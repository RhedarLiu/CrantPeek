"""Offline checks for publication ordering, retries and credential boundaries."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock, patch

spec = importlib.util.spec_from_file_location("sync_downloads", Path(__file__).with_name("sync-downloads.py"))
sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sync)


class SyncTests(unittest.TestCase):
    def test_signed_storage_headers_are_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "test.zip"
            path.write_bytes(b"archive")
            metadata = {"url": "https://storage.myhuaweicloud.com/upload?signed=example",
                        "headers": {"Content-Type": "application/zip", "x-obs-date": "example",
                                    "Authorization": "OBS storage-signature"}}
            result = MagicMock(returncode=0, stdout="200|7|0.02")
            with patch.object(sync.subprocess, "run", return_value=result) as run:
                sync.upload(path, metadata)
            args = run.call_args.args[0]
            config = run.call_args.kwargs["input"]
            self.assertIn('header = "Authorization: OBS storage-signature"', config)
            self.assertIn('header = "x-obs-date: example"', config)
            self.assertNotIn("OBS storage-signature", " ".join(args))
            self.assertIn("--ipv4", args)
            self.assertIn("PUT", args)

    def test_private_and_foreign_urls_are_rejected(self):
        for url in ("http://gitcode.com/a", "https://gitcode.com.evil.test/a",
                    "https://user:password@gitcode.com/a", "https://gitcode.com/a?access_token=secret"):
            with self.assertRaises(sync.SyncError):
                sync.public_url(url)

    def test_no_credential_does_not_start_network_operations(self):
        with patch.dict(os.environ, {"GITCODE_TOKEN": ""}), patch.object(sync, "api") as api:
            with self.assertRaises(sync.SyncError):
                sync.synchronize()
            api.assert_not_called()

    def run_fixture(self, corrupt=False, newer=False):
        content = {name: name.encode() for name in sync.NAMES[:-1]}
        hashes = {name: hashlib.sha256(data).hexdigest() for name, data in content.items()}
        content[sync.NAMES[-1]] = "".join(f"{value}  {name}\n" for name, value in hashes.items()).encode()
        hashes[sync.NAMES[-1]] = hashlib.sha256(content[sync.NAMES[-1]]).hexdigest()
        uploaded, mutations = set(), []
        if corrupt:
            uploaded.add(sync.NAMES[0])

        def gh(*args):
            if args[0] == "api":
                return json.dumps({"sha": "1" * 40})
            if args[1] == "download":
                root = Path(args[args.index("--dir") + 1])
                for name, data in content.items():
                    (root / name).write_bytes(data)
                return ""
            return json.dumps({"name": "Version", "body": "Notes", "isDraft": False,
                               "isPrerelease": False, "assets": [{"name": n} for n in sync.NAMES]})

        def api(path, token, method="GET", body=None, **params):
            if method != "GET":
                mutations.append((method, body))
                return {}
            if path == "/releases":
                return [{"tag_name": "v0.2.0", "prerelease": False}] if newer else []
            if path.endswith("upload_url"):
                return {"name": params["file_name"]}
            return {"assets": [{"name": n, "browser_download_url": "https://gitcode.com/" + n}
                               for n in uploaded]}

        def upload(path, metadata):
            uploaded.add(metadata["name"])

        def digest(url):
            name = url.rsplit("/", 1)[1]
            return "wrong" if corrupt else hashes[name]

        with patch.dict(os.environ, {"GITCODE_TOKEN": "test-only", "RELEASE_TAG": "v0.1.0", "GH_REPO": "test/repo"}), \
                patch.object(sync, "gh", gh), patch.object(sync, "api", api), \
                patch.object(sync, "upload", upload), patch.object(sync, "digest_url", digest):
            if corrupt or newer:
                with self.assertRaises(sync.SyncError):
                    sync.synchronize()
            else:
                sync.synchronize()
        return mutations, uploaded

    def test_publish_only_after_all_attachments_are_verified(self):
        mutations, uploaded = self.run_fixture()
        self.assertEqual(uploaded, set(sync.NAMES))
        self.assertEqual(mutations[0][1]["release_status"], "pre")
        self.assertEqual(mutations[-1][1]["release_status"], "latest")

    def test_conflicting_attachment_never_becomes_latest(self):
        mutations, _ = self.run_fixture(corrupt=True)
        self.assertFalse(any(method == "PATCH" for method, _ in mutations))

    def test_backfill_cannot_replace_newer_version(self):
        mutations, _ = self.run_fixture(newer=True)
        self.assertEqual(mutations, [])


if __name__ == "__main__":
    unittest.main()
