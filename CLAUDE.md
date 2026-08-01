# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repo is

RShell — a cross-platform (Windows / macOS / Linux) Xshell-like remote terminal & SFTP client, written in Rust with a **Tauri 2.0 + Vue 3 web frontend**. Workspace version `0.1.0`, Rust `1.90` (see `rust-toolchain.toml`). All Chinese-language design docs live under `docs/` (read `docs/02-project-plan.md`, `docs/03-detailed-design.md`, and `docs/05-development-standards.md` before making non-trivial changes — they encode the architecture and conventions).

> **切片 0 状态（2026-07-31）**：rhai 已开 `sync` feature（`assert_impl_all!(CommandDispatcher: Send, Sync)` 编译期通过）；前端从 React 18 切换为 Vue 3 + Element Plus + pinia + dockview-vue；Tauri 壳雏形已建（`AppState` + `list_sessions` 真命令 + 1 MiB `Channel` 基线命令 `push_one_mb`）。详见 [docs/superpowers/specs/2026-07-31-tauri2-frontend-backend-split-design.md](docs/superpowers/specs/2026-07-31-tauri2-frontend-backend-split-design.md) §7.2 / §10 与 `docs/superpowers/plans/`。

## Common commands

```bash
# 后端 (Rust workspace 在 src-tauri/)
cd src-tauri
cargo build --workspace          # 编译所有后端 crate
cargo check -p rshell             # 快速检查主二进制
cargo test --workspace            # 跑全部单元测试

# 前端 (Vite + Vue 3 + TS, 仓库根)
npm install                       # 安装前端依赖
npm run dev                       # 启动 Vite dev server (http://localhost:1420)
npm run build                     # 产出 dist/ 给 Tauri 加载
npm run typecheck                 # vue-tsc --noEmit
npm test                          # Vitest (jsdom 环境, js + ts)

# Tauri 集成开发 (需要 tauri-cli: cargo install tauri-cli --version "^2.0")
cd src-tauri
cargo tauri dev                   # 同时跑 Rust + 前端,开窗口
cargo tauri build                 # 打包成安装包

# Lint / format
cargo clippy --workspace --all-targets
cargo fmt --all
```

Release profile enables LTO + `codegen-units = 1` + `strip` (`src-tauri/Cargo.toml`). Dev profile keeps `debug = true`. Cross-platform PTY lives in `src-tauri/crates/rshell-infra/src/pty/{unix,windows}.rs`.

## Workspace layout

```
rshell/                                  ← 仓库根
├── src/                                 ← 前端代码 (React + TS + Vite)
│   ├── main.tsx                         # React 入口
│   ├── App.tsx                          # 主布局
│   ├── ipc/                             # Tauri invoke/emit 封装
│   ├── store/                           # 状态管理 (zustand 等)
│   ├── views/                           # 视图组件
│   ├── components/                      # 可复用组件
│   └── styles.css                       # CSS 变量 (SPEC §8.1)
│
├── src-tauri/                           ← Rust workspace + Tauri 壳
│   ├── Cargo.toml                       # workspace manifest
│   ├── tauri.conf.json                  # Tauri 配置
│   ├── build.rs                         # tauri_build::build()
│   ├── capabilities/default.json        # 权限声明
│   ├── icons/                           # 应用图标
│   ├── src/                             # Tauri 主二进制
│   │   ├── main.rs                      # 入口
│   │   └── lib.rs                       # tauri::Builder + commands
│   └── crates/                          # 业务 crate
│       ├── rshell-api/                  # 零运行时依赖边界层
│       ├── rshell-core/                 # 业务逻辑 (不依赖 Tauri/GPUI)
│       ├── rshell-protocol/             # SSH/Telnet/Serial/RDP
│       ├── rshell-infra/                # crypto/storage/PTY
│       ├── rshell-plugin-sdk/           # 插件 SDK
│       └── xtask/                       # 自定义构建脚本
│
├── docs/                                # 设计文档 (中文)
├── scripts/                             # build.sh / build.cmd / build.ps1
├── package.json                         # 前端依赖 + 脚本
├── vite.config.ts
├── tsconfig.json
├── index.html
└── CLAUDE.md
```

