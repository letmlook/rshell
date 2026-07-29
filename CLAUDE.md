# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repo is

RShell — a cross-platform (Windows / macOS / Linux) Xshell-like remote terminal & SFTP client, written in Rust with a GPUI frontend. Workspace version `0.1.0`, Rust `1.90` (see `rust-toolchain.toml`). All Chinese-language design docs live under `docs/` (read `docs/02-project-plan.md`, `docs/03-detailed-design.md`, and `docs/05-development-standards.md` before making non-trivial changes — they encode the architecture and conventions).

## Common commands

```bash
# Build everything
cargo build

# Run the desktop app (the only binary in the workspace)
cargo run --package rshell-ui         # or simply: cargo run

# Run all tests across the workspace
cargo test

# Run tests in a single crate
cargo test -p rshell-core
cargo test -p rshell-protocol
cargo test -p rshell-api

# Run a single test by name
cargo test -p rshell-core -- event_bus::tests::test_publish_subscribe

# Lint / format
cargo clippy --workspace --all-targets
cargo fmt --all

# Apply the cargo alias declared in .cargo/config.toml
cargo xtask …           # NOTE: the `xtask` alias is configured but no xtask
                         # crate exists in the workspace yet — the alias will
                         # currently fail with "package not found".
```

Release profile enables LTO + `codegen-units = 1` + `strip` (`Cargo.toml`). Dev profile keeps `debug = true`. Cross-platform PTY lives in `crates/rshell-infra/src/pty/{unix,windows}.rs`.

## Workspace layout (six crates)

```
crates/
├── rshell-api/         # ZERO-RUNTIME-DEPENDENCY boundary layer.
│                       # AppCommand, AppEvent, shared types. Only deps: serde, uuid.
│                       # No tokio, no gpui, no alacritty_terminal. Keep it pure.
│
├── rshell-infra/       # crypto (AES via ring), persistent storage (TOML),
│                       # cross-platform PTY abstraction.
│
├── rshell-protocol/    # SSH (russh 0.48 + russh-sftp), Telnet, Serial, RDP.
│                       # Exposes the unified `Connection` async trait
│                       # (lib.rs: connect / disconnect / send / recv / resize).
│
├── rshell-core/        # Backend business logic — does NOT import gpui.
│                       # terminal/  (alacritty_terminal-based VTE buffer)
│                       # session/   (SessionService + repository)
│                       # transfer/  (SFTP upload/download queue)
│                       # security/  (key_manager, master_password,
│                       #              host_key_manager, tunnel_manager)
│                       # script/    (rhai engine, quick commands, triggers,
│                       #              compose pane, sync-input)
│                       # theme/     (themes + terminal color schemes)
│                       # event_bus.rs + command_dispatcher.rs (see below)
│
├── rshell-plugin-sdk/  # Plugin loader, WASM sandbox (sandbox.rs is
│                       # currently a scaffold — wasmtime integration pending),
│                       # RShellPlugin trait, manifests, permissions.
│
└── rshell-ui/          # GPUI frontend. Single binary `rshell`.
                        # main.rs → bridge.rs → app.rs → views/ + view_models/.
```

The dependency direction is strictly downward: `ui → api`, `core → {api, protocol, infra, plugin-sdk}`, `protocol → {api, infra}`, `infra → api`, `plugin-sdk → api`. `api` has no internal deps.

## Big-picture architecture: strict front-end / back-end split

This is the single most important invariant. `docs/05-development-standards.md` §2 codifies it; violating it breaks the codebase.

**Rule 1.** `rshell-core`, `rshell-protocol`, `rshell-infra` MUST NOT `use gpui::*`.
**Rule 2.** `rshell-ui` views and view_models MUST NOT `use rshell_core::*`. The only files in `rshell-ui` allowed to touch core are `main.rs` and `bridge.rs`.
**Rule 3.** The sole channel between the two halves is `rshell-api` — enums `AppCommand` (UI→backend, intent) and `AppEvent` (backend→UI, snapshot).

### How a click becomes a render

1. GPUI `View` (e.g. `views/session_view.rs`) handles input → builds an `AppCommand` → calls `AppBridge::send_command` (`crates/rshell-ui/src/bridge.rs`).
2. `bridge.rs` is a `mpsc::UnboundedSender<AppCommand>` on the GPUI side. The receiver lives on a **dedicated background OS thread** (`thread::Builder::new().name("rshell-backend")`) running a single-thread tokio runtime + `LocalSet`. This is required because `rhai::Engine` inside `ScriptEngine` is `!Send`.
3. On that thread, `CommandDispatcher::dispatch` (`crates/rshell-core/src/command_dispatcher.rs`) matches on every `AppCommand` variant and routes it to the right service (`SessionService`, `TerminalService`, `TransferService`, `KeyManager`, etc.).
4. Services do their async work and call `EventBus::publish` with `AppEvent` snapshots.
5. `bridge.rs` subscribes to the `EventBus` from inside the main process (before spawning the backend thread) and pushes every event into a shared `Arc<Mutex<Vec<AppEvent>>>`. GPUI's `RshellApp::process_events` drains that queue every render and updates local state.

