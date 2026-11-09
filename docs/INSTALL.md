# Installing c9s

c9s is one self-contained program (no runtime, no dependencies). Every
release on [GitHub](https://github.com/mrWiecek/c9s/releases) has:

| File | For |
|---|---|
| `c9s-v<version>-x86_64-pc-windows-msvc.zip` | Windows (64-bit) |
| `c9s-v<version>-x86_64-unknown-linux-musl.tar.gz` | Linux and WSL: a static binary for any distribution |
| `c9s-v<version>-x86_64-unknown-linux-gnu.tar.gz` | Linux with a recent glibc (built on Ubuntu 24.04); prefer the static build |
| `c9s_<version>_amd64.deb`, `c9s-<version>-1.x86_64.rpm` | Debian/Ubuntu, RHEL/Fedora/SUSE packages (the static binary) |
| `c9s-v<version>-aarch64-apple-darwin.tar.gz`, `…-x86_64-apple-darwin.tar.gz` | macOS (Apple silicon, Intel) |
| `install.ps1`, `install.sh` | The install scripts below |
| `c9s.json` | Scoop manifest |
| `winget-manifests.zip` | winget manifests (see below) |
| `SHA256SUMS` | Checksums of all of the above archives and packages |

Each archive and package also has signed build provenance: GitHub can
prove it was built by this repository's release workflow, from the tagged
commit.

## Windows

### Install script (recommended)

```powershell
irm https://github.com/mrWiecek/c9s/releases/latest/download/install.ps1 | iex
```

It downloads the latest Windows build, checks it against `SHA256SUMS`,
copies `c9s.exe` to `%LOCALAPPDATA%\Programs\c9s` and adds that folder to
your user PATH. No administrator rights. Open a new terminal, then run
`c9s`. Run it again to update.

- A specific version: `$env:C9S_VERSION='1.9.0'; irm … | iex`
- Another folder: `$env:C9S_INSTALL_DIR='C:\Tools\c9s'; irm … | iex`

Works in PowerShell 7 and in Windows PowerShell 5.1.

### Scoop

```powershell
scoop install https://github.com/mrWiecek/c9s/releases/latest/download/c9s.json
```

### winget

Not in the winget catalog yet; each release carries ready manifests
(`winget-manifests.zip`, package `mrWiecek.c9s`). Until it is accepted,
a manifest can be installed locally:

```powershell
winget settings --enable LocalManifestFiles   # once, as administrator
winget install --manifest .\winget             # the unzipped folder
```

### By hand

Download the `.zip`, check it (below), extract `c9s.exe` anywhere on your
PATH.

## Linux and WSL

### Install script (recommended)

```bash
curl -fsSL https://github.com/mrWiecek/c9s/releases/latest/download/install.sh | sh
```

It picks the static build, checks it against `SHA256SUMS` and installs it
to `~/.local/bin` (no sudo). Run it again to update. Settings:
`C9S_VERSION=1.9.0`, `C9S_INSTALL_DIR=/opt/bin`. It works on macOS too.

### Packages

```bash
sudo apt install ./c9s_1.9.0_amd64.deb      # Debian, Ubuntu, WSL
sudo dnf install ./c9s-1.9.0-1.x86_64.rpm   # RHEL, Fedora (zypper on SUSE)
```

Both install `/usr/bin/c9s`, the static binary; nothing else is needed.

## From source

With [Rust](https://rustup.rs) 1.88 or newer (no other tools: TLS is
rustls, SQLite is built in):

```bash
cargo install --git https://github.com/mrWiecek/c9s --locked c9s
cargo install --git https://github.com/mrWiecek/c9s --tag v1.9.0 --locked c9s
```

## Verifying a download

```bash
sha256sum --ignore-missing -c SHA256SUMS                  # Linux, WSL
gh attestation verify c9s-v1.9.0-x86_64-pc-windows-msvc.zip --repo mrWiecek/c9s
```

```powershell
(Get-FileHash .\c9s-v1.9.0-x86_64-pc-windows-msvc.zip).Hash   # compare with SHA256SUMS
```

The install scripts check `SHA256SUMS` themselves.

## When security software blocks it

The binaries are not code-signed yet. What that means, and what helps:

- **"Windows protected your PC" (SmartScreen)** appears for downloads
  from a browser, which marks them as coming from the internet. The
  install script and Scoop download with PowerShell, which doesn't, so
  SmartScreen doesn't stop them. For a browser download: right-click the
  zip → Properties → Unblock, before extracting.
- **Antivirus (Defender) quarantines c9s.exe**: a false positive on a new,
  unsigned program. Report it to Microsoft
  ([submit a file](https://www.microsoft.com/wdsi/filesubmission)) and ask
  your IT team to allow it; the checksum and the build provenance above
  show the file is the released one.
- **Company policy (AppLocker, WDAC, Intune) allows only approved or
  signed programs**: per-user folders are often not allowed to run
  programs. Install where your company allows tools (set
  `C9S_INSTALL_DIR`), or ask IT to approve c9s by its hash or to deploy it.
  Code signing would remove this; it is planned.
- **GitHub downloads blocked by a proxy**: build from source with `cargo`
  (it downloads from crates.io), or have someone copy the release files
  in; check them with `SHA256SUMS`.
- **Linux: "version GLIBC_2.xx not found"**: use the static `musl` build,
  the `.deb`/`.rpm` or the install script, which all use it.