## Big-picture architecture: Rust backend ↔ Web frontend via Tauri IPC

This is the single most important invariant. `docs/05-development-standards.md` §2 codifies it; violating it breaks the codebase.

**Rule 1.** `rshell-core`, `rshell-protocol`, `rshell-infra`, `rshell-plugin-sdk` MUST NOT depend on `tauri`. They stay pure Rust async libraries.
**Rule 2.** Frontend (`src/`) MUST NOT reach into backend services directly. The only entry points are `invoke()` (TS → Rust command) and `listen()` / `emit` (Rust → TS event).
**Rule 3.** The sole typed contract between the two halves is `rshell-api` — enums `AppCommand` (UI→backend, intent) and `AppEvent` (backend→UI, snapshot). All IPC payloads serialize as one of these.

### How a click becomes a render

1. Vue 3 component handles input → calls `invoke<AppCommand, AppEvent>(cmd)` from `@tauri-apps/api/core`.
2. Tauri's IPC router delivers the JSON to `src-tauri/src/commands.rs` — a `#[tauri::command]` function that **directly** invokes `state.dispatcher.dispatch(AppCommand)` (设计 §3 / §1.2：D5 让 `CommandDispatcher` 走多线程 runtime + Tauri `State`,不再走中间 channel)。返回 `Result<T, IpcError>`（§3.5）;读命令经 `CommandOutcome` 解包 (§3.2)。
3. `CommandDispatcher::dispatch` matches on every `AppCommand` variant and routes to the right service (`SessionService`, `TransferService`, `KeyManager`, etc.). Terminal bytes bypass the dispatcher entirely via `TerminalChannels::attach` / `push`（设计 §4.1 双态 sink —— 高频路径专用通道,不污染全局事件总线）。
4. Services do their async work and call `EventBus::publish` with `AppEvent` snapshots.
5. `events::subscribe_bridge` in `src-tauri/src/events.rs` subscribes to the EventBus on Tauri setup, and forwards each `AppEvent` via `app.emit("rshell://event", payload)`（事件名见 `src/ipc/events.ts:10`）。
6. The frontend store listens via `listen<AppEvent>("rshell://event", handler)` and updates state, triggering re-render.

### Conventions for `AppCommand`, `AppEvent`, and `CommandOutcome`

- `AppCommand` (`src-tauri/crates/rshell-api/src/commands.rs`): verb+noun, intent, all named fields (no tuple variants), every field `Serialize + Deserialize + Clone`. Add new variants here, extend the `match` in `CommandDispatcher::dispatch`, AND add a `#[tauri::command]` wrapper in `src-tauri/src/commands.rs`.
- `AppEvent` (`src-tauri/crates/rshell-api/src/events.rs`): past tense, full snapshot data attached (UI never has to re-query). High-frequency events carry binary payload via Tauri `Channel` (not serialized as JSON) — 设计 §1 D1：终端原始字节经 Channel 直推前端 xterm.js,事件总线不再承担高频路径。
- `CommandOutcome` (`src-tauri/crates/rshell-api/src/outcome.rs`): 读命令返回值类型,设计 §3.2。`ListTriggers` / `ListQuickCommands` / `ListSessions` 等以前仅 publish `*Changed` 事件导致死循环的分支,已改为返回 `Triggers(Vec<_>)` / `QuickCommands(Vec<_>)` / `Sessions(Vec<_>)` 等变体。9 个 `*Snapshot` 事件 + `ClipboardCopy` 已删除（切片 2.2 / §3.3）。
- TS bindings: hand-written for now. `ts-rs 12` 已加入 workspace 依赖（切片 2.3），全量 `#[derive(TS)]` 等 types.rs 按域 derive 后挂 CI 护栏（`git diff --exit-code src/ipc/generated.ts`）。Auto-generation via `specta` 是更远期规划。

## Other conventions worth knowing

