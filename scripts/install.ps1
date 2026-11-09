# Install c9s on Windows: download a release from GitHub, verify its SHA-256
# checksum, install it for the current user (no admin) and add it to PATH.
#
#   irm https://github.com/mrWiecek/c9s/releases/latest/download/install.ps1 | iex
#
# Settings (environment variables):
#   C9S_VERSION      a version to install, e.g. 1.9.0 (default: the latest)
#   C9S_INSTALL_DIR  where to put c9s.exe (default: %LOCALAPPDATA%\Programs\c9s)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'   # Invoke-WebRequest is slow with it
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$repo = 'mrWiecek/c9s'
$dir = if ($env:C9S_INSTALL_DIR) { $env:C9S_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\c9s' }

function Say($text) { Write-Host "c9s: $text" }

if (-not [Environment]::Is64BitOperatingSystem) {
    throw 'c9s: only 64-bit Windows is supported'
}

$version = $env:C9S_VERSION
if (-not $version) {
    # The latest release's tag (the GitHub API, no token needed).
    $release = Invoke-RestMethod -UseBasicParsing "https://api.github.com/repos/$repo/releases/latest"
    $version = $release.tag_name
}
$version = $version.TrimStart('v')
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw "c9s: not a version: '$version'" }

$base = "https://github.com/$repo/releases/download/v$version"
$name = "c9s-v$version-x86_64-pc-windows-msvc"
$archive = "$name.zip"
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("c9s-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    $sums = (Invoke-WebRequest -UseBasicParsing "$base/SHA256SUMS").Content
    if ($sums -is [byte[]]) { $sums = [Text.Encoding]::UTF8.GetString($sums) }
    $line = ($sums -split "`n") | Where-Object { $_.Trim().EndsWith(" $archive") } | Select-Object -First 1
    if (-not $line) { throw "c9s: release v$version has no $archive" }
    $expected = ($line.Trim() -split '\s+')[0].ToLowerInvariant()

    Say "downloading $archive"
    $zip = Join-Path $tmp $archive
    Invoke-WebRequest -UseBasicParsing "$base/$archive" -OutFile $zip
    $actual = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { throw "c9s: checksum mismatch for $archive, refusing to install" }
    Say 'checksum verified'

    Expand-Archive -Path $zip -DestinationPath $tmp -Force
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $exe = Join-Path $dir 'c9s.exe'
    Copy-Item -Force (Join-Path $tmp "$name\c9s.exe") $exe
    Say "installed $(& $exe -V) to $exe"
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

# On the user's PATH (takes effect in new terminals).
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$parts = @($userPath -split ';' | Where-Object { $_ })
if ($parts -notcontains $dir) {
    [Environment]::SetEnvironmentVariable('Path', (($parts + $dir) -join ';'), 'User')
    Say "added $dir to your PATH: open a new terminal, then run c9s"
}
if (($env:Path -split ';') -notcontains $dir) { $env:Path = "$env:Path;$dir" }
