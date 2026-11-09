#!/usr/bin/env bash
# Package-manager manifests for a release, from its SHA256SUMS:
#   <out>/c9s.json             Scoop: scoop install <release url>/c9s.json
#   <out>/winget/*.yaml        winget: for a pull request to microsoft/winget-pkgs
#
#   scripts/manifests.sh <version> <dir with SHA256SUMS> <out-dir>
set -euo pipefail

version="$1"
sums="$2/SHA256SUMS"
out="$3"
repo="mrWiecek/c9s"
name="c9s-v$version-x86_64-pc-windows-msvc"
zip="$name.zip"
url="https://github.com/$repo/releases/download/v$version/$zip"
hash=$(awk -v f="$zip" '$2 == f { print $1 }' "$sums")
[ -n "$hash" ] || { echo "no $zip in $sums" >&2; exit 1; }
summary="Keyboard-first terminal UI for Azure, Google Cloud and AWS"
mkdir -p "$out/winget"

cat > "$out/c9s.json" <<JSON
{
    "version": "$version",
    "description": "$summary",
    "homepage": "https://github.com/$repo",
    "license": "Apache-2.0",
    "architecture": {
        "64bit": {
            "url": "$url",
            "hash": "$hash",
            "extract_dir": "$name"
        }
    },
    "bin": "c9s.exe",
    "checkver": {
        "github": "https://github.com/$repo"
    },
    "autoupdate": {
        "architecture": {
            "64bit": {
                "url": "https://github.com/$repo/releases/download/v\$version/c9s-v\$version-x86_64-pc-windows-msvc.zip",
                "extract_dir": "c9s-v\$version-x86_64-pc-windows-msvc",
                "hash": {
                    "url": "https://github.com/$repo/releases/download/v\$version/SHA256SUMS"
                }
            }
        }
    }
}
JSON

id="mrWiecek.c9s"
common="PackageIdentifier: $id
PackageVersion: $version"
cat > "$out/winget/$id.yaml" <<YAML
$common
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.6.0
YAML
cat > "$out/winget/$id.installer.yaml" <<YAML
$common
InstallerType: zip
NestedInstallerType: portable
NestedInstallerFiles:
  - RelativeFilePath: $name\\c9s.exe
    PortableCommandAlias: c9s
ReleaseDate: $(date -u +%Y-%m-%d)
Installers:
  - Architecture: x64
    InstallerUrl: $url
    InstallerSha256: $(echo "$hash" | tr '[:lower:]' '[:upper:]')
ManifestType: installer
ManifestVersion: 1.6.0
YAML
cat > "$out/winget/$id.locale.en-US.yaml" <<YAML
$common
PackageLocale: en-US
Publisher: mrWiecek
PublisherUrl: https://github.com/mrWiecek
PackageName: c9s
PackageUrl: https://github.com/$repo
License: Apache-2.0
LicenseUrl: https://github.com/$repo/blob/main/LICENSE
ShortDescription: $summary
Description: c9s ("canines for clouds") browses Azure (management groups, subscriptions, resources, Policy, RBAC, Entra ID), Google Cloud and AWS from the terminal. Read-only; uses the az, gcloud and aws CLIs' logins.
Moniker: c9s
Tags:
  - azure
  - aws
  - gcp
  - cloud
  - tui
  - cli
ReleaseNotesUrl: https://github.com/$repo/releases/tag/v$version
ManifestType: defaultLocale
ManifestVersion: 1.6.0
YAML
ls -1 "$out" "$out/winget"
