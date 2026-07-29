# 更新日志

本项目的所有重要变更都会记录在此文件中。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

---

## [Unreleased]

### 计划中
- Phase 2（M2）SFTP 双窗格文件管理器
- Phase 3（M3）快速命令、触发器、脚本引擎完善
- Phase 4（M4）密钥管理与端口转发隧道
- Phase 5（M5）Telnet / Serial / RDP 协议完善
- Phase 6（M6）WASM 插件系统落地

---

## [0.1.0] - 2026-07-29

### 已完成（Initial Commit）

RShell 项目初始提交，包含完整的 workspace 结构、协议设计与基础服务实现。

#### 工作区与工具链
- Cargo workspace 初始化（6 个 crate）
- 锁定 Rust 工具链至 1.80（含 rustfmt / clippy 组件）
- 启用 release profile LTO、`codegen-units = 1`、`strip`
- 启用 `.cargo/config.toml` 中的 `xtask` 别名（xtask crate 待落地）

#### crate 结构
- **`rshell-api`**：零运行时依赖的前后端边界层，定义 `AppCommand` / `AppEvent` 及共享数据类型
- **`rshell-infra`**：基础设施 — AES 加密（ring）、TOML 持久化、跨平台 PTY 抽象
- **`rshell-protocol`**：SSH（russh 0.48 + russh-sftp）、Telnet、Serial、RDP 协议，统一 `Connection` trait
- **`rshell-core`**：后端业务逻辑 — 终端（alacritty_terminal）、会话、传输、安全、脚本（rhai）、主题、事件总线、命令分发器
- **`rshell-plugin-sdk`**：插件 SDK — `RShellPlugin` trait、`PluginLoader`、`WasmSandbox`（脚手架）
- **`rshell-ui`**：GPUI 前端 — 应用入口、`AppBridge`、根组件 `RshellApp`、View / ViewModel

#### 架构
- 严格的前后端分离：后端代码不引用 `gpui`，前端 View 通过 `rshell-api` 的 Command/Event 与后端通信
- 后端运行在专用 OS 线程上（rhai ScriptEngine 非 Send），前端通过 `mpsc::UnboundedSender<AppCommand>` 与共享 `Mutex<Vec<AppEvent>>` 与之交互
- `EventBus`（基于 `RwLock<Vec<(id, Box<dyn Fn>)>>`）作为后端→前端的事件通道
- `CommandDispatcher` 集中路由所有 `AppCommand` 到对应 Service

#### 已实现功能（v0.1.0 范围）
- SSH 连接（含密码 / 公钥 / 键盘交互认证）
- 终端 VT 解析与缓冲（基于 alacritty_terminal）
- 多标签会话管理
- 会话 CRUD + 持久化（TOML）
- SFTP 上传 / 下载队列
- 快速命令、撰写窗格、同步输入、触发器
- SSH 密钥生成 / 导入 / 导出 / 删除
- 主密码保护
- 主机密钥信任管理
- 端口转发隧道（Local / Remote / Dynamic）
- 应用主题与终端配色方案切换
- 浅色 / 深色 / 跟随系统主题
- 插件扫描 / 加载 / 卸载 / 启用 / 禁用框架

#### 文档
- 设计文档：`docs/01-xshell-xftp-feature-research.md` ~ `docs/07-project-setup-guide.md`
- UI 设计预览：`docs/ui-design-preview.html`
- 开源文档：`README.md`、`LICENSE`、`CONTRIBUTING.md`、`CLAUDE.md`

#### 已知未完成（脚手架）
- `WasmSandbox`（`rshell-plugin-sdk/src/sandbox.rs`）— 尚未集成 wasmtime
- `rdp/` 模块 — 仅声明
- `AppCommand::CopySelection` — 分发器明确标注未实现
- `cargo xtask` 别名 — xtask crate 尚未加入 workspace

[Unreleased]: https://github.com/letmlook/rshell/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/letmlook/rshell/releases/tag/v0.1.0