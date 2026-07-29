# RShell

> 现代化的跨平台远程终端与文件传输客户端 — Xshell/Xftp 的 Rust 原生替代
>
> Built with Rust + GPUI. Apache-2.0 licensed.

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](rust-toolchain.toml)
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

需要 **Rust 1.80 或更高**（版本在 `rust-toolchain.toml` 中锁定）。

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

### 路线图

| 里程碑 | 阶段 | 状态 |
|--------|------|------|
| M1 | MVP 核心终端（SSH + VT 解析） | 进行中 |
| M2 | 文件传输（SFTP 双窗格） | 计划中 |
| M3 | 效率工具（快速命令 / 触发器 / 脚本） | 计划中 |
| M4 | 安全与隧道 | 计划中 |
| M5 | 多协议完善（Telnet / Serial / RDP） | 计划中 |
| M6 | 插件生态（WASM 沙箱） | 计划中 |

详细交付计划见 `docs/02-project-plan.md` §2。

### 贡献

欢迎贡献！请阅读 [CONTRIBUTING.md](CONTRIBUTING.md) 了解开发流程、规范与提交约定。

### 许可证

本项目采用 [Apache License 2.0](LICENSE)。