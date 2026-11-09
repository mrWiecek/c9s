#!/bin/sh
# Install c9s: download a release from GitHub, verify its SHA-256 checksum
# and put the binary in ~/.local/bin (no sudo).
#
#   curl -fsSL https://github.com/mrWiecek/c9s/releases/latest/download/install.sh | sh
#
# Settings (environment variables):
#   C9S_VERSION      a version to install, e.g. 1.9.0 (default: the latest)
#   C9S_INSTALL_DIR  where to put c9s (default: ~/.local/bin)
set -eu

repo="mrWiecek/c9s"
dir="${C9S_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf 'c9s: %s\n' "$*" >&2; }
fail() { say "$*"; exit 1; }

download() { # url file
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --proto '=https' --tlsv1.2 -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -q --https-only -O "$2" "$1"
    else
        fail "needs curl or wget"
    fi
}

latest_version() {
    # The latest release's tag, from the redirect of /releases/latest.
    if command -v curl >/dev/null 2>&1; then
        url=$(curl -fsSLI --proto '=https' -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest")
    else
        url=$(wget -q --https-only -S --spider "https://github.com/$repo/releases/latest" 2>&1 | sed -n 's/^ *Location: //p' | tail -1)
    fi
    echo "${url##*/v}" | tr -d '\r'
}

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        fail "needs sha256sum or shasum to verify the download"
    fi
}

os=$(uname -s)
arch=$(uname -m)
case "$os/$arch" in
    Linux/x86_64 | Linux/amd64) targets="x86_64-unknown-linux-musl x86_64-unknown-linux-gnu" ;;
    Darwin/arm64 | Darwin/aarch64) targets="aarch64-apple-darwin" ;;
    Darwin/x86_64) targets="x86_64-apple-darwin" ;;
    *) fail "no prebuilt c9s for $os/$arch; build it: cargo install --git https://github.com/$repo c9s" ;;
esac

version="${C9S_VERSION:-}"
[ -n "$version" ] || version=$(latest_version)
version="${version#v}"
case "$version" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) fail "could not tell the latest version (got '$version'); set C9S_VERSION" ;;
esac
base="https://github.com/$repo/releases/download/v$version"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
download "$base/SHA256SUMS" "$tmp/SHA256SUMS" || fail "no release v$version"

# The first target this release has (the static Linux build, else glibc).
archive=""
for target in $targets; do
    name="c9s-v$version-$target.tar.gz"
    if grep -q " $name\$" "$tmp/SHA256SUMS"; then
        archive="$name"
        break
    fi
done
[ -n "$archive" ] || fail "release v$version has no build for $os/$arch"

say "downloading $archive"
download "$base/$archive" "$tmp/$archive"
expected=$(grep " $archive\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
actual=$(sha256 "$tmp/$archive")
[ "$expected" = "$actual" ] || fail "checksum mismatch for $archive: refusing to install"
say "checksum verified"

tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$dir"
install -m 755 "$tmp/${archive%.tar.gz}/c9s" "$dir/c9s"
say "installed $("$dir/c9s" -V) to $dir/c9s"

case ":$PATH:" in
    *":$dir:"*) ;;
    *) say "$dir is not on your PATH; add it, e.g.: echo 'export PATH=\"$dir:\$PATH\"' >> ~/.profile" ;;
esac