So even though the backend lives on another thread, GPUI only ever touches a `Mutex<Vec<AppEvent>>` + an `mpsc::Sender`. Keep it that way.

### Conventions for `AppCommand` and `AppEvent`

- `AppCommand` (`rshell-api/src/commands.rs`): verb+noun, intent, all named fields (no tuple variants), every field `Serialize + Deserialize + Clone`. Add new variants here and extend the `match` in `CommandDispatcher::dispatch`.
- `AppEvent` (`rshell-api/src/events.rs`): past tense, full snapshot data attached (UI never has to re-query). High-frequency events like `TerminalOutput` carry raw bytes; large state uses `*Snapshot` structs (`TerminalBufferSnapshot`).
- ViewModels (`rshell-ui/src/view_models/`): backend projection + local UI state only. Must not hold any `Arc<Service>` reference. Live-state fields are separate from local-only fields (scroll offset, search query, etc.).

## Other conventions worth knowing

- **Errors.** `thiserror` enums per module; `CoreError` aggregates at the crate root (`crates/rshell-core/src/error.rs`). No `unwrap`/`expect` in production code. `?` for conversion via `From`.
- **Logging.** `tracing` crate, structured fields (`info!(session_id = %id, host = %config.host, "…")`), `#[instrument]` on the entry points of `CommandDispatcher::dispatch`, `SessionService::connect`, etc. Init in `rshell-ui/src/main.rs` with env filter defaulting to `info,rshell=debug`.
- **Async.** `tokio` everywhere; `tokio::spawn` for I/O, `spawn_blocking` for CPU work, `tokio::select!` for timeouts, `tokio::sync::mpsc` for channels. `rhai` is single-threaded, hence the dedicated backend thread.
- **Persistence.** Data root is `dirs::data_local_dir()/rshell/` (keys in `keys/`, `known_hosts`, `plugins/`). TOML config + ring-encrypted secrets.
- **Plugins.** Loaded by `PluginLoader` from `data_local_dir/rshell/plugins`. `WasmSandbox` is a scaffold (`crates/rshell-plugin-sdk/src/sandbox.rs`) — wasmtime integration is not yet implemented.
- **Git.** Trunk-based with `feat/*`, `fix/*`, `refactor/*`, `release/*` branches (no direct push to `main`). Conventional Commits; scope matches a module name (`terminal`, `ssh`, `sftp`, `transfer`, `session`, `ui`, `security`, `script`, `plugin`, `core`, `infra`).
- **Versions.** `MAJOR.MINOR.PATCH`; planned milestone tags are listed in `docs/05-development-standards.md` §3.3.

## Where to start when changing X

- New connection protocol: add `Connection` impl under `crates/rshell-protocol/src/<proto>/`, add `Protocol` variant + `AppCommand::ConnectXxx` + `CommandDispatcher` arm.
- New user-facing feature: extend `AppCommand`/`AppEvent` → add a service in the right `rshell-core/<module>/` → wire into `CommandDispatcher` → add/update ViewModel in `rshell-ui/src/view_models/` → render in `rshell-ui/src/views/`.
- New persisted secret: use `rshell-infra::crypto` (AES via `ring`), store under `data_local_dir/rshell/`, surface through `rshell-core::security::master_password`.
- New plugin extension point: extend `RShellPlugin` trait + `PluginManifest` in `rshell-plugin-sdk`, document in `docs/03-detailed-design.md`.

## Things that are intentionally incomplete

These exist as scaffolding and may be referenced but are not yet wired up — be careful when adding tests or docs that assume they work:

- **RDP full handshake**: `crates/rshell-protocol/src/rdp/mod.rs` completes X.224 negotiation via `ironrdp-async::connect_begin`, but the TLS upgrade + NLA (CredSSP) + ActiveStage frame pump are not yet implemented. `RdpConnection::recv` always returns 0; frame data flows through `take_frame_receiver()` once the pump lands.
- **GPUI focus & keyboard input**: `TerminalView` renders `TerminalBufferSnapshot` but does not yet capture keystrokes and convert them to `AppCommand::SendInput`. Adding `.track_focus(&focus_handle).on_key_down(cx.listener(...))` requires real GPUI 0.2 runtime testing that this environment cannot run (Metal toolchain missing).
- **GPUI text inputs**: `compose_pane_view.rs` and `quick_commands_view.rs` still render static `div()` placeholders for text input rather than `gpui_component::input::TextInput` (declared in `Cargo.toml` but not imported anywhere).
- **rshell-ui event routing**: `RshellApp::process_events` only mutates local `tabs`/`sessions` state; it does not yet forward events to mounted ViewModels via `cx.update`. A future rev should route `AppEvent` into `SessionViewModel::handle_event` etc.
- **AppCommand::CopySelection**: dispatcher still logs "not yet implemented" — pending TerminalView selection-copy wiring.