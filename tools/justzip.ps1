[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string] $Path = (Get-Location).Path
)

$ErrorActionPreference = 'Stop'

function Fail([string] $Message) {
    Write-Error "justzip: $Message" -ErrorAction Continue
    exit 1
}

try {
    $source = (Resolve-Path -LiteralPath $Path).Path
} catch {
    Fail "folder not found: $Path"
}

if (-not (Test-Path -LiteralPath $source -PathType Container)) {
    Fail "not a folder: $Path"
}

if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Fail 'Git is not installed or is not on PATH.'
}

# Asking Git for the file list is the most accurate way to apply all relevant
# ignore sources: nested .gitignore files, .git/info/exclude, and global ignores.
$rawList = & git -C $source ls-files --cached --others --exclude-standard -z 2>&1
if ($LASTEXITCODE -ne 0) {
    $detail = ($rawList | Out-String).Trim()
    if (-not $detail) { $detail = 'the folder is not inside a Git repository' }
    Fail $detail
}

$directoryName = Split-Path -Leaf $source.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
if (-not $directoryName) { Fail 'cannot derive an archive name from this folder.' }

$archiveName = "$directoryName.zip"
$destination = Join-Path (Get-Location).Path $archiveName
$temporary = "$destination.$PID.tmp"

# The output normally lives outside the source when a path is passed. If the
# caller's working directory is inside the source, exclude its exact relative
# path so an existing output archive can never include itself.
$relativeOutput = [IO.Path]::GetRelativePath($source, $destination).Replace('\', '/')
$outputIsInsideSource = -not [IO.Path]::IsPathRooted($relativeOutput) -and
    $relativeOutput -ne '..' -and
    -not $relativeOutput.StartsWith('../', [StringComparison]::Ordinal)

$files = @(
    ($rawList -join '') -split "`0" |
        Where-Object {
            $_ -ne '' -and (-not $outputIsInsideSource -or $_.Replace('\', '/') -ne $relativeOutput)
        } |
        Sort-Object -Unique
)

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

try {
    $stream = [IO.File]::Open($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        $archive = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create, $false)
        try {
            foreach ($relative in $files) {
                $fullPath = Join-Path $source ($relative -replace '/', [IO.Path]::DirectorySeparatorChar)
                if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf)) {
                    # Gitlinks (submodules) appear as directory entries in ls-files;
                    # ZIP has no useful representation for the gitlink itself.
                    if (Test-Path -LiteralPath $fullPath -PathType Container) { continue }
                    throw "file disappeared while archiving: $relative"
                }

                $entryName = $relative.Replace('\', '/')
                [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                    $archive,
                    $fullPath,
                    $entryName,
                    [IO.Compression.CompressionLevel]::SmallestSize
                ) | Out-Null
            }
        } finally {
            if ($null -ne $archive) { $archive.Dispose() }
        }
    } finally {
        if ($null -ne $stream) { $stream.Dispose() }
    }

    Move-Item -LiteralPath $temporary -Destination $destination -Force
} catch {
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
    Fail $_.Exception.Message
}

$size = (Get-Item -LiteralPath $destination).Length
$displaySize = if ($size -ge 1GB) {
    '{0:N2} GiB' -f ($size / 1GB)
} elseif ($size -ge 1MB) {
    '{0:N2} MiB' -f ($size / 1MB)
} elseif ($size -ge 1KB) {
    '{0:N2} KiB' -f ($size / 1KB)
} else {
    "$size bytes"
}

Write-Host "justzip: archived $($files.Count) files -> $destination ($displaySize)"
