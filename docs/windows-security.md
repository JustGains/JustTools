# Windows release verification

Tagged Windows releases require a trusted code-signing certificate. The native
workflow signs and timestamps `just.exe`, `justtools_shell.dll`, and the sparse
`justtools-shell.msix` identity package, then verifies their signatures. Set the
repository secrets `JUSTTOOLS_SIGNING_PFX` (base64 PFX) and
`JUSTTOOLS_SIGNING_PASSWORD`. The certificate must have its private key and
chain to a public root trusted by Windows. No key or password belongs in the
repository. Unsigned branch builds remain available as CI artifacts.

Both Windows build jobs update Microsoft Defender's security intelligence and
scan the executable, extension, optional identity package, and finished ZIP.
A detection, scan error, inactive antivirus, or failed signature verification
blocks publication. Each Windows archive ships a `.defender.json` sidecar with
the SHA-256 hashes, scan result, signature status, and engine/intelligence
versions. The publishing job checks that it describes the exact archived
payload and ZIP. Archive `.sha256` sidecars are calculated after signing.

The executable and extension also include matching company, product, and
version resources. Those fields identify the product; they are not a digital
signature or a guarantee against antivirus detections.

## Check a local build

Build both Windows artifacts, then scan them without changing Defender's
preferences, exclusions, or quarantine:

```powershell
cargo build --locked --release -p justtools -p justtools-shell
./scripts/windows/scan-release.ps1 `
  -Path target/release/just.exe,target/release/justtools_shell.dll `
  -ReportPath target/windows-security/defender.json
```

The custom scan uses `-DisableRemediation`: for this scan, exclusions are
ignored and detections are reported without quarantining the build. It does
not disable real-time protection. A nonzero scan exit code fails verification.
CI also passes `-UpdateSignatures` to require a successful intelligence update.
See [Microsoft's scanner documentation](https://learn.microsoft.com/en-us/defender-endpoint/command-line-arguments-microsoft-defender-antivirus).

## If Defender detects JustTools

Record the exact detection name, affected path, file SHA-256, intelligence
version, and whether detection occurred during a scan or after running a
command. An alert's generic malware description does not identify which
JustTools feature caused the classification. A rebuilt file passing a scan
does not establish that the older file was safe or that future definitions
will never detect the new one.

Compare the file with the matching release and its checksum. Submit the
affected release binary as a software developer through
[Microsoft's sample submission portal](https://www.microsoft.com/wdsi/filesubmission)
and track the analyst's determination. Use the existing CI/release artifact
when the installed copy is quarantined; do not restore it just to execute it.
Do not work around a detection by adding exclusions, disabling protection,
obfuscating code, or repeatedly changing builds until one is accepted.

Microsoft recommends consistent trusted code signing to establish publisher
identity, and submission for review to dispute a detection. Neither a signature
nor one successful scan guarantees that every machine or later intelligence
update will accept a file. This is distinct from SmartScreen's download
reputation warning. See the
[software developer FAQ](https://learn.microsoft.com/en-us/defender-xdr/developer-faq).
