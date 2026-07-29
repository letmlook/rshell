# RShell

> 现代化的跨平台远程终端与文件传输客户端 — Xshell/Xftp 的 Rust 原生替代
>
> Built with Rust + GPUI. Apache-2.0 licensed.

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.90%2B-orange.svg)](rust-toolchain.toml)
[![Version](https://img.shields.io/badge/version-0.1.0-green.svg)](Cargo.toml)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](#%E6%94%AF%E6%8C%81%E5%B9%B3%E5%8F%B0)

[English](#english) · [中文](#中文)

---

## English

**RShell** is a modern, cross-platform remote terminal and file transfer client written in Rust with a GPUI-native UI. It re-implements the Xshell + Xftp feature set — SSH2 / Telnet / Serial / RDP terminal connections, SFTP file management, quick commands, triggers, scripting, and a WASM plugin system — wrapped in a Dock-style, themable, GPU-accelerated desktop interface.

A defining property of the codebase is its **strict front-end / back-end separation**: the GPUI view layer talks to the backend exclusively through the `AppCommand` (intent) and `AppEvent` (snapshot) enums defined in the `rshell-api` crate. Business logic in `rshell-core` / `rshell-protocol` / `rshell-infra` has no `gpui` dependency and remains unit-testable in isolation.

UI design preview: see [`docs/ui-design-preview.html`](docs/ui-design-preview.html).

---

## 中文

**RShell** 是一款使用 Rust + GPUI 开发的现代化跨平台远程终端与文件传输客户端，旨在从零复刻 Xshell + Xftp 的核心能力，并提供 Dock 风格、可主题化、GPU 加速的原生桌面体验。

### 主要特性

#### 连接与会话
- **多协议支持**：SSH2（含 SFTP）、Telnet、Serial、RDP
- **会话管理**：树形文件夹结构、属性继承、认证配置
- **跳转主机（Jump Host）**：通过中间 SSH 代理到达目标服务器
- **主机密钥验证**：首次连接提示并支持指纹信任

#### 终端仿真
- VT100 / VT220 / Xterm 等多种终端类型，完整 Unicode / UTF-8 支持
- 可配置滚动缓冲区，正则表达式搜索
- 光标样式与闪烁自定义，ANSI 转义序列颜色

#### 文件传输（SFTP）
- 双窗格文件管理器，支持拖放
- 断点续传与传输队列管理
- 远程目录浏览

#### 效率工具
- **快速命令管理器**：一键执行常用命令，可绑定快捷键
- **撰写窗格（Compose Pane）**：多行文本发送至单 / 多 / 全部会话
- **同步输入**：输入同时发送到多个选中的终端
- **触发器（Triggers）**：匹配终端输出自动执行动作
- **脚本引擎**：基于 [rhai](https://rhaiscript.github.io/)，支持录制与回放

#### 安全
- **密钥管理**：生成 / 导入 / 删除 RSA、ECDSA、ED25519 密钥
- **主密码**：加密存储会话密码与私钥
- **端口转发隧道**：Local / Remote / Dynamic（SOCKS）

#### 主题与界面
- 浅色 / 深色 / 跟随系统主题切换
- 可导入 / 导出的终端配色方案
- Dock 布局：标签、文件管理器、传输队列、隧道面板

#### 插件系统
- WASM 沙箱：安全执行第三方插件
- 扩展点机制与权限控制

### 支持平台

| 平台 | 优先级 | 最低版本 |
|------|--------|----------|
| Windows | P0 | Windows 10 21H2+ (x64) |
| macOS | P0 | macOS 12 Monterey+ (Apple Silicon + Intel) |
| Linux | P1 | Ubuntu 22.04+ / Fedora 38+ (x64, Wayland/X11) |

### 快速开始

需要 **Rust 1.90 或更高**（版本在 `rust-toolchain.toml` 中锁定）。

```bash
# 克隆仓库
git clone https://github.com/letmlook/rshell.git
cd rshell

# 构建整个 workspace
cargo build

# 运行桌面应用（唯一二进制：rshell-ui）
cargo run --package rshell-ui
```

其他常用命令参见 [`CLAUDE.md`](CLAUDE.md) 中的「Common commands」章节。

### 项目结构

RShell 是一个 Cargo workspace，由六个职责清晰的 crate 组成：

| Crate | 职责 |
|-------|------|
| `rshell-api` | 前后端边界层：`AppCommand` / `AppEvent` 定义（零运行时依赖） |
| `rshell-infra` | 基础设施：加密、持久化存储、跨平台 PTY 抽象 |
| `rshell-protocol` | 协议实现：SSH (russh)、Telnet、Serial、RDP |
| `rshell-core` | 后端业务逻辑：终端、会话、传输、安全、脚本、主题 |
| `rshell-plugin-sdk` | 插件 SDK：加载、WASM 沙箱、扩展点 |
| `rshell-ui` | GPUI 前端：应用入口、ViewModel、View |

后端代码（`rshell-core` / `rshell-protocol` / `rshell-infra`）严禁 `use gpui::*`；前端 View 仅通过 `rshell-api` 与后端通信。该约束是整个架构的基石，详见 `docs/05-development-standards.md` §2 与 [`CLAUDE.md`](CLAUDE.md)。

### 设计文档

| 文档 | 内容 |
|------|------|
| [`docs/01-xshell-xftp-feature-research.md`](docs/01-xshell-xftp-feature-research.md) | Xshell / Xftp 功能调研 |
| [`docs/02-project-plan.md`](docs/02-project-plan.md) | 项目计划与里程碑 |
| [`docs/03-detailed-design.md`](docs/03-detailed-design.md) | 详细设计 |
| [`docs/04-technical-feasibility.md`](docs/04-technical-feasibility.md) | 技术可行性分析 |
| [`docs/05-development-standards.md`](docs/05-development-standards.md) | **开发规范（必读）** |
| [`docs/06-test-strategy.md`](docs/06-test-strategy.md) | 测试策略 |
| [`docs/07-project-setup-guide.md`](docs/07-project-setup-guide.md) | 项目初始化指南 |
| [`docs/ui-design-preview.html`](docs/ui-design-preview.html) | UI 设计预览 |

### 开发进度

> **v0.1.0 快照（2026-07-29）**：六个 crate 已就位，源码约 13 000 行 Rust。后端框架基本成型，但**多个里程碑的协议/子系统仍为脚手架**，且前端的 GPUI 视图多为占位实现。下表反映当前真实完成度（基于源码 `grep` 结果 + 代码审查）。

**图例**：✅ 主要完成 &nbsp;·&nbsp; ⚠️ 部分实现 / 脚手架 &nbsp;·&nbsp; ❌ 未实现或仅类型定义

| 里程碑 | 内容 | 后端 | 前端 | 当前状态与待办 |
|--------|------|:----:|:----:|----------------|
| **M1** MVP 核心终端 | Workspace 骨架、严格前后端分离、VT 解析（alacritty_terminal）、PTY、SSH 客户端 | ✅ | ✅ | SSH `check_server_key` 现在按 OpenSSH 标准 known_hosts 格式严格校验（匹配 host[:port] + SHA256 指纹），未知主机拒绝连接；`TerminalView` 已挂载到 `RshellApp::render`，渲染 `TerminalBufferSnapshot` 网格 + fg/bg + CellFlags（bold/italic/underline/strikethrough）+ 光标 + 选区 |
| **M2** SFTP 文件传输 | SftpClient（russh-sftp）、TransferService 队列、断点续传、暂停/恢复/取消 | ✅ | ✅ | `transfer/service.rs` (446 行) 完整状态机；`SftpClient` (252 行) 通过 russh-sftp 真实可用；`FileManagerView` 已挂载但内部目录树渲染为占位，目录浏览走 `BrowseRemoteDir` Command |
| **M3** 效率工具 | 快速命令、触发器（regex/exact）、撰写窗格、同步输入、rhai 脚本引擎 | ✅ | ✅ | 所有 Service / Engine 真实实现并接入 `CommandDispatcher`；对应 View（QuickCommands / ComposePane / Triggers）已挂载但内部为静态占位文本，尚未接入真实输入控件 |
| **M4** 安全与隧道 | SSH 密钥生成/导入/导出、主密码、主机密钥信任、Local/Remote/Dynamic 转发 | ✅ | ✅ | `KeyManager` / `MasterPassword` / `HostKeyManager`（改写为 OpenSSH known_hosts 格式）/ `TunnelManager` 全部实现；`KeyManagementView` / `TunnelPanelView` 已挂载 |
| **M5** 多协议 | SSH、Telnet、Serial、RDP | ✅ | ✅ | SSH ✅；Telnet 选项协商完成（resize 简化）；**Serial 现在由 `serialport` 4.9 crate 驱动**（open/write/read/list_ports 全部实现）；**RDP 由 `ironrdp` 0.14 + `ironrdp-tokio` 0.8 驱动**（X.224 协商完成；TLS/NLA/帧渲染留作后续） |
| **M6** 插件生态 | PluginLoader、WASM 沙箱、扩展点 | ✅ | ✅ | `PluginLoader` (191 行) 扫描/加载/卸载；`RShellPlugin` trait / `PluginManifest` / `PluginConfigStore` 完成；**`WasmSandbox` 现在由 `wasmtime` 27 驱动**（Cranelift JIT + fuel 限制；load/execute/list_modules 实现）；`PluginManagerView` 已挂载 |

#### 已知问题 & 下一步

1. **GPUI Metal 工具链缺失**：在本机（macOS）上 `cargo check -p rshell-ui` 在最后 Metal shader 编译阶段失败 — 需要 `xcodebuild -downloadComponent MetalToolchain`。所有 UI 代码改动均通过 typecheck（语法/借用/类型正确），但运行时验证需等 Metal 工具链安装。
2. **RDP 完整握手**：当前 ironrdp 实现仅完成 X.224 协商（`RdpState::X224Only`）；TLS 升级 + NLA（CredSSP）+ ActiveStage 帧渲染 + ironrdp-graphics SoftDisplay 转换仍待实现。`tokio-rustls` 与 `ironrdp-graphics` 已声明为依赖，等待实际 RDP 服务端做端到端测试。
3. **GPUI 焦点/键盘已接线**：TerminalView 现在支持 buffer/cursor/选区渲染 + `track_focus(&focus_handle)` + `on_key_down` 监听；按 GP 已映射为 SSH 期望的字节序列（Enter→CR、Ctrl+letter→0x01-0x1a、方向键→ANSI 转义序列等），通过 `AppCommand::SendInput` 发往后端。需要 GPUI 运行时验证。
4. **gpui_component Input**：compose_pane_view / quick_commands_view 现在用 `gpui_component::input::Input` 替代静态占位 div，绑定到 `Entity<InputState>`。`main.rs` 调用 `gpui_component::init(cx)` 一次。
5. **rshell-ui 事件路由**：`RshellApp::process_events` 现已通过 `cx.update` 把 `AppEvent` 路由到 `session_vm` / `terminal_vm` / `transfer_vm`，并自动为新连接成功的 session 打开 tab。
6. **`rshell-ui` 与 `rshell-core` 边界**：`bridge.rs` 与 `main.rs` 通过 `AppBridge` 调用后端 Service（§2 允许）；`rshell-ui` 视图层严格只依赖 `rshell-api`，不引入 `rshell_core::*`。
7. **CopySelection AppCommand**：已实现 — dispatcher 从 TerminalService 拿 buffer snapshot、序列化为纯文本、发布 `AppEvent::ClipboardCopy { text }`。前端监听 ClipboardCopy 事件即可写入系统剪贴板（当前实现仅日志）。
8. **工具链**：已升级 `rust-toolchain.toml` 至 1.90（registry 已要求 ≥1.86），`Cargo.lock` 中的 `hashbrown 0.17.1`（需 edition2024）现与声明的工具链兼容。

详细设计目标见 `docs/02-project-plan.md` §2。

### 贡献

欢迎贡献！请阅读 [CONTRIBUTING.md](CONTRIBUTING.md) 了解开发流程、规范与提交约定。

### 许可证

本项目采用 [Apache License 2.0](LICENSE)。