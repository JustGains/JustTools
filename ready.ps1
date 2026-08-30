# JustReady bootstrap for Windows.
# Run remotely:
#   irm https://raw.githubusercontent.com/JustGains/JustTools/main/ready.ps1 | iex

& {
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Test-Enabled([string] $Value) {
    return $Value -match '^(?i:1|true|yes|on)$'
}

function Get-Setting([string] $Name) {
    return [Environment]::GetEnvironmentVariable($Name)
}

function Get-ReleaseFile([string] $Uri, [string] $Destination) {
    Write-Host "Downloading $Uri"
    $parameters = @{
        Uri = $Uri
        OutFile = $Destination
        ErrorAction = "Stop"
    }
    if ($PSVersionTable.PSVersion.Major -lt 6) {
        $parameters.UseBasicParsing = $true
    }
    Invoke-WebRequest @parameters
}

if (![Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
        [Runtime.InteropServices.OSPlatform]::Windows)) {
    throw "ready.ps1 is for Windows. On macOS or Linux, use ready.sh."
}

$architecture = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
$architecture = switch ($architecture) {
    "x64" { "x64" }
    "arm64" { "arm64" }
    default { throw "JustTools does not publish a Windows build for architecture '$architecture'." }
}

$repository = Get-Setting "JUSTTOOLS_REPO"
if ([string]::IsNullOrWhiteSpace($repository)) {
    $repository = "JustGains/JustTools"
}
$version = Get-Setting "JUSTTOOLS_VERSION"
$asset = "justtools-windows-$architecture.zip"
$archiveOverride = Get-Setting "JUSTTOOLS_ARCHIVE"
$skipVerification = Test-Enabled (Get-Setting "JUSTTOOLS_SKIP_VERIFY")
$skipLaunch = Test-Enabled (Get-Setting "JUSTREADY_NO_RUN")
$skipPath = Test-Enabled (Get-Setting "JUSTTOOLS_NO_PATH")

$temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\', '/')
$workDirectory = Join-Path $temporaryRoot ("justtools-ready-" + [Guid]::NewGuid().ToString("N"))
$archive = Join-Path $workDirectory $asset
$checksum = "$archive.sha256"
$extractDirectory = Join-Path $workDirectory "extract"

New-Item -ItemType Directory -Path $extractDirectory -Force | Out-Null
try {
    if (![string]::IsNullOrWhiteSpace($archiveOverride)) {
        $sourceArchive = (Resolve-Path -LiteralPath $archiveOverride).Path
        Copy-Item -LiteralPath $sourceArchive -Destination $archive
        if (!$skipVerification) {
            $sourceChecksum = "$sourceArchive.sha256"
            if (!(Test-Path -LiteralPath $sourceChecksum -PathType Leaf)) {
                throw "Missing checksum sidecar: $sourceChecksum"
            }
            Copy-Item -LiteralPath $sourceChecksum -Destination $checksum
        }
    } else {
        if ([string]::IsNullOrWhiteSpace($version) -or $version -eq "latest") {
            $releaseBase = "https://github.com/$repository/releases/latest/download"
        } else {
            if (!$version.StartsWith("v")) {
                $version = "v$version"
            }
            $releaseBase = "https://github.com/$repository/releases/download/$version"
        }
        Get-ReleaseFile "$releaseBase/$asset" $archive
        if (!$skipVerification) {
            Get-ReleaseFile "$releaseBase/$asset.sha256" $checksum
        }
    }

    if (!$skipVerification) {
        $expected = ((Get-Content -LiteralPath $checksum -Raw) -split '\s+')[0].ToLowerInvariant()
        if ($expected -notmatch '^[0-9a-f]{64}$') {
            throw "The release checksum is malformed."
        }
        $actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected) {
            throw "Checksum verification failed for $asset."
        }
        Write-Host "Verified $asset"
    }

    Expand-Archive -LiteralPath $archive -DestinationPath $extractDirectory -Force
    $staged = Join-Path $extractDirectory "just.exe"
    if (!(Test-Path -LiteralPath $staged -PathType Leaf)) {
        $candidate = Get-ChildItem -LiteralPath $extractDirectory -Filter "just.exe" -File -Recurse |
            Select-Object -First 1
        if ($null -eq $candidate) {
            throw "$asset does not contain just.exe."
        }
        $staged = $candidate.FullName
    }

    & $staged ready --help *> $null
    if ($LASTEXITCODE -ne 0) {
        throw "The selected JustTools release predates JustReady; no existing installation was changed."
    }

    $binDirectory = Get-Setting "JUSTTOOLS_BIN"
    if ([string]::IsNullOrWhiteSpace($binDirectory)) {
        if (Test-Path -LiteralPath "C:\cmd\bin" -PathType Container) {
            $binDirectory = "C:\cmd\bin"
        } else {
            $binDirectory = Join-Path ([Environment]::GetFolderPath("LocalApplicationData")) "JustTools\bin"
        }
    }

    $installArguments = @("install", "--bin-dir", $binDirectory, "--yes")
    if ($skipPath) {
        $installArguments += "--no-path"
    }
    & $staged @installArguments
    if ($LASTEXITCODE -ne 0) {
        throw "JustTools installation failed with exit code $LASTEXITCODE."
    }

    $ready = Join-Path $binDirectory "justready.exe"
    if (!(Test-Path -LiteralPath $ready -PathType Leaf)) {
        throw "Installation completed without the expected JustReady alias: $ready"
    }

    if ($skipLaunch -or [Console]::IsInputRedirected -or [Console]::IsOutputRedirected) {
        Write-Host "JustReady is installed at $ready"
        if (!$skipLaunch) {
            Write-Host "Run 'justready' from a new terminal to open it."
        }
    } else {
        & $ready
        if ($LASTEXITCODE -ne 0) {
            throw "JustReady exited with code $LASTEXITCODE."
        }
    }
} finally {
    $resolvedWork = [IO.Path]::GetFullPath($workDirectory)
    $safePrefix = $temporaryRoot + [IO.Path]::DirectorySeparatorChar + "justtools-ready-"
    if ($resolvedWork.StartsWith($safePrefix, [StringComparison]::OrdinalIgnoreCase) -and
        (Test-Path -LiteralPath $resolvedWork)) {
        [IO.Directory]::Delete($resolvedWork, $true)
    }
}
}
