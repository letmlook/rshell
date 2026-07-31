# Read rshell workspace version from Cargo.toml.
# Used by build.cmd and build.ps1 to avoid fragile cmd TOML parsing.
#
# Outputs the version on stdout. Exits 0 on success, 1 if not found.

$ErrorActionPreference = "Stop"
$content = Get-Content -Path (Join-Path $PSScriptRoot "..\src-tauri\Cargo.toml") -Raw -ErrorAction Stop
$match = [regex]::Match($content, '(?m)^version\s*=\s*"([^"]+)"')
if (-not $match.Success) {
    Write-Error "cannot read version from src-tauri/Cargo.toml"
    exit 1
}
$version = $match.Groups[1].Value

# Prefer git describe when available
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if (Test-Path (Join-Path $repoRoot ".git")) {
    $gitDesc = (& git describe --tags --always --dirty="-dev" 2>$null) | ForEach-Object { $_.Trim() }
    if ($gitDesc -and ($gitDesc -match '^\d+\.\d+\.\d+' -or $gitDesc -match '^v\d+\.\d+\.\d+')) {
        $version = ($gitDesc -replace '^v', '')
    }
}
Write-Output $version
