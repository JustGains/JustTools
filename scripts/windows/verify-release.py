"""Bind a passing Defender report to the signed files in a Windows release ZIP."""

import hashlib
import json
import sys
from pathlib import Path
from zipfile import ZipFile

PACKAGE = "justtools-shell.msix"


def verify_release(archive: Path, allow_unsigned: bool = False) -> None:
    """Check the scan report against the archive.

    `allow_unsigned` accepts payloads that carry no signature at all, for a
    release the maintainer chose to publish unsigned. A broken or untrusted
    signature, a hash mismatch, or mixed publishers still fail.
    """
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
    payloads = ["just.exe", "justtools_shell.dll"]
    # Only signing produces the identity package, so an unsigned release has none.
    if not allow_unsigned or PACKAGE in records:
        payloads.append(PACKAGE)
    for name in (*payloads, archive.name):
        record = records.get(name)
        if record is None:
            raise ValueError(f"Defender report is missing {name}")
        if record["exitCode"] != 0 or record["unchanged"] is not True:
            raise ValueError(f"Defender did not verify {name}")
    if records[archive.name]["sha256"] != hashlib.sha256(archive.read_bytes()).hexdigest():
        raise ValueError(f"The scanned archive hash does not match {archive.name}")
    publishers = set()
    with ZipFile(archive) as contents:
        for name in payloads:
            record = records[name]
            if record["signatureStatus"] == "Valid" and record.get("signer"):
                publishers.add(record["signer"])
            elif not (allow_unsigned and record["signatureStatus"] == "NotSigned"):
                raise ValueError(f"A trusted signature is required for {name}")
            if record["sha256"] != hashlib.sha256(contents.read(name)).hexdigest():
                raise ValueError(f"The scanned payload hash does not match {name}")
    if len(publishers) > 1 or (not allow_unsigned and len(publishers) != 1):
        raise ValueError("Windows release files must have the same publisher")


if __name__ == "__main__":
    arguments = sys.argv[1:]
    allow_unsigned = "--allow-unsigned" in arguments
    archives = [argument for argument in arguments if argument != "--allow-unsigned"]
    if not archives:
        sys.exit("usage: verify-release.py [--allow-unsigned] ARCHIVE.zip [...]")
    for argument in archives:
        verify_release(Path(argument), allow_unsigned)
        kind = "payload hashes" if allow_unsigned else "signed payload hashes"
        print(f"Verified Windows scan report and {kind}: {argument}")
