#!/usr/bin/env bash
# Build .deb and .rpm packages around a prebuilt (static) c9s binary.
#
#   scripts/package-linux.sh <version> <path/to/c9s> <out-dir>
#
# Needs dpkg-deb and rpmbuild. The binary is static (musl), so the packages
# depend on nothing and install on any Debian/Ubuntu or RHEL/Fedora/SUSE.
set -euo pipefail

version="$1"
binary="$2"
out="$3"
root="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

summary="Keyboard-first terminal UI for Azure, Google Cloud and AWS"

# --- .deb -------------------------------------------------------------
deb="$work/deb"
install -Dm755 "$binary" "$deb/usr/bin/c9s"
install -Dm644 "$root/README.md" "$deb/usr/share/doc/c9s/README.md"
install -Dm644 "$root/LICENSE" "$deb/usr/share/doc/c9s/copyright"
mkdir -p "$deb/DEBIAN"
cat > "$deb/DEBIAN/control" <<CONTROL
Package: c9s
Version: $version
Section: utils
Priority: optional
Architecture: amd64
Maintainer: c9s contributors <noreply@github.com>
Homepage: https://github.com/mrWiecek/c9s
Description: $summary
 c9s ("canines for clouds") browses Azure (management groups,
 subscriptions, resources, Policy, RBAC, Entra ID), Google Cloud and AWS
 from the terminal. Read-only; uses the az, gcloud and aws CLIs' logins.
CONTROL
dpkg-deb --root-owner-group --build "$deb" "$out/c9s_${version}_amd64.deb" >/dev/null

# --- .rpm -------------------------------------------------------------
top="$work/rpm"
mkdir -p "$top"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}
cp "$binary" "$top/SOURCES/c9s"
cp "$root/README.md" "$root/LICENSE" "$top/SOURCES/"
cat > "$top/SPECS/c9s.spec" <<SPEC
%define debug_package %{nil}
%define __strip /bin/true
%define _build_id_links none
Name:           c9s
Version:        $version
Release:        1
Summary:        $summary
License:        Apache-2.0
URL:            https://github.com/mrWiecek/c9s
BuildArch:      x86_64

%description
c9s ("canines for clouds") browses Azure, Google Cloud and AWS from the
terminal. Read-only; uses the az, gcloud and aws CLIs' logins.

%install
install -Dm755 %{_sourcedir}/c9s %{buildroot}/usr/bin/c9s
install -Dm644 %{_sourcedir}/README.md %{buildroot}/usr/share/doc/c9s/README.md
install -Dm644 %{_sourcedir}/LICENSE %{buildroot}/usr/share/licenses/c9s/LICENSE

%files
/usr/bin/c9s
/usr/share/doc/c9s/README.md
/usr/share/licenses/c9s/LICENSE
SPEC
rpmbuild --quiet --define "_topdir $top" -bb "$top/SPECS/c9s.spec"
cp "$top"/RPMS/x86_64/c9s-"$version"-1.x86_64.rpm "$out/"

ls -1 "$out"
