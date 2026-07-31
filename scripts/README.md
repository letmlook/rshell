# Cross-platform build scripts

This directory contains build scripts for producing a release binary of
`rshell` on the three target platforms.

| Script           | Platform              | Invocation         |
| ---------------- | --------------------- | ------------------ |
| `build.sh`       | Linux, macOS, WSL     | `./scripts/build.sh [target]` |
| `build.ps1`      | Windows (PowerShell)  | `.\scripts\build.ps1 [-Target <triple>]` |
| `build.cmd`      | Windows (cmd)         | `scripts\build.cmd [target]` |
| `read-version.ps1` | helper (PowerShell) | used by both `.ps1` and `.cmd` |

## Common behavior

- Reads `version` from `[workspace.package] version = ...` in
  `Cargo.toml`. If a git tag is present (`v0.1.0` or `0.1.0`), the
  most recent reachable tag + commits-since is preferred (matches
  `git describe --tags --always`).
- Calls `cargo build --release --locked` for the host triple (or the
  triple you pass as the first argument).
- Copies the resulting binary to
  `target/release/<os>-<arch>/rshell-<version>[.exe]`.
- Creates a `latest` symlink/copy at the same directory
  (`rshell[-<version>][.exe]` is the versioned artifact, `rshell` is
  always the freshest).

## Cross-compilation

The scripts accept an explicit `target` argument for cross compilation,
e.g.

```bash
# from a Linux host
./scripts/build.sh aarch64-unknown-linux-gnu
./scripts/build.sh x86_64-apple-darwin

# from Windows (PowerShell)
.\scripts\build.ps1 -Target aarch64-pc-windows-msvc
```

For non-host targets you must have the target installed:

```bash
rustup target add aarch64-unknown-linux-gnu
```

## Notes on macOS

`build.sh` recognizes `apple-darwin` host triples and produces an
`rshell` Mach-O binary under `target/release/macos-<arch>/`. The binary
is unsigned — `codesign` / `notarytool` should be added if you
intend to distribute the build.

## Versioning

`version` is sourced from `[workspace.package]` in `Cargo.toml`. The
release artifact is named `rshell-<version>[.exe]`. A `latest`
copy/symlink is provided for convenience.
