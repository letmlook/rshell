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
| **M1** MVP 核心终端 | Workspace 骨架、严格前后端分离、VT 解析（alacritty_terminal）、PTY、SSH 客户端 | ✅ | ⚠️ | SSH 客户端基于 russh 已实现密码/公钥认证；但 `check_server_key` 的 known_hosts 校验未完成；`views/terminal_view.rs` 仅占位文本，GPUI 网格渲染未实现；`app.rs` 终端区域显示"终端输出区域"字样 |
| **M2** SFTP 文件传输 | SftpClient（russh-sftp）、TransferService 队列、断点续传、暂停/恢复/取消 | ✅ | ⚠️ | `transfer/service.rs` (446 行) 实现完整状态机；`SftpClient` (252 行) 真实可用；但 `FileManagerView` (343 行) 仍为占位，目录浏览需走 `BrowseRemoteDir` Command |
| **M3** 效率工具 | 快速命令、触发器（regex/exact）、撰写窗格、同步输入、rhai 脚本引擎 | ✅ | ⚠️ | 所有 Service / Engine 真实实现并接入 `CommandDispatcher`；对应 View 文件存在但渲染为占位；脚本录制未实现 |
| **M4** 安全与隧道 | SSH 密钥生成/导入/导出、主密码、主机密钥信任、Local/Remote/Dynamic 转发 | ✅ | ⚠️ | `KeyManager` / `MasterPassword` / `HostKeyManager` / `TunnelManager` 全部实现；`KeyManagementView` 存在但未挂载到 `RshellApp` |
| **M5** 多协议 | SSH、Telnet、Serial、RDP | ⚠️ | ❌ | SSH ✅；Telnet 选项协商完成但 `resize` 标注"简化实现：暂不支持"；Serial 全部方法为 stub（缺 `serialport`）；RDP 仅结构体（缺 `ironrdp`） |
| **M6** 插件生态 | PluginLoader、WASM 沙箱、扩展点 | ⚠️ | ❌ | `PluginLoader` (191 行) 实现扫描/加载/卸载；`RShellPlugin` trait / `PluginManifest` / `PluginConfigStore` 完成；`WasmSandbox` (105 行) 全部方法为 stub（缺 `wasmtime`）；`PluginManagerView` 占位 |

#### 已知问题 & 下一步

1. **工具链**：已升级 `rust-toolchain.toml` 至 1.90（registry 已要求 ≥1.86），`Cargo.lock` 中的 `hashbrown 0.17.1`（需 edition2024）现与声明的工具链兼容，`cargo build` 可成功跑通。
2. **GPUI 终端渲染（关键路径）**：`crates/rshell-ui/src/views/terminal_view.rs` 需要接入 `TerminalBufferSnapshot`，把后端 `alacritty_terminal::Term` 状态投影到 GPUI 网格。
3. **WASM 沙箱落地**：在 `rshell-plugin-sdk/Cargo.toml` 加入 `wasmtime` 依赖，实现 `WasmSandbox::load` / `execute`。
4. **Serial / RDP 真实实现**：分别引入 `serialport` 与 `ironrdp` crate。
5. **`cargo xtask` 别名**：`.cargo/config.toml` 已声明 `xtask = "run --package xtask --"`，但 workspace 中尚无 `xtask` crate，调用会报"package not found"。
6. **UI 视图挂载**：`RshellApp::render` 当前直接内联渲染占位布局，未使用 `views/` 中已写好的组件；需要重构以正确挂载 `SessionView` / `TerminalView` / `TransferView` 等。
7. **`rshell-ui` 直接 `use rshell_core::*` 的违规**：当前 `rshell-ui/src/bridge.rs` 与 `main.rs` 通过 `AppBridge` 调用后端 Service，**未违反**架构约束（§2 允许这两个文件接触 core）；但需要持续在 Code Review 中守住这条边界。

详细设计目标见 `docs/02-project-plan.md` §2，模块级状态见 `CLAUDE.md` 末尾「Things that are intentionally incomplete」。

### 贡献

欢迎贡献！请阅读 [CONTRIBUTING.md](CONTRIBUTING.md) 了解开发流程、规范与提交约定。

### 许可证

本项目采用 [Apache License 2.0](LICENSE)。