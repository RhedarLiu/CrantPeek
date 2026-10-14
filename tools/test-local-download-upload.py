"""Offline checks for the encrypted, release-bound local upload fallback."""
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("local_upload", Path(__file__).with_name("local-download-upload.py"))
transfer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(transfer)


class TransferTests(unittest.TestCase):
    def item(self):
        return {"name": "Crant-Peek-macOS.zip", "sha256": hashlib.sha256(b"release").hexdigest(),
                "upload": {"url": "https://file.gitcode.com/object?signature=test-only",
                           "headers": {"Content-Type": "application/zip"}}}

    def test_only_release_files_and_public_storage_hosts_are_allowed(self):
        transfer.validate_request(self.item())
        for changes in ({"name": "../private.pem"}, {"sha256": "unknown"},
                        {"upload": {"url": "https://foreign.test/a", "headers": {}}},
                        {"upload": {"url": "https://file.gitcode.com/a?access_token=private", "headers": {}}},
                        {"upload": {"url": "https://file.gitcode.com/a", "headers": {"Cookie": "private"}}}):
            with self.assertRaises(transfer.sync.SyncError):
                transfer.validate_request(self.item() | changes)

    def test_a_different_local_package_is_never_uploaded(self):
        payload = {"destination": transfer.sync.API, "tag": "v0.1.1", "files": [self.item()]}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / self.item()["name"]).write_bytes(b"different executable")
            with patch.object(transfer.subprocess, "run") as decrypt, \
                    patch.object(transfer.sync, "upload") as upload, \
                    patch.object(transfer.urllib.request, "build_opener") as network:
                decrypt.return_value.returncode = 0
                decrypt.return_value.stdout = json.dumps(payload).encode()
                with self.assertRaises(transfer.sync.SyncError):
                    transfer.upload(root / "encrypted.der", root / "private.pem", root)
                network.assert_not_called()
                upload.assert_not_called()

    @unittest.skipUnless(shutil.which("openssl"), "OpenSSL is required for the Linux/macOS transfer helper")
    def test_requests_remain_encrypted_until_the_local_key_decrypts_them(self):
        payload = {"destination": transfer.sync.API, "tag": "v0.1.1", "files": [self.item()]}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            key, cert, encrypted = root / "private.pem", root / "public.pem", root / "transfer.der"
            subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                            "-subj", "/CN=TestOnly", "-keyout", str(key), "-out", str(cert)],
                           check=True, capture_output=True)
            transfer.encrypt(payload, cert.read_text(), encrypted)
            self.assertNotIn(b"signature=test-only", encrypted.read_bytes())
            result = subprocess.run(["openssl", "cms", "-decrypt", "-binary", "-inform", "DER",
                                     "-in", str(encrypted), "-inkey", str(key)], check=True, capture_output=True)
            self.assertEqual(json.loads(result.stdout), payload)


if __name__ == "__main__":
    unittest.main()
