# Build rshell release binary on Windows.
#
# Usage:  .\scripts\build.ps1 [-Target <triple>] [-SkipCopy]
#
#   -Target   : rustc target triple (e.g. x86_64-pc-windows-msvc).
#               If omitted, uses the host triple.
#   -SkipCopy : skip the copy-out step (mostly for CI debug).
#
# Output: target\release\<os>-<arch>\rshell-<version>.exe
#         plus a "latest" copy at the same path.

[CmdletBinding()]
param(
    [string]$Target = "",
    [switch]$SkipCopy
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

# ---------- 1. workspace version via shared PowerShell helper ----------
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot  = (Resolve-Path (Join-Path $ScriptDir "..")).Path
Set-Location $RepoRoot

$version = (& powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $ScriptDir "read-version.ps1")) | ForEach-Object { $_.Trim() } | Select-Object -First 1
if (-not $version) {
    Write-Error "cannot read version via read-version.ps1"
    exit 1
}

# ---------- 2. host triple detection ----------
if (-not $Target) {
    $rustcOut = (& rustc -vV 2>$null | Out-String).Trim()
    $Target = ($rustcOut -split "`n" | Where-Object { $_ -match '^host:' } | ForEach-Object { ($_ -split '\s+')[1] })
}
if (-not $Target) {
    Write-Error "cannot determine host triple; pass -Target"
    exit 1
}

# ---------- 3. output paths ----------
$osLabel = "windows"
$archLabel = if ($env:PROCESSOR_ARCH -eq "ARM64") { "arm64" } else { "x86_64" }

$outDir = Join-Path $RepoRoot "target\release\$osLabel-$archLabel"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

$outBin = Join-Path $outDir ("rshell-" + $version + ".exe")
$latestBin = Join-Path $outDir "rshell.exe"

# ---------- 4. build ----------
Write-Host "Building rshell $version for $Target -> $outBin"
& cargo build --release --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# ---------- 5. copy out ----------
if (-not $SkipCopy) {
    $srcBin = Get-ChildItem -Path (Join-Path $RepoRoot "target\release") -Filter "rshell.exe" -Recurse -ErrorAction SilentlyContinue |
              Select-Object -First 1 -ExpandProperty FullName
    if (-not $srcBin) {
        Write-Error "built binary not found under target\release\"
        exit 1
    }
    Copy-Item -Path $srcBin -Destination $outBin -Force
    Copy-Item -Path $outBin -Destination $latestBin -Force

    Write-Host ""
    Write-Host "Done."
    Write-Host "  Binary : $outBin"
    Write-Host "  Latest : $latestBin"
    Get-Item $outBin | Select-Object FullName, Length | Format-Table -AutoSize
}