- **Errors.** `thiserror` enums per module; `CoreError` aggregates at the crate root (`crates/rshell-core/src/error.rs`). No `unwrap`/`expect` in production code. `?` for conversion via `From`. Tauri commands return `Result<T, IpcError>` (`src-tauri/src/error.rs`,设计 §3.5);`IpcError` 的 `kind` 字段是稳定机器可读判别串,前端据此分支处理。
- **Logging.** `tracing` crate, structured fields (`info!(session_id = %id, host = %config.host, "…")`), `#[instrument]` on the entry points of `CommandDispatcher::dispatch`, `SessionService::connect`, etc. Init in `src-tauri/src/lib.rs` with env filter defaulting to `info,rshell=debug`.
- **Async.** `tokio` everywhere; `tokio::spawn` for I/O, `spawn_blocking` for CPU work, `tokio::select!` for timeouts, `tokio::sync::mpsc` for channels. `rhai` is single-threaded, hence the dedicated backend task.
- **Persistence.** Data root is `dirs::data_local_dir()/rshell/` (keys in `keys/`, `known_hosts`, `plugins/`). TOML config + ring-encrypted secrets.
- **Plugins.** Loaded by `PluginLoader` from `data_local_dir/rshell/plugins`. `WasmSandbox` is a scaffold (`crates/rshell-plugin-sdk/src/sandbox.rs`) — wasmtime integration is not yet implemented.
- **Git.** Trunk-based with `feat/*`, `fix/*`, `refactor/*`, `release/*` branches (no direct push to `main`). Conventional Commits; scope matches a module name (`terminal`, `ssh`, `sftp`, `transfer`, `session`, `ui`, `security`, `script`, `plugin`, `core`, `infra`).
- **Versions.** `MAJOR.MINOR.PATCH`; planned milestone tags are listed in `docs/05-development-standards.md` §3.3.

## Where to start when changing X

- New connection protocol: add `Connection` impl under `src-tauri/crates/rshell-protocol/src/<proto>/`, add `Protocol` variant + `AppCommand::ConnectXxx` + `CommandDispatcher` arm.
- New user-facing feature: extend `AppCommand`/`AppEvent` → add a service in the right `src-tauri/crates/rshell-core/<module>/` → wire into `CommandDispatcher` → add a `#[tauri::command]` wrapper + emit helper in `src-tauri/src/` → render in `src/views/`.
- New persisted secret: use `rshell-infra::crypto` (AES via `ring`), store under `data_local_dir/rshell/`, surface through `rshell-core::security::master_password`.
- New plugin extension point: extend `RShellPlugin` trait + `PluginManifest` in `rshell-plugin-sdk`, document in `docs/03-detailed-design.md`.

## Things that are intentionally incomplete

These exist as scaffolding and may be referenced but are not yet wired up — be careful when adding tests or docs that assume they work:

- **RDP full handshake**: `src-tauri/crates/rshell-protocol/src/rdp/mod.rs` completes X.224 negotiation via `ironrdp-async::connect_begin` and exposes an `RdpState::X224Only` / `Active` distinction, but the actual TLS upgrade (tokio-rustls wrapping the framed stream) + NLA (CredSSP via sspi) + ActiveStage frame pump + ironrdp-graphics SoftDisplay → RGBA conversion are not yet implemented. `RdpConnection::recv` always returns 0; frame data flows through `take_frame_receiver()` once the pump lands. The ironrdp screenshot example in the upstream repo is the reference structure.
- **Tauri migration in progress** (branch `refactor/tauri-migration`): 截至 2026-07-31 切片 2 完成,`AppCommand` / `AppEvent` / `CommandOutcome` 契约已通,9 个首批 `#[tauri::command]` 薄壳已实现,前端 Vue 3 + Element Plus + dockview-vue + xterm.js 链路已跑通（切片 0/1/2 累计实现）。仍待实现的视图迁移:`theme_settings_view` / `key_management_view` / `plugin_manager_view` / `tunnel_panel_view` / `transfer_queue` 等业务面板（按切片 3~9 推进）。`crates/rshell-ui/` 已在基线 commit 删除。