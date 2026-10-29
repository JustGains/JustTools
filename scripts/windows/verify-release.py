"""Bind a passing Defender report to the signed files in a Windows release ZIP."""

import hashlib
import json
import sys
from pathlib import Path
from zipfile import ZipFile


def verify_release(archive: Path) -> None:
    report = json.loads(Path(f"{archive}.defender.json").read_text(encoding="utf-8"))
    if report.get("success") is not True:
        raise ValueError(f"Defender verification failed for {archive.name}")
    for field in ("engineVersion", "signatureVersion", "scannedAtUtc"):
        if not report.get(field):
            raise ValueError(f"Defender report is missing {field}")
    entries = report["files"]
    records = {item["name"]: item for item in entries}
    if len(records) != len(entries):
        raise ValueError("Defender report contains duplicate file names")
    payloads = ("just.exe", "justtools_shell.dll", "justtools-shell.msix")
    for name in (*payloads, archive.name):
        record = records[name]
        if record["exitCode"] != 0 or record["unchanged"] is not True:
            raise ValueError(f"Defender did not verify {name}")
    if records[archive.name]["sha256"] != hashlib.sha256(archive.read_bytes()).hexdigest():
        raise ValueError(f"The scanned archive hash does not match {archive.name}")
    publishers = set()
    with ZipFile(archive) as contents:
        for name in payloads:
            record = records[name]
            if record["signatureStatus"] != "Valid" or not record.get("signer"):
                raise ValueError(f"A trusted signature is required for {name}")
            publishers.add(record["signer"])
            if record["sha256"] != hashlib.sha256(contents.read(name)).hexdigest():
                raise ValueError(f"The scanned payload hash does not match {name}")
    if len(publishers) != 1:
        raise ValueError("Windows release files must have the same publisher")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit("usage: verify-release.py ARCHIVE.zip [...]")
    for argument in sys.argv[1:]:
        verify_release(Path(argument))
        print(f"Verified Windows scan report and signed payload hashes: {argument}")
