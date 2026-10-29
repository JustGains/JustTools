[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string[]] $Path,
    [Parameter(Mandatory = $true)]
    [string] $ReportPath,
    [switch] $UpdateSignatures
)

# Scan the bytes that will ship. This never changes Defender preferences or
# exclusions, restores a quarantined file, or executes the scanned payload.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scannerCandidates = @(
    (Join-Path $env:ProgramData 'Microsoft\Windows Defender\Platform\CurrentVersion\MpCmdRun.exe'),
    (Join-Path $env:ProgramFiles 'Windows Defender\MpCmdRun.exe')
)
$scanner = $scannerCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
    Select-Object -First 1
if (!$scanner) { throw 'Microsoft Defender MpCmdRun.exe is required to verify a Windows release.' }

$status = Get-MpComputerStatus
if (!$status.AntivirusEnabled) {
    throw 'Microsoft Defender Antivirus is unavailable or inactive; the release has not been scanned.'
}
if ($UpdateSignatures) {
    & $scanner -SignatureUpdate
    if ($LASTEXITCODE -ne 0) { throw "Defender signature update failed with exit code $LASTEXITCODE." }
    $status = Get-MpComputerStatus
}

$report = [ordered]@{
    scannedAtUtc = [DateTime]::UtcNow.ToString('o')
    engineVersion = $status.AMEngineVersion
    signatureVersion = $status.AntivirusSignatureVersion
    signatureUpdatedAtUtc = $status.AntivirusSignatureLastUpdated.ToUniversalTime().ToString('o')
    success = $false
    files = @()
}
$failed = @()
foreach ($inputPath in $Path) {
    $artifact = Get-Item -LiteralPath $inputPath
    if ($artifact.PSIsContainer) { throw "Expected a release file, got a directory: $inputPath" }
    $hash = (Get-FileHash -LiteralPath $artifact.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    $signature = Get-AuthenticodeSignature -LiteralPath $artifact.FullName
    # For custom scans, DisableRemediation ignores exclusions and reports
    # detections through output/exit status, without quarantining the artifact.
    # A nonzero code means detection OR a scanner error; both block publication.
    $output = @(& $scanner -Scan -ScanType 3 -File $artifact.FullName -DisableRemediation 2>&1)
    $scanExitCode = $LASTEXITCODE
    $unchanged = (Get-FileHash -LiteralPath $artifact.FullName -Algorithm SHA256).Hash.ToLowerInvariant() -eq $hash
    $report.files += [ordered]@{
        name = $artifact.Name
        sha256 = $hash
        signatureStatus = $signature.Status.ToString()
        signer = $(if ($signature.SignerCertificate) { $signature.SignerCertificate.Subject } else { $null })
        exitCode = $scanExitCode
        unchanged = $unchanged
        output = ($output | Out-String).Trim()
    }
    $output | Write-Host
    if ($scanExitCode -ne 0 -or !$unchanged) { $failed += $artifact.Name }
}

$report.success = $failed.Count -eq 0 -and $report.files.Count -gt 0
$reportFile = [IO.Path]::GetFullPath($ReportPath)
[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($reportFile)) | Out-Null
[IO.File]::WriteAllText($reportFile, ($report | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($false))
if (!$report.success) {
    throw "Windows release verification failed for: $($failed -join ', '). See $reportFile."
}
Write-Host "Defender found no threats in $($report.files.Count) release files (signatures $($report.signatureVersion))."
