#!/bin/sh
# JustReady bootstrap for macOS and Linux.
# Run remotely:
#   curl -fsSL https://raw.githubusercontent.com/JustGains/JustTools/main/ready.sh | sh

set -eu

enabled() {
    case "${1:-}" in
        1|true|TRUE|yes|YES|on|ON) return 0 ;;
        *) return 1 ;;
    esac
}

fail() {
    printf 'JustReady bootstrap: %s\n' "$*" >&2
    exit 1
}

download() {
    source_url=$1
    destination=$2
    printf 'Downloading %s\n' "$source_url"
    if command -v curl >/dev/null 2>&1; then
        curl -fL --retry 3 --connect-timeout 15 -o "$destination" "$source_url"
    elif command -v wget >/dev/null 2>&1; then
        wget -q --tries=3 -O "$destination" "$source_url"
    else
        fail "curl or wget is required to download the release"
    fi
}

case "$(uname -s)" in
    Darwin) platform=macos ;;
    Linux) platform=linux ;;
    *) fail "ready.sh supports macOS and Linux; use ready.ps1 on Windows" ;;
esac

case "$(uname -m)" in
    x86_64|amd64) architecture=x64 ;;
    arm64|aarch64) architecture=arm64 ;;
    *) fail "JustTools does not publish a $platform build for architecture '$(uname -m)'" ;;
esac

repository=${JUSTTOOLS_REPO:-JustGains/JustTools}
version=${JUSTTOOLS_VERSION:-latest}
asset="justtools-$platform-$architecture.tar.gz"
temp_root=${TMPDIR:-/tmp}
case "$temp_root" in
    /) ;;
    *) temp_root=${temp_root%/} ;;
esac
work_directory=$(mktemp -d "$temp_root/justtools-ready.XXXXXX") || fail "could not create a temporary directory"

cleanup() {
    case "$work_directory" in
        "$temp_root"/justtools-ready.*)
            rm -rf -- "$work_directory"
            ;;
    esac
}
trap cleanup EXIT HUP INT TERM

archive="$work_directory/$asset"
checksum="$archive.sha256"
extract_directory="$work_directory/extract"
mkdir -p "$extract_directory"

if [ -n "${JUSTTOOLS_ARCHIVE:-}" ]; then
    [ -f "$JUSTTOOLS_ARCHIVE" ] || fail "archive does not exist: $JUSTTOOLS_ARCHIVE"
    cp "$JUSTTOOLS_ARCHIVE" "$archive"
    if ! enabled "${JUSTTOOLS_SKIP_VERIFY:-}"; then
        [ -f "$JUSTTOOLS_ARCHIVE.sha256" ] || fail "missing checksum sidecar: $JUSTTOOLS_ARCHIVE.sha256"
        cp "$JUSTTOOLS_ARCHIVE.sha256" "$checksum"
    fi
else
    case "$version" in
        ''|latest) release_base="https://github.com/$repository/releases/latest/download" ;;
        v*) release_base="https://github.com/$repository/releases/download/$version" ;;
        *) release_base="https://github.com/$repository/releases/download/v$version" ;;
    esac
    download "$release_base/$asset" "$archive"
    if ! enabled "${JUSTTOOLS_SKIP_VERIFY:-}"; then
        download "$release_base/$asset.sha256" "$checksum"
    fi
fi

if ! enabled "${JUSTTOOLS_SKIP_VERIFY:-}"; then
    expected=$(awk 'NR == 1 { print tolower($1) }' "$checksum")
    case "$expected" in
        *[!0-9a-f]*|'') fail "the release checksum is malformed" ;;
    esac
    [ "${#expected}" -eq 64 ] || fail "the release checksum is malformed"
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$archive" | awk '{ print tolower($1) }')
    elif command -v shasum >/dev/null 2>&1; then
        actual=$(shasum -a 256 "$archive" | awk '{ print tolower($1) }')
    else
        fail "sha256sum or shasum is required to verify the release"
    fi
    [ "$actual" = "$expected" ] || fail "checksum verification failed for $asset"
    printf 'Verified %s\n' "$asset"
fi

tar -xzf "$archive" -C "$extract_directory"
staged="$extract_directory/just"
if [ ! -f "$staged" ]; then
    staged=$(find "$extract_directory" -type f -name just -print -quit)
fi
[ -n "$staged" ] && [ -f "$staged" ] || fail "$asset does not contain just"
chmod +x "$staged"

if ! "$staged" ready --help >/dev/null 2>&1; then
    fail "the selected JustTools release predates JustReady; no existing installation was changed"
fi

bin_directory=${JUSTTOOLS_BIN:-"$HOME/.local/bin"}
set -- install --bin-dir "$bin_directory" --yes
if enabled "${JUSTTOOLS_NO_PATH:-}"; then
    set -- "$@" --no-path
fi
"$staged" "$@"

ready="$bin_directory/justready"
[ -x "$ready" ] || fail "installation completed without the expected JustReady alias: $ready"

if enabled "${JUSTREADY_NO_RUN:-}"; then
    printf 'JustReady is installed at %s\n' "$ready"
elif [ -c /dev/tty ] && (: </dev/tty) 2>/dev/null; then
    "$ready" </dev/tty >/dev/tty 2>/dev/tty
else
    printf 'JustReady is installed at %s\n' "$ready"
    printf '%s\n' "Run 'justready' from a new terminal to open it."
fi
