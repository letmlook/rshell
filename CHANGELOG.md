# 更新日志

本项目的所有重要变更都会记录在此文件中。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

---

## [Unreleased]

### Changed — 完成 v0.1.0 完整化

#### 工具链
- 升级 `rust-toolchain.toml` 至 Rust 1.90（registry 已要求 ≥1.86）；同步更新 `Cargo.toml` workspace `rust-version`、`CLAUDE.md`、`README.md`、`docs/07-project-setup-guide.md`。
- 升级 `alacritty_terminal` 0.24 → 0.25 以匹配新版 rustix API；修复 `rshell-infra` PTY Unix 实现缺失的 `std::io::{Read, Write}` 导入。

#### 新增
- **`crates/xtask/`**：clap 驱动的任务运行器，子命令 `fmt` / `lint` / `test` / `dev` / `build` / `xtask-help`。`.cargo/config.toml` 中已存在的 `cargo xtask` 别名现在真正可用。
- **SSH 主机密钥校验**：`SshHandler::check_server_key` 现在按 OpenSSH 标准 `known_hosts` 格式（`<host_pattern> <keytype> <base64-key>`）严格匹配 host[:port] + SHA256 指纹，未知主机**拒绝连接**（之前是静默接受）。`HostKeyManager` 重写为 OpenSSH 文件格式，与 `ssh-keygen` 等工具互操作。
- **Serial（`crates/rshell-protocol/src/serial/mod.rs`）**：基于 `serialport` 4.9 实现真实串口通信。`open` / `write` / `read` / `list_ports` 全部就绪；阻塞 I/O 通过 `tokio::task::spawn_blocking` 包装；`SerialPort`（非 `Sync`）由 `Arc<Mutex<Box<dyn SerialPort>>>` 持有。
- **RDP（`crates/rshell-protocol/src/rdp/mod.rs`）**：基于 `ironrdp` 0.14 + `ironrdp-tokio` 0.8 + `ironrdp-connector` 0.8 + `ironrdp-async` 0.8 + `ironrdp-pdu` 0.7。TCP + `TokioFramed` + `connect_begin` 完成 X.224 协商。`RdpFrame` 通过独立 mpsc 通道对外提供。⚠️ TLS 升级 + NLA 认证 + ActiveStage 帧渲染留作后续工作（标记在 README「已知问题」中）。
- **WASM 沙箱（`crates/rshell-plugin-sdk/src/sandbox.rs`）**：基于 `wasmtime` 27（Cranelift JIT）。Engine 启动 `consume_fuel`；`Store::set_fuel` 近似 `max_execution_time_ms` 限制执行时间；`Module::new` / `Instance::new` / `Func::call` 全部走通；测试通过 `(2, 3) == 5` 的 WAT `add` 函数验证。

#### 重构
- **`crates/rshell-ui/src/views/*`**：6 个 View 的构造器从 `new(_window, _cx)` 统一为 `new(cx)` 以适配 `cx.new(|cx| ...)` 挂载闭包。
- **`crates/rshell-ui/src/app.rs`**：`RshellApp` 现在持有 10 个 `gpui::Entity<View>` 字段（FileManager / Session / Terminal / Transfer / Key / Theme / QuickCommands / Compose / Tunnel / Plugin），新增 `PanelKind` 枚举 + `render_active_panel()` 路由方法。侧边栏"会话树"占位 → 真实 `SessionView`；底部"传输队列"占位 → 真实 `TransferView`；中央"终端输出区域"占位 → 真实 `TerminalView`。
- **`crates/rshell-ui/src/views/terminal_view.rs`**：增强为支持 `CellFlags`（bold / italic / underline / strikethrough）渲染、绝对定位光标覆盖层、`Selection` 数据结构 + 选区高亮。

### 待办
- RDP TLS / NLA / ActiveStage 帧渲染（见 README「已知问题」§2）
- TerminalView 焦点 + 键盘输入捕获（GPUI 0.2 焦点机制需运行时验证）
- gpui_component 文本输入控件接入 ComposePane / QuickCommands 搜索框
- rshell-ui 事件经 `cx.update` 路由到已挂载的 ViewModel

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