@echo off
REM Build rshell release binary on Windows (cmd / batch version).
REM
REM Usage:  scripts\build.cmd [target]
REM   target (optional): rustc triple, e.g. x86_64-pc-windows-msvc.
REM                     Default = host triple detected via `rustc -vV`.
REM
REM Output: target\release\windows-<arch>\rshell-<version>.exe
REM         plus a "latest" copy rshell.exe in the same directory.
REM
REM Requires: cargo + rustc + git on PATH.

setlocal EnableDelayedExpansion

set "SCRIPT_DIR=%~dp0"
pushd "%SCRIPT_DIR%\.."

REM ---------- 1. workspace version via PowerShell helper ----------
for /f "delims=" %%V in ('powershell -NoProfile -ExecutionPolicy Bypass -File "%SCRIPT_DIR%read-version.ps1"') do set "VERSION=%%V"
if "!VERSION!"=="" (
    echo ERROR: cannot read version from Cargo.toml 1>&2
    popd
    exit /b 1
)

REM ---------- 2. host triple detection ----------
set "TARGET=%~1"
if "!TARGET!"=="" (
    for /f "tokens=2 delims=:" %%T in ('rustc -vV 2^>nul ^| findstr /B /C:"host:"') do set "TARGET=%%T"
    for /f "tokens=* delims= " %%U in ("!TARGET!") do set "TARGET=%%U"
)
if "!TARGET!"=="" (
    echo ERROR: cannot determine host triple; pass as arg 1>&2
    popd
    exit /b 1
)

REM ---------- 3. output paths ----------
if /I "!PROCESSOR_ARCH!"=="ARM64" (set "ARCH=arm64") else (set "ARCH=x86_64")
set "OUT_DIR=target\release\windows-!ARCH!"
if not exist "!OUT_DIR!" mkdir "!OUT_DIR!"

set "OUT_BIN=!OUT_DIR!\rshell-!VERSION!.exe"
set "LATEST_BIN=!OUT_DIR!\rshell.exe"

REM ---------- 4. build ----------
echo Building rshell !VERSION! for !TARGET! -^> !OUT_BIN!
call cargo build --release --locked
if !ERRORLEVEL! neq 0 (
    popd
    exit /b !ERRORLEVEL!
)

REM ---------- 5. copy out ----------
set "SRC_BIN="
for /r "target\release" %%F in (rshell.exe) do (
    if "!SRC_BIN!"=="" set "SRC_BIN=%%F"
)
if "!SRC_BIN!"=="" (
    echo ERROR: built binary not found under target\release\ 1>&2
    popd
    exit /b 1
)
copy /Y "!SRC_BIN!" "!OUT_BIN!" >nul
copy /Y "!OUT_BIN!" "!LATEST_BIN!" >nul

echo.
echo Done.
echo   Binary : !OUT_BIN!
echo   Latest : !LATEST_BIN!
dir "!OUT_BIN!"

popd
endlocal
