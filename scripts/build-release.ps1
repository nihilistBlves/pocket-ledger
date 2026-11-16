# Builds the per-user NSIS installer (unsigned) and prints its path and SHA-256.
# Run from anywhere:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build-release.ps1
# Set CARGO_TARGET_DIR beforehand to use a short path (the vendored OpenSSL build needs it
# when the checkout path is deep). Otherwise <repo>\tmp\target-release is used.
$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
. 'C:\Dev\dev-env.ps1'
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $repo 'tmp\target-release' }

Set-Location $repo
$timer = [Diagnostics.Stopwatch]::StartNew()

pnpm install --frozen-lockfile
if ($LASTEXITCODE -ne 0) { throw "pnpm install failed ($LASTEXITCODE)" }
pnpm tauri build
if ($LASTEXITCODE -ne 0) { throw "pnpm tauri build failed ($LASTEXITCODE)" }

$bundle = Join-Path $env:CARGO_TARGET_DIR 'release\bundle\nsis'
$installer = Get-ChildItem -Path $bundle -Filter '*-setup.exe' -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $installer) { throw "No *-setup.exe found in $bundle" }

$hash = (Get-FileHash -Algorithm SHA256 -Path $installer.FullName).Hash.ToLowerInvariant()
Write-Output ("Installer: {0}" -f $installer.FullName)
Write-Output ("Size:      {0:n0} bytes" -f $installer.Length)
Write-Output ("SHA-256:   {0}" -f $hash)
Write-Output ("Build time: {0:n0} s" -f $timer.Elapsed.TotalSeconds)
