"""Check publication guards with synthetic scan reports, without running an AV."""

import hashlib
import json
import runpy
import tempfile
import unittest
from pathlib import Path
from zipfile import ZipFile

verify_release = runpy.run_path(str(Path(__file__).with_name("verify-release.py")))["verify_release"]


class ReleaseVerificationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.archive = Path(self.directory.name, "justtools-windows-x64.zip")
        self.payloads = {
            "just.exe": b"synthetic executable",
            "justtools_shell.dll": b"synthetic extension",
            "justtools-shell.msix": b"synthetic identity",
        }
        self.write_archive()
        files = []
        for name, contents in (*self.payloads.items(), (self.archive.name, self.archive.read_bytes())):
            files.append({
                "name": name,
                "sha256": hashlib.sha256(contents).hexdigest(),
                "signatureStatus": "Valid" if name != self.archive.name else "NotSigned",
                "signer": "CN=Fixture publisher" if name != self.archive.name else None,
                "exitCode": 0,
                "unchanged": True,
            })
        self.report = {
            "success": True,
            "engineVersion": "fixture-engine",
            "signatureVersion": "fixture-intelligence",
            "scannedAtUtc": "2026-10-28T00:00:00Z",
            "files": files,
        }

    def write_archive(self):
        with ZipFile(self.archive, "w") as archive:
            for name, contents in self.payloads.items():
                archive.writestr(name, contents)

    def check(self):
        Path(f"{self.archive}.defender.json").write_text(json.dumps(self.report), encoding="utf-8")
        verify_release(self.archive)

    def test_accepts_matching_payloads_and_archive(self):
        self.check()

    def test_rejects_archive_changed_after_scan(self):
        with self.archive.open("ab") as archive:
            archive.write(b"changed after scan")
        with self.assertRaisesRegex(ValueError, "archive hash"):
            self.check()

    def test_rejects_payload_substitution_even_with_updated_zip_hash(self):
        self.payloads["just.exe"] = b"different executable"
        self.write_archive()
        self.report["files"][-1]["sha256"] = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        with self.assertRaisesRegex(ValueError, "payload hash"):
            self.check()

    def test_rejects_unsigned_executable(self):
        self.report["files"][0]["signatureStatus"] = "NotSigned"
        with self.assertRaisesRegex(ValueError, "trusted signature"):
            self.check()

    def test_rejects_scan_errors_and_files_changed_during_scan(self):
        for field, value in (("exitCode", 2), ("unchanged", False)):
            with self.subTest(field=field):
                record = self.report["files"][0]
                original = record[field]
                record[field] = value
                with self.assertRaisesRegex(ValueError, "did not verify"):
                    self.check()
                record[field] = original

    def test_rejects_different_publishers(self):
        self.report["files"][1]["signer"] = "CN=Another publisher"
        with self.assertRaisesRegex(ValueError, "same publisher"):
            self.check()

    def test_rejects_failed_or_incomplete_report(self):
        self.report["success"] = False
        with self.assertRaisesRegex(ValueError, "verification failed"):
            self.check()
        self.report["success"] = True
        del self.report["signatureVersion"]
        with self.assertRaisesRegex(ValueError, "missing signatureVersion"):
            self.check()

    def test_rejects_duplicate_file_records(self):
        self.report["files"].append(self.report["files"][0].copy())
        with self.assertRaisesRegex(ValueError, "duplicate file names"):
            self.check()


if __name__ == "__main__":
    unittest.main()
