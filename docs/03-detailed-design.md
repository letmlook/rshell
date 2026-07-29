# RShell 软件详细设计文档

> **文档版本**：v1.1  
> **编写日期**：2026-07-29  
> **更新日期**：2026-07-29（v1.1 前后端分离架构重构）  
> **项目名称**：RShell  
> **技术栈**：Rust + GPUI + russh  
> **参考依据**：`docs/01-xshell-xftp-feature-research.md`

---

## 目录

1. [项目概述](#1-项目概述)
2. [系统架构设计](#2-系统架构设计)
3. [模块详细设计](#3-模块详细设计)
   - 3.1 终端模拟器模块
   - 3.2 SSH / 协议连接模块
   - 3.3 会话管理模块
   - 3.4 SFTP / 文件传输模块
   - 3.5 安全模块
   - 3.6 UI 框架模块
   - 3.7 脚本与自动化模块
   - 3.8 插件化扩展模块
4. [核心数据结构设计](#4-核心数据结构设计)
5. [关键接口设计](#5-关键接口设计)
6. [第三方依赖选型](#6-第三方依赖选型)
7. [开发路线图](#7-开发路线图)
8. [UI 视觉与交互设计规范](#8-ui-视觉与交互设计规范)
   - 8.1 设计语言与视觉基础
   - 8.2 核心界面详细设计
   - 8.3 交互状态与动效规范
   - 8.4 可复用 UI 组件库

---

## 1. 项目概述

### 1.1 项目目标

RShell 是一款基于 Rust 语言与 GPUI 框架构建的**跨平台原生远程连接与文件管理工具**，旨在完整替代 Xshell + Xftp 的全部功能，提供：

- 业界领先的终端仿真体验（SSH / Telnet / Serial / RDP）
- 高效的文件传输与管理能力（SFTP / FTP / FTPS / SCP）
- 原生级别的性能与内存效率（无 Electron 开销，GPU 直接渲染）
- 现代化的用户界面与主题系统
- 可扩展的脚本与自动化能力

### 1.2 技术选型

| 维度 | 选型 | 理由 |
|------|------|------|
| 开发语言 | Rust | 内存安全、高性能、跨平台 |
| UI 框架 | GPUI 0.2 | Zed 编辑器同款 GPU 渲染框架，声明式 UI |
| UI 组件库 | gpui-component 0.5 | 60+ 原生组件，Dock / Tiles 布局，虚拟化表格 |
| SSH 协议 | russh（纯 Rust） | 异步 Tokio 友好，支持客户端/服务端 |
| 终端仿真 | alacritty_terminal | 成熟的 VT100/220/320/Xterm 仿真引擎 |
| 构建工具 | Cargo | Rust 标准构建系统 |

### 1.3 目标平台

| 平台 | 支持状态 | 说明 |
|------|----------|------|
| Windows 10/11 | 主要目标 | ConPTY 伪终端、原生窗口 |
| macOS 12+ | 主要目标 | posix_openpt 伪终端 |
| Linux (X11/Wayland) | 主要目标 | posix_openpt 伪终端 |

### 1.4 设计原则

- **前后端分离**：UI 层（前端）与业务逻辑层（后端）严格解耦，通过事件总线和命令模式通信，前端不直接调用后端内部方法，后端不感知任何 UI 组件
- **安全第一**：所有网络流量加密，主密码保护，零信任架构
- **性能优先**：GPU 渲染、零拷贝传输、异步 I/O
- **可扩展**：模块化设计，协议可插拔，脚本可扩展
- **用户友好**：现代化 UI，主题可定制，工作流优化
- **可测试性**：后端服务可独立于 UI 进行单元测试，前端可通过 Mock 后端进行开发

---

## 2. 系统架构设计

### 2.1 整体架构图（前后端分离）

```
┌─────────────────────────────────────────────────────────────────────────┐
│                       前端层 (rshell-ui crate)                          │
│                       “只负责显示和用户交互”                              │
│                                                                         │
│  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌─────────────────┐  │
│  │ 终端视图    │ │ 文件管理器  │ │ 会话树视图  │ │ 设置/隧道/日志  │  │
│  │TerminalView│ │FileMgrView │ │SessionView │ │ SettingsView    │  │
│  └─────┬──────┘ └─────┬──────┘ └─────┬──────┘ └───────┬─────────┘  │
│        │              │              │                │             │
│  ┌─────┴──────────────┴──────────────┴────────────────┴──────────┐  │
│  │            GPUI 应用框架 (Window / Dock / Tabs / Theme)        │  │
│  └────────────────────────────┬──────────────────────────────────┘  │
│                               │                                     │
│  ┌────────────────────────────┴──────────────────────────────────┐  │
│  │            ViewModel 层 (状态订阅 + 命令发送)                  │  │
│  │  TerminalVM | FileMgrVM | SessionVM | TransferVM | TunnelVM  │  │
│  └────────────────────────────┬──────────────────────────────────┘  │
├───────────────────────────────┼─────────────────────────────────────┤
│                         API 边界层                                   │
│  ┌────────────────────────────┴──────────────────────────────────┐  │
│  │              EventBus + CommandDispatcher                       │  │
│  │  (前端 → 后端: Command)    (后端 → 前端: Event/StateChange)    │  │
│  └────────────────────────────┬──────────────────────────────────┘  │
├───────────────────────────────┼─────────────────────────────────────┤
│                       后端层 (rshell-core crate)                     │
│                       “不感知任何 UI 组件”                              │
│                                                                         │
│  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌─────────────────┐  │
│  │ Terminal   │ │ Session    │ │ Transfer   │ │ Script          │  │
│  │ Emulator   │ │ Manager    │ │ Engine     │ │ Engine          │  │
│  │ Service    │ │ Service    │ │ Service    │ │ Service         │  │
│  └─────┬──────┘ └─────┬──────┘ └─────┬──────┘ └───────┬─────────┘  │
│        │              │              │                │             │
│  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌─────────────────┐  │
│  │ Tunnel     │ │ Key        │ │ Quick      │ │ Trigger         │  │
│  │ Manager    │ │ Manager    │ │ Command    │ │ Engine          │  │
│  └────────────┘ └────────────┘ └────────────┘ └─────────────────┘  │
├─────────────────────────────────────────────────────────────────────────┤
│                       协议层 (rshell-protocol crate)                    │
│  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌──────┐  │
│  │SSH2    │ │Telnet  │ │Serial  │ │SFTP    │ │FTP/FTPS│ │RDP   │  │
│  │russh   │ │tokio   │ │serial- │ │russh-  │ │suppaftp│ │ironrdp│  │
│  │        │ │tcp     │ │port    │ │sftp    │ │        │ │      │  │
│  └────────┘ └────────┘ └────────┘ └────────┘ └────────┘ └──────┘  │
├─────────────────────────────────────────────────────────────────────────┤
│                       基础设施层 (rshell-infra crate)                   │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ │
│  │ 加密引擎  │ │ 配置存储  │ │ 日志系统  │ │ 主密码   │ │ PTY 管理 │ │
│  │ ring/    │ │ serde +  │ │ tracing  │ │ keyring  │ │ portable │ │
│  │ rustls   │ │ toml     │ │          │ │          │ │ -pty     │ │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘ └──────────┘ │
└─────────────────────────────────────────────────────────────────────────┘
```

### 2.2 分层架构说明

| 层级 | Crate | 职责 | 关键约束 |
|------|-------|------|----------|
| **前端层** | `rshell-ui` | GPUI 视图渲染、用户交互事件捕获、ViewModel 状态绑定 | **仅依赖 GPUI 框架和 rshell-api（Command/Event），不直接依赖 rshell-core** |
| **API 边界层** | `rshell-api` (模块) | EventBus 事件分发 + CommandDispatcher 命令路由 | **前后端唯一通信通道，前端发 Command，后端发 Event** |
| **后端层** | `rshell-core` | 核心业务逻辑、状态管理、协议协调 | **不引用任何 GPUI 类型，通过 Event 通知前端状态变化** |
| **协议层** | `rshell-protocol` | 各远程协议的连接建立、数据收发 | 统一抽象为 `Connection` trait，可插拔扩展 |
| **基础设施层** | `rshell-infra` | 加密、存储、日志、PTY 等底层能力 | 提供跨平台抽象，屏蔽 OS 差异 |

### 2.3 前后端分离架构设计

#### 2.3.1 核心原则

```
前端 (rshell-ui)                  后端 (rshell-core)
┌─────────────┐                 ┌─────────────────┐
│  View 组件   │                 │  Service 服务    │
│  (纯渲染)    │                 │  (纯业务逻辑)    │
│             │                 │                 │
│  ViewModel  │──── Command ───▶│  Service 处理   │
│  (状态映射)  │                 │  并产生 Event    │
│             │◀─── Event ─────│                 │
│  订阅 Event  │                 │  发布 Event     │
│  更新 VM    │                 │                 │
└─────────────┘                 └─────────────────┘
```

**单向数据流**：
1. 用户操作 → View 捕获 → 转换为 **Command** → 发送到后端
2. 后端 Service 处理 Command → 状态变化 → 发布 **Event**
3. 前端 ViewModel 订阅 Event → 更新本地状态 → GPUI 自动重绘

**关键约束**：
- 前端 **不能** 直接调用后端 Service 的方法，只能通过 Command 发送请求
- 后端 **不能** 引用任何 GPUI 类型（`Entity`, `Window`, `View` 等）
- 后端 **不能** 感知 UI 状态（选中项、焦点、滚动位置等）
- 前端 ViewModel 是后端状态的“投影”，不是后端状态本身

#### 2.3.2 Command 与 Event 定义

```rust
// ===== 前端 → 后端：Command =====

/// 前端发送的所有命令
enum AppCommand {
    // 会话命令
    ConnectSession { session_id: Uuid },
    DisconnectSession { session_id: Uuid },
    CreateSession { config: SessionConfig },
    UpdateSession { id: Uuid, config: SessionConfig },
    DeleteSession { id: Uuid },

    // 终端命令
    SendInput { session_id: Uuid, data: Vec<u8> },
    ResizeTerminal { session_id: Uuid, cols: u16, rows: u16 },
    CopySelection { session_id: Uuid },

    // 文件传输命令
    EnqueueUpload { local: PathBuf, remote: String, session_id: Uuid },
    EnqueueDownload { remote: String, local: PathBuf, session_id: Uuid },
    PauseTransfer { task_id: Uuid },
    ResumeTransfer { task_id: Uuid },
    CancelTransfer { task_id: Uuid },
    BrowseRemoteDir { session_id: Uuid, path: String },

    // 隧道命令
    CreateTunnel { session_id: Uuid, rule: PortForwardRule },
    CloseTunnel { tunnel_id: Uuid },

    // 快速命令
    ExecuteQuickCommand { command_id: Uuid, target_sessions: Vec<Uuid> },
}

// ===== 后端 → 前端：Event =====

/// 后端发布的所有事件
enum AppEvent {
    // 连接状态变化
    ConnectionStateChanged {
        session_id: Uuid,
        state: ConnectionState,
        info: Option<ConnectionInfo>,
    },

    // 终端输出（高频事件，通过专用通道）
    TerminalOutput {
        session_id: Uuid,
        data: Vec<u8>,
    },
    TerminalTitleChanged {
        session_id: Uuid,
        title: String,
    },

    // 会话数据变化
    SessionListChanged,  // 会话树变更，前端重新拉取
    SessionUpdated { session_id: Uuid },

    // 传输进度
    TransferProgress { task_id: Uuid, bytes: u64, total: u64, speed_bps: f64 },
    TransferCompleted { task_id: Uuid },
    TransferFailed { task_id: Uuid, error: String },
    TransferQueueChanged,

    // 远程目录浏览结果
    RemoteDirListed {
        session_id: Uuid,
        path: String,
        entries: Vec<RemoteFileEntry>,
    },

    // 隧道状态
    TunnelStateChanged { tunnel_id: Uuid, state: TunnelState },

    // 安全事件
    HostKeyMismatch { host: String, expected: String, received: String },
    MasterPasswordRequired,
}
```

#### 2.3.3 EventBus + CommandDispatcher 实现

```rust
/// 事件总线（后端 → 前端）
/// 后端 Service 发布事件，前端 ViewModel 订阅感兴趣的事件
struct EventBus {
    subscribers: Vec<Box<dyn Fn(&AppEvent) + Send + Sync>>,
}

impl EventBus {
    /// 发布事件（后端调用）
    fn publish(&self, event: AppEvent);
    /// 订阅事件（前端调用）
    fn subscribe<F: Fn(&AppEvent) + Send + Sync + 'static>(&self, handler: F);
}

/// 命令分发器（前端 → 后端）
/// 前端发送命令，CommandDispatcher 路由到对应的 Service
struct CommandDispatcher {
    session_service: Arc<SessionService>,
    terminal_service: Arc<TerminalService>,
    transfer_service: Arc<TransferService>,
    tunnel_service: Arc<TunnelService>,
    // ...
}

impl CommandDispatcher {
    /// 分发命令（前端调用）
    async fn dispatch(&self, command: AppCommand) -> Result<()> {
        match command {
            AppCommand::ConnectSession { session_id } => {
                self.session_service.connect(session_id).await
            }
            AppCommand::SendInput { session_id, data } => {
                self.terminal_service.send_input(session_id, &data).await
            }
            AppCommand::EnqueueUpload { local, remote, session_id } => {
                self.transfer_service.enqueue_upload(local, remote, session_id).await
            }
            // ...
        }
    }
}
```

#### 2.3.4 ViewModel 模式

```rust
/// ViewModel 是后端状态在前端的“投影”
/// 每个 View 对应一个 ViewModel，ViewModel 订阅 Event 并维护本地状态

/// 终端 ViewModel
struct TerminalViewModel {
    // 本地 UI 状态（后端不感知）
    scroll_offset: usize,
    selection: Option<Selection>,
    is_search_mode: bool,
    search_query: String,

    // 后端状态投影
    session_id: Uuid,
    connection_state: ConnectionState,
    title: String,
    cols: u16,
    rows: u16,
}

impl TerminalViewModel {
    /// 处理后端事件，更新本地状态
    fn handle_event(&mut self, event: &AppEvent) -> bool {
        match event {
            AppEvent::TerminalOutput { session_id, data }
                if *session_id == self.session_id =>
            {
                // 通知终端缓冲区更新
                true  // 返回 true 表示需要重绘
            }
            AppEvent::ConnectionStateChanged { session_id, state, .. }
                if *session_id == self.session_id =>
            {
                self.connection_state = state.clone();
                true
            }
            _ => false,  // 不关心此事件
        }
    }

    /// 用户输入时发送 Command
    fn on_user_input(&self, data: Vec<u8>) -> AppCommand {
        AppCommand::SendInput {
            session_id: self.session_id,
            data,
        }
    }
}
```

#### 2.3.5 数据流示意图

```
用户点击“连接”按钮
    │
    ▼
[View] SessionView
    │ 捕获点击事件
    │ 构造 Command
    ▼
[CommandDispatcher]
    │ dispatch(ConnectSession { session_id })
    ▼
[SessionService] (后端)
    │ 解析配置 → 建立连接 → 认证成功
    │ 状态变化
    ▼
[EventBus]
    │ publish(ConnectionStateChanged { state: Connected })
    ▼
[ViewModel] SessionViewModel
    │ handle_event() → 更新本地状态
    ▼
[View] SessionView
    │ GPUI 自动重绘（连接状态图标变为绿色）
    ▼
用户看到连接成功
```

### 2.4 核心模块划分

| 模块 ID | 所属层 | 模块名称 | 职责概述 |
|---------|--------|----------|----------|
| `ui-terminal` | 前端 | 终端视图 | 终端 GPU 渲染、选区、滚动、搜索 UI |
| `ui-filemgr` | 前端 | 文件管理器视图 | 双窗格 UI、拖放、传输进度展示 |
| `ui-session` | 前端 | 会话树视图 | 会话树展示、右键菜单、拖放排序 |
| `ui-settings` | 前端 | 设置视图 | 设置表单、主题选择、配色方案编辑 |
| `ui-common` | 前端 | 通用 UI 组件 | 选项卡、状态栏、工具栏、通知 |
| `core-terminal` | 后端 | 终端服务 | VT 解析、终端缓冲区管理、触发器检测 |
| `core-session` | 后端 | 会话服务 | 会话树 CRUD、属性继承、认证配置 |
| `core-transfer` | 后端 | 传输服务 | SFTP/FTP 文件浏览、传输队列、同步 |
| `core-security` | 后端 | 安全服务 | 密钥管理、主密码、隧道/端口转发 |
| `core-script` | 后端 | 脚本服务 | 快速命令、触发器引擎、Rhai 脚本 |
| `plugin-sdk` | 后端 | 插件 SDK | 插件加载、生命周期、扩展点注册 |
| `protocol-ssh` | 协议 | SSH 协议 | SSH2 连接、认证、通道管理 |
| `protocol-telnet` | 协议 | Telnet 协议 | Telnet 连接、选项协商 |
| `protocol-serial` | 协议 | Serial 协议 | 串口通信 |
| `protocol-rdp` | 协议 | RDP 协议 | 远程桌面连接 |
| `infra-crypto` | 基础设施 | 加密服务 | 加密算法、密钥派生 |
| `infra-storage` | 基础设施 | 存储服务 | 配置持久化、加密存储 |
| `infra-pty` | 基础设施 | PTY 管理 | 跨平台伪终端创建与管理 |

---

## 3. 模块详细设计

### 3.1 终端模拟器模块

#### 3.1.1 前后端职责划分

| 职责 | 前端 (`ui-terminal`) | 后端 (`core-terminal`) |
|------|---------------------|----------------------|
| PTY 创建/销毁 | ✘ | ✔ |
| VT 转义序列解析 | ✘ | ✔ |
| 终端缓冲区维护 | ✘ | ✔ |
| 触发器检测 | ✘ | ✔ |
| 字形图集管理 | ✔ | ✘ |
| GPU 字符绘制 | ✔ | ✘ |
| 光标渲染 | ✔ | ✘ |
| 选区高亮 | ✔ | ✘ |
| 滚动位置管理 | ✔ (本地 UI 状态) | ✘ |
| 搜索 UI | ✔ | ✘ |
| 窗口 resize → 通知后端 | ✔ (发 Command) | ✔ (处理 resize) |
| 用户键盘输入 → 发送到远程 | ✔ (发 Command) | ✔ (写入 PTY) |

#### 3.1.2 模块职责（后端）

- 管理伪终端（PTY）的创建、读写与生命周期
- 解析 ANSI / VT100 / VT220 / VT320 / Xterm 转义序列
- 维护终端缓冲区（字符网格 + 属性 + 滚动历史）
- 检测触发器条件并通知事件总线
- **不包含任何渲染逻辑**，仅提供终端状态数据供前端消费

#### 3.1.3 内部架构（后端）

```
后端 (core-terminal):                    前端 (ui-terminal):
┌─────────────────────────────┐          ┌─────────────────────────────┐
│        VTParser              │          │      TerminalRenderer       │
│  - CSI/OSC/DCS 序列解析    │          │  - 字形图集(Atlas)管理      │
│  - 字符集切换 (G0-G3)       │          │  - GPU 字符批量绘制          │
└──────────┬─────────────────┘          │  - 光标绘制                  │
           │ 写入                            │  - 选区高亮                  │
           ▼                                │  - ANSI 颜色映射             │
┌─────────────────────────────┐          └──────────┬─────────────────┘
│     TerminalBuffer           │                     │ 读取
│  - 主/备用缓冲区             │                     │
│  - 滚动历史 (RingBuffer)    │                     │
│  - 光标位置/状态             │                     │
└──────────┬─────────────────┘                     │
           │ 状态变化通知                          ▼
           ▼                                ┌─────────────────────────────┐
┌─────────────────────────────┐          │      TerminalViewModel       │
│      PTY Handler             │          │  - scroll_offset (本地 UI)    │
│  - Windows: ConPTY           │          │  - selection (本地 UI)        │
│  - Unix: posix_openpt        │          │  - session_id (后端状态投影)  │
│  - 异步读写 (tokio)          │          │  - connection_state          │
└─────────────────────────────┘          └─────────────────────────────┘
```

#### 3.1.4 关键设计决策

| 决策点 | 方案 | 理由 |
|--------|------|------|
| VT 解析引擎 | 复用 `alacritty_terminal` 的 `vte` 解析器 | 工业级成熟度，覆盖所有 VT 序列 |
| 字形渲染 | GPUI `TextRun` + 字形图集缓存 | GPU 批量绘制，避免逐字符 CPU 开销 |
| 字体回退 | `font-kit` 级联回退链 | 支持 CJK / Emoji / 特殊符号 |
| 双缓冲区 | 主缓冲区 + 备用缓冲区（vim 等全屏应用切换） | 与 xterm 行为一致 |
| 滚动缓冲区 | 环形缓冲区（RingBuffer），上限 32767 行 | 固定内存上限，避免无限增长 |
| PTY 尺寸同步 | 监听 GPUI 窗口 resize 事件 → `ioctl(TIOCSWINSZ)` | 确保远程应用感知窗口变化 |

#### 3.1.5 终端数据流（前后端分离）

```
[后端] PTY 输出字节流
    │
    ▼
[后端] VTParser 解析转义序列
    │
    ├── 字符输出 → [后端] TerminalBuffer 写入字符网格
    ├── CSI/OSC  → [后端] 修改光标位置/颜色/标题等状态
    └── 特殊序列 → [后端] 切换缓冲区/滚动区域等
    │
    ▼
[后端] TerminalBuffer 状态变更 → 发布 TerminalOutput Event
    │
    ▼
[前端] TerminalViewModel 接收 Event → 更新本地状态
    │
    ▼
[前端] TerminalRenderer 遍历可见行
    │
    ├── 构建 TextRun 批次（相同样式连续字符合并）
    ├── 查询字形图集（缺失则异步加载）
    └── 提交 GPU 绘制命令
    │
    ▼
GPUI 合成帧 → 窗口显示
```

#### 3.1.6 支持的终端类型

| 终端类型 | 说明 | 实现优先级 |
|----------|------|------------|
| xterm-256color | 默认，256 色支持 | P0 |
| vt100 | 基础 VT100 | P0 |
| vt220 | 增加 DEC 私有序列 | P0 |
| vt320 | 增加 ReGIS/SIXEL | P2 |
| linux | Linux 控制台模拟 | P1 |
| scoansi | SCO ANSI | P2 |
| ansi | 标准 ANSI | P0 |

---

### 3.2 SSH / 协议连接模块 (`protocol-ssh`)

#### 3.2.1 模块职责

- 实现多种远程协议的连接建立、认证、数据通道管理
- 统一抽象为 `Connection` trait，上层无需感知协议差异
- 管理连接生命周期（连接、重连、断开、超时）

#### 3.2.2 协议支持矩阵

| 协议 | 实现方式 | 功能范围 | 优先级 |
|------|----------|----------|--------|
| SSH2 | `russh` crate | 交互式 Shell、命令执行、端口转发、SFTP 子系统 | P0 |
| SSH1 | 不实现 | 已过时，存在安全隐患，建议用户升级 | 不实现 |
| Telnet | 自实现（Tokio TCP + RFC 854 解析） | 交互式 Shell、选项协商 | P1 |
| Rlogin | 自实现（TCP + RFC 1258） | 交互式 Shell | P2 |
| Serial | `serialport` crate | 串口通信、波特率/数据位/停止位配置 | P1 |
| RDP | `ironrdp` crate | 远程桌面连接 | P2 |
| 本地 Shell | `portable-pty` | Windows CMD/PowerShell、Unix shell | P1 |

#### 3.2.3 SSH 连接流程

```
用户发起连接（SessionConfig）
    │
    ▼
TCP 连接建立（支持代理：HTTP/SOCKS）
    │
    ▼
SSH 握手
    ├── 版本交换 (SSH-2.0-RShell_x.x)
    ├── 密钥交换 (KEX: curve25519-sha256 / ecdh-sha2-nistp256 / ...)
    ├── 服务器主机密钥验证
    └── 加密算法协商
    │
    ▼
用户认证
    ├── 密码认证 (password)
    ├── 公钥认证 (RSA/DSA/ECDSA/ED25519)
    ├── 键盘交互 (keyboard-interactive)
    ├── GSSAPI/Kerberos
    └── PKCS#11 硬件令牌
    │
    ├── 认证失败 → 重试 / 提示用户
    │
    ▼
通道建立
    ├── Session Channel → PTY 请求 → Shell 启动
    ├── 端口转发通道（本地/远程/动态）
    ├── X11 转发通道
    └── SFTP 子系统通道
    │
    ▼
连接就绪，数据双向传输
```

#### 3.2.4 连接重连策略

```rust
// 重连配置
struct ReconnectPolicy {
    enabled: bool,
    max_retries: u32,          // 0 = 无限重试
    initial_delay_ms: u64,     // 初始延迟 1000ms
    max_delay_ms: u64,         // 最大延迟 30000ms
    backoff_multiplier: f64,   // 指数退避因子 2.0
}

// 重连状态机
enum ReconnectState {
    Connected,
    Disconnected { reason: DisconnectReason, since: Instant },
    Reconnecting { attempt: u32, next_try: Instant },
}
```

#### 3.2.5 跳转主机（Jump Host）

支持多级跳转，每级独立认证：

```
本地 → Jump Host A → Jump Host B → Target Server
       (SSH proxy)   (SSH proxy)    (最终目标)
```

实现方式：在 SSH 通道内嵌套建立新的 SSH 连接（SSH 协议级隧道，无需目标服务器运行 shell）。

#### 3.2.6 Telnet 协议实现

```rust
// Telnet 选项协商 (RFC 854)
enum TelnetCommand {
    SE,    // 子协商结束
    NOP,   // 无操作
    DM,    // 数据标记
    BRK,   // 中断
    IP,    // 中断进程
    AO,    // 中止输出
    AYT,   // 你在那里
    EC,    // 擦除字符
    EL,    // 擦除行
    GA,    // 继续
    SB,    // 子协商开始
    WILL,  // 愿意
    WONT,  // 不愿意
    DO,    // 要求对方
    DONT,  // 拒绝对方
}

// 支持的 Telnet 选项
enum TelnetOption {
    Echo,           // RFC 857
    SuppressGoAhead, // RFC 858
    TerminalType,   // RFC 1091
    WindowSize,     // RFC 1073
    Naocrd,         // 输出CR处理
    Linemode,       // RFC 1184
}
```

---

### 3.3 会话管理模块

#### 3.3.1 前后端职责划分

| 职责 | 前端 (`ui-session`) | 后端 (`core-session`) |
|------|---------------------|----------------------|
| 会话树 CRUD | ✘ | ✔ |
| 属性继承解析 | ✘ | ✔ |
| 认证配置管理 | ✘ | ✔ |
| 会话持久化 | ✘ | ✔ |
| 会话树 UI 展示 | ✔ | ✘ |
| 右键菜单/拖放排序 | ✔ | ✘ |
| 搜索/过滤会话 | ✔ (UI 过滤) | ✔ (数据查询) |
| 展开/折叠状态 | ✔ (本地 UI 状态) | ✘ |
| 选中状态 | ✔ (本地 UI 状态) | ✘ |

#### 3.3.2 会话树结构

```
SessionRoot
├── 📁 生产环境
│   ├── 📁 Web 服务器
│   │   ├── 🔗 web-prod-01 (SSH)
│   │   ├── 🔗 web-prod-02 (SSH)
│   │   └── 🔗 web-prod-03 (SSH)
│   └── 📁 数据库
│       ├── 🔗 db-master (SSH)
│       └── 🔗 db-slave-01 (SSH)
├── 📁 测试环境
│   ├── 🔗 test-server-01 (SSH)
│   └── 🔗 test-server-02 (Telnet)
├── 📁 IoT 设备
│   └── 🔗 sensor-gateway (Serial)
└── 🔗 快速连接 (未保存)
```

#### 3.3.3 模块职责（后端）

- 管理会话树（文件夹 → 子文件夹 → 会话）的 CRUD
- 实现属性继承机制（文件夹属性 → 子会话继承）
- 管理认证配置文件（独立存储，可复用于多个会话）
- 会话持久化（文件存储 + 加密）
- 提供快速连接 / 最近会话 / 书签功能

#### 3.3.4 属性继承机制

```rust
// 会话属性（可继承字段）
struct InheritableProperties {
    // 连接属性
    port: Option<u16>,
    protocol: Option<Protocol>,
    jump_host: Option<JumpHostConfig>,
    
    // 终端属性
    terminal_type: Option<String>,
    encoding: Option<Encoding>,
    scrollback_lines: Option<u32>,
    
    // 外观属性
    color_scheme: Option<String>,
    font_family: Option<String>,
    font_size: Option<f32>,
    
    // 安全属性
    auth_profile_id: Option<Uuid>,
    keep_alive_interval: Option<Duration>,
    
    // 高级属性
    reconnect_policy: Option<ReconnectPolicy>,
    proxy_config: Option<ProxyConfig>,
}

// 属性解析顺序：会话自身 → 父文件夹 → 祖父文件夹 → ... → 全局默认
fn resolve_property<T>(&self, getter: fn(&InheritableProperties) -> Option<T>) -> Option<T> {
    // 从自身开始向上遍历文件夹链
    let mut current = Some(self);
    while let Some(node) = current {
        if let Some(val) = getter(&node.properties) {
            return Some(val);
        }
        current = node.parent;
    }
    None  // 使用全局默认值
}
```

#### 3.3.5 认证配置文件

```rust
// 独立存储的认证配置，可复用于多个会话
struct AuthProfile {
    id: Uuid,
    name: String,
    method: AuthMethod,
    // 敏感数据使用主密码加密存储
    credentials: EncryptedCredentials,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

enum AuthMethod {
    Password,
    PublicKey { key_path: PathBuf, passphrase: Option<EncryptedSecret> },
    KeyboardInteractive,
    Gssapi,
    Pkcs11 { token_label: String, key_id: Vec<u8> },
}
```

#### 3.3.6 会话持久化格式

```toml
# sessions/uuid.toml
[session]
id = "550e8400-e29b-41d4-a716-446655440000"
name = "web-prod-01"
host = "192.168.1.100"
protocol = "ssh"

[session.properties]
port = 22
terminal_type = "xterm-256color"
encoding = "utf-8"
color_scheme = "monokai"
auth_profile_id = "660e8400-e29b-41d4-a716-446655440001"

[session.jump_host]
host = "gateway.example.com"
port = 22
auth_profile_id = "660e8400-e29b-41d4-a716-446655440002"
```

---

### 3.4 SFTP / 文件传输模块

#### 3.4.1 前后端职责划分

| 职责 | 前端 (`ui-filemgr`) | 后端 (`core-transfer`) |
|------|---------------------|----------------------|
| SFTP 协议操作 | ✘ | ✔ |
| 远程目录列表获取 | ✘ | ✔ |
| 传输队列管理 | ✘ | ✔ |
| 断点续传逻辑 | ✘ | ✔ |
| 文件夹同步算法 | ✘ | ✔ |
| 双窗格 UI 布局 | ✔ | ✘ |
| 文件列表虚拟化渲染 | ✔ | ✘ |
| 拖放交互 | ✔ | ✘ |
| 传输进度条展示 | ✔ | ✘ |
| 本地/远程路径导航 | ✔ (发 Command) | ✔ (返回结果) |
| 文件过滤/排序显示 | ✔ (UI 层过滤) | ✔ (服务端过滤) |
| 书签 UI | ✔ (本地 UI 状态) | ✘ |

#### 3.4.2 模块职责（后端）

- 远程文件浏览（目录列表、文件属性、权限）
- 文件上传 / 下载 / 删除 / 重命名
- 传输队列管理与断点续传
- 文件夹同步（比较 + 同步）
- 同步浏览（双服务器导航联动）
- 传输调度（定时传输任务）
- **不包含任何 UI 布局或渲染逻辑**，通过 Event 向前端推送目录数据和传输进度

#### 3.4.3 文件管理器架构（前后端分离）

```
前端 (ui-filemgr):                          后端 (core-transfer):
┌───────────────────────────────────────┐     ┌───────────────────────────────────────┐
│  本地文件浏览器 UI  │  远程文件浏览器 UI │     │                                       │
│  ┌───────────────┐ │ ┌───────────────┐│     │  ┌─────────────────────────────────┐ │
│  │ 路径栏 / 书签  │ │ │ 路径栏 / 书签  ││     │  │  SftpClient (协议层)             │ │
│  ├───────────────┤ │ ├───────────────┤│     │  │  - read_dir / stat / open_file   │ │
│  │ 文件列表(虚拟化)│ │ │ 文件列表(虚拟化)││     │  └───────────────┬─────────────────┘ │
│  └───────────────┘ │ └───────────────┘│     │                  │                   │
├───────────────────────────────────────┤     │  ┌───────────────┴─────────────────┐ │
│  传输队列面板 UI                       │     │  │  TransferQueue (传输队列)        │ │
│  - 进度条  - 速度  - 暂停/恢复       │     │  │  - enqueue / pause / resume      │ │
└───────────────────────────────────────┘     │  └───────────────┬─────────────────┘ │
                                              │                  │                   │
┌───────────────────────────────────────┐     │  ┌───────────────┴─────────────────┐ │
│  FileMgrViewModel                     │     │  │  SyncEngine (同步引擎)           │ │
│  - local_path / remote_path           │     │  │  - compare / mirror / update     │ │
│  - selected_files (本地 UI)           │◀──────│  └─────────────────────────────────┘ │
│  - scroll_offset (本地 UI)            │Event│                                       │
│  - transfer_progress (后端状态投影)  │     └───────────────────────────────────────┘
└───────────────────────────────────────┘
          │ Command (BrowseRemoteDir / EnqueueUpload / ...)
          ▼
```

#### 3.4.4 传输引擎设计（后端）

```rust
// 传输任务
struct TransferTask {
    id: Uuid,
    direction: TransferDirection,  // Upload / Download
    source_path: PathBuf,
    dest_path: RemotePath,
    file_size: u64,
    transferred: u64,
    state: TransferState,
    priority: u32,
    resume_from: u64,  // 断点续传偏移
}

enum TransferState {
    Pending,
    InProgress { speed_bps: f64, started_at: Instant },
    Paused { offset: u64 },
    Completed { duration: Duration },
    Failed { error: TransferError, retry_count: u32 },
    Cancelled,
}

// 传输队列管理器
struct TransferQueue {
    tasks: Vec<TransferTask>,
    max_concurrent: usize,       // 最大并发传输数
    buffer_size: usize,          // 每任务缓冲区大小 (默认 32KB)
    speed_limit: Option<u64>,    // 全局速度限制 (bytes/s)
}

impl TransferQueue {
    /// 添加传输任务
    fn enqueue(&mut self, task: TransferTask) -> Uuid;
    /// 暂停指定任务
    fn pause(&mut self, task_id: Uuid) -> Result<()>;
    /// 恢复传输（支持断点续传）
    fn resume(&mut self, task_id: Uuid) -> Result<()>;
    /// 取消任务
    fn cancel(&mut self, task_id: Uuid) -> Result<()>;
    /// 获取实时传输状态
    fn status(&self) -> TransferQueueStatus;
}
```

#### 3.4.5 文件夹同步算法（后端）

```rust
// 同步模式
enum SyncMode {
    Mirror,       // 镜像：目标与源完全一致（删除多余文件）
    Update,       // 更新：仅传输较新文件
    Bidirectional, // 双向：两边都保留最新
}

// 同步比较结果
struct SyncDiff {
    only_local: Vec<FileEntry>,     // 仅本地存在
    only_remote: Vec<FileEntry>,    // 仅远程存在
    modified_local: Vec<FileEntry>, // 本地较新
    modified_remote: Vec<FileEntry>,// 远程较新
    identical: Vec<FileEntry>,      // 完全一致
}

// 同步执行
async fn sync_folders(
    local: &LocalFileSystem,
    remote: &RemoteFileSystem,
    mode: SyncMode,
    filter: FileFilter,
) -> SyncResult;
```

#### 3.4.6 断点续传实现（后端）

```rust
// 断点续传流程
async fn resume_transfer(task: &TransferTask, sftp: &SftpClient) -> Result<()> {
    // 1. 获取远程文件当前大小（已传输部分）
    let remote_size = sftp.stat(&task.dest_path).await?.size;
    
    // 2. 校验已传输部分完整性（可选，基于文件大小+修改时间）
    let local_meta = tokio::fs::metadata(&task.source_path).await?;
    
    // 3. 从断点继续传输
    let mut local_file = File::open(&task.source_path).await?;
    local_file.seek(SeekFrom::Start(remote_size)).await?;
    
    let mut remote_file = sftp.open_with_flags(
        &task.dest_path,
        OpenFlags::WRITE | OpenFlags::APPEND,
    ).await?;
    
    // 4. 继续传输剩余部分
    let mut buffer = vec![0u8; BUFFER_SIZE];
    loop {
        let n = local_file.read(&mut buffer).await?;
        if n == 0 { break; }
        remote_file.write_all(&buffer[..n]).await?;
    }
    
    Ok(())
}
```

---

### 3.5 安全模块

#### 3.5.1 前后端职责划分

| 职责 | 前端 (`ui-settings`) | 后端 (`core-security`) |
|------|---------------------|----------------------|
| 密钥生成/导入/导出 | ✘ | ✔ |
| 主密码加密/验证 | ✘ | ✔ |
| 隧道/端口转发管理 | ✘ | ✔ |
| 主机密钥检测 | ✘ | ✔ |
| 密钥列表 UI | ✔ | ✘ |
| 主密码输入对话框 | ✔ | ✘ |
| 隧道状态监控面板 | ✔ | ✘ |
| 主机密钥警告对话框 | ✔ | ✘ |

#### 3.5.2 密钥管理

```rust
// 支持的密钥类型
enum KeyType {
    RSA { bits: u32 },          // 2048 / 4096
    DSA { bits: u32 },          // 1024 (legacy)
    ECDSA { curve: ECurve },    // P256 / P384 / P521
    ED25519,
}

enum ECurve { P256, P384, P521 }

// 密钥管理器
struct KeyManager {
    keys_dir: PathBuf,
    keys: Vec<SshKey>,
}

struct SshKey {
    id: Uuid,
    name: String,
    key_type: KeyType,
    fingerprint_sha256: String,  // SHA256:xxxxx
    public_key: PublicKey,
    // 私钥加密存储（使用主密码或 passphrase）
    private_key_encrypted: Vec<u8>,
    comment: String,
    created_at: DateTime<Utc>,
}

impl KeyManager {
    /// 生成新密钥对
    fn generate_key(name: &str, key_type: KeyType, passphrase: Option<&str>) -> Result<SshKey>;
    /// 导入私钥文件（OpenSSH / PuTTY PPk 格式）
    fn import_private_key(path: &Path, passphrase: Option<&str>) -> Result<SshKey>;
    /// 导出公钥（OpenSSH 格式 / RFC4716 格式）
    fn export_public_key(key_id: Uuid, format: KeyExportFormat) -> Result<String>;
    /// 删除密钥
    fn delete_key(key_id: Uuid) -> Result<()>;
    /// 获取密钥指纹
    fn fingerprint(key_id: Uuid) -> Result<String>;
}
```

#### 3.5.3 模块职责（后端）

- SSH 密钥生成、导入、导出、管理
- 主密码管理（加密存储会话密码）
- SSH 隧道 / 端口转发管理
- SOCKS 代理（动态端口转发）
- X11 转发支持

#### 3.5.4 主密码系统

```rust
// 主密码管理器
struct MasterPassword {
    // 主密码不直接存储，而是用其派生密钥加密验证令牌
    // 使用 Argon2id 派生密钥
    _private: (),
}

impl MasterPassword {
    /// 设置主密码（首次）
    fn setup(password: &str) -> Result<()>;
    /// 验证主密码
    fn verify(password: &str) -> Result<MasterPasswordGuard>;
    /// 修改主密码（需重新加密所有受保护数据）
    fn change_password(old: &str, new: &str) -> Result<()>;
}

// 加密存储流程：
// 1. 用户输入主密码 → Argon2id(salt, password) → 256-bit 派生密钥
// 2. 派生密钥 + AES-256-GCM 加密会话密码 / 私钥
// 3. 加密数据持久化到配置文件
// 4. 验证令牌存储在内存中（MasterPasswordGuard 生命周期内有效）
```

#### 3.5.5 隧道 / 端口转发管理

```rust
// 端口转发规则
enum PortForwardRule {
    Local {
        bind_addr: SocketAddr,    // 本地监听地址
        remote_host: String,      // 目标远程主机
        remote_port: u16,         // 目标远程端口
    },
    Remote {
        bind_addr: SocketAddr,    // 远程监听地址
        local_host: String,       // 目标本地主机
        local_port: u16,          // 目标本地端口
    },
    Dynamic {
        bind_addr: SocketAddr,    // 本地 SOCKS 监听地址
    },
}

// 隧道管理器
struct TunnelManager {
    active_tunnels: HashMap<Uuid, ActiveTunnel>,
}

struct ActiveTunnel {
    id: Uuid,
    session_id: Uuid,
    rule: PortForwardRule,
    state: TunnelState,
    bytes_transferred: u64,
    connections_count: u32,
}

enum TunnelState {
    Active,
    Suspended,
    Error(String),
}

impl TunnelManager {
    /// 即时创建隧道（运行中会话）
    fn create_tunnel(session_id: Uuid, rule: PortForwardRule) -> Result<Uuid>;
    /// 关闭隧道
    fn close_tunnel(tunnel_id: Uuid) -> Result<()>;
    /// 列出所有活跃隧道
    fn list_tunnels(&self) -> Vec<&ActiveTunnel>;
}
```

#### 3.5.6 主机密钥管理

```rust
// 主机密钥存储
struct HostKeyEntry {
    host: String,          // 主机名或 IP
    port: u16,
    key_type: KeyType,
    fingerprint: String,
    key_data: Vec<u8>,
    trust_level: TrustLevel,
    first_seen: DateTime<Utc>,
    last_seen: DateTime<Utc>,
}

enum TrustLevel {
    Trusted,       // 用户明确信任
    AutoAccepted,  // 首次连接自动接受
    Revoked,       // 已撤销
}

// 主机密钥变更检测（MITM 防护）
fn verify_host_key(host: &str, port: u16, server_key: &PublicKey) -> HostKeyResult {
    // 对比 known_hosts 中存储的密钥
    // 不匹配 → 警告用户可能遭受中间人攻击
}
```

---

### 3.6 UI 框架模块 (`ui-*` 系列 crate)

> 本模块纯前端，不涉及后端。所有业务逻辑通过 Command 发送到后端，通过 Event 从后端接收状态变化。

#### 3.6.1 模块职责（纯前端）

- 基于 GPUI + gpui-component 构建整体应用 UI
- 实现 Dock / Tiles 可拖拽布局系统
- 管理主题与配色方案
- 提供选项卡、分屏、面板管理

#### 3.6.2 应用布局结构

```
┌─────────────────────────────────────────────────────────────────────┐
│  菜单栏 (Menu Bar)                                                  │
│  文件 | 编辑 | 查看 | 会话 | 工具 | 帮助                            │
├─────────────────────────────────────────────────────────────────────┤
│  工具栏 (Toolbar) [可自定义按钮]                                     │
│  [🔗连接▼] [📁文件] [⚡快速命令▼] [🔧隧道] [⚙设置]               │
├────────┬────────────────────────────────────────────────┬───────────┤
│        │              主工作区 (Dock Center)             │           │
│  会话  │  ┌─────────────────┬─────────────────────────┐ │   隧道    │
│  管理  │  │  Tab: web-01    │  Tab: db-master         │ │   面板    │
│  器    │  │  ┌─────────────┐│  ┌─────────────────────┐│ │  (可停靠) │
│        │  │  │             ││  │                     ││ │           │
│ (左侧  │  │  │  终端视图    ││  │   终端视图          ││ │  本地端口 │
│  面板) │  │  │             ││  │                     ││ │  远程端口 │
│        │  │  │             ││  │                     ││ │  动态SOCKS│
│ 📁生产  │  │  └─────────────┘│  └─────────────────────┘│ │           │
│ 📁测试  │  └─────────────────┴─────────────────────────┘ │           │
│ 📁IoT  │  ┌───────────────────────────────────────────┐ │           │
│        │  │  Tab: SFTP - web-01                       │ │           │
│        │  │  ┌──────────────────┬──────────────────┐  │ │           │
│        │  │  │ 本地文件          │ 远程文件          │  │ │           │
│        │  │  │ /home/user/      │ /var/www/         │  │ │           │
│        │  │  │ 📄 index.html    │ 📄 index.html     │  │ │           │
│        │  │  │ 📄 style.css     │ 📄 style.css      │  │ │           │
│        │  │  └──────────────────┴──────────────────┘  │ │           │
│        │  └───────────────────────────────────────────┘ │           │
├────────┴────────────────────────────────────────────────┴───────────┤
│  状态栏 (Status Bar)                                                │
│  🔒 已连接 | web-01:22 | SSH2 | ↑ 1.2KB/s ↓ 3.4KB/s | UTF-8      │
└─────────────────────────────────────────────────────────────────────┘
```

#### 3.6.3 GPUI 组件使用映射

| UI 区域 | GPUI Component 组件 | 说明 |
|---------|---------------------|------|
| 菜单栏 | `Menu` + `MenuItem` | 标准应用菜单 |
| 工具栏 | `Toolbar` + `Button` + `Dropdown` | 可自定义按钮 |
| 会话管理器 | `Sidebar` + `TreeView` | 树形会话列表 |
| 选项卡 | `Tabs` + `TabBar` | 多标签管理 |
| 终端视图 | 自定义 `Element` | GPU 渲染终端内容 |
| 文件列表 | `Table`（虚拟化） | 大目录流畅滚动 |
| 传输队列 | `Table` + `Progress` | 实时传输进度 |
| 隧道面板 | `Table` + `Badge` | 隧道状态展示 |
| 设置对话框 | `Modal` + `Tabs` + `Form` | 多页设置 |
| 快速命令 | `Dropdown` + `Button` | 命令按钮组 |
| 撰写窗格 | `TextArea` + `Modal` | 多行编辑发送 |
| 通知 | `Notification` + `Toast` | 连接状态通知 |
| 右键菜单 | `ContextMenu` | 文件/终端右键操作 |
| 搜索栏 | `TextInput` + `Popover` | 终端/文件搜索 |
| 状态栏 | 自定义 `Element` | 连接信息展示 |

#### 3.6.4 主题与配色方案

```rust
// 应用主题（控制 UI 外观）
struct AppTheme {
    name: String,
    mode: ThemeMode,  // Light / Dark / System
    colors: ThemeColors,
}

struct ThemeColors {
    // UI 颜色
    background: Rgba,
    foreground: Rgba,
    accent: Rgba,
    border: Rgba,
    sidebar_bg: Rgba,
    toolbar_bg: Rgba,
    statusbar_bg: Rgba,
    // ... 更多 UI 颜色
}

// 终端配色方案（控制终端显示颜色）
struct TerminalColorScheme {
    name: String,
    // 标准 16 色 (ANSI 0-15)
    ansi_colors: [Rgba; 16],
    // 256 色扩展
    extended_colors: HashMap<u8, Rgba>,
    // 默认前景/背景
    default_fg: Rgba,
    default_bg: Rgba,
    // 光标颜色
    cursor_fg: Rgba,
    cursor_bg: Rgba,
    // 选区颜色
    selection_fg: Rgba,
    selection_bg: Rgba,
}

// 内置配色方案
const BUILT_IN_SCHEMES: &[&str] = &[
    "Monokai", "Solarized Dark", "Solarized Light",
    "Dracula", "Nord", "One Dark", "Tomorrow Night",
    "Gruvbox Dark", "Gruvbox Light", "Zenburn",
    "Default Dark", "Default Light",
];
```

#### 3.6.5 选项卡管理

```rust
// 选项卡模型
struct TabItem {
    id: Uuid,
    title: String,
    icon: TabIcon,
    content: TabContent,
    session_id: Option<Uuid>,
    is_modified: bool,
    is_pinned: bool,
}

enum TabContent {
    Terminal(TerminalViewId),
    FileTransfer(FileTransferViewId),
    LocalShell(LocalShellViewId),
    Settings(SettingsViewId),
}

// 标签组（支持分组）
struct TabGroup {
    id: Uuid,
    name: String,
    tabs: Vec<TabItem>,
    active_tab: Option<Uuid>,
}

// 标签管理器
struct TabManager {
    groups: Vec<TabGroup>,
    active_group: Option<Uuid>,
    max_tabs: usize,
}
```

#### 3.6.6 分屏布局

```rust
// 分屏方向
enum SplitDirection {
    Horizontal,  // 左右分割
    Vertical,    // 上下分割
}

// 分屏布局
struct SplitLayout {
    direction: SplitDirection,
    ratio: f32,  // 分割比例 0.0 ~ 1.0
    first: Box<LayoutNode>,
    second: Box<LayoutNode>,
}

enum LayoutNode {
    Tab(TabItem),
    Split(SplitLayout),
    Empty,
}
```

---

### 3.7 脚本与自动化模块

#### 3.7.1 前后端职责划分

| 职责 | 前端 (`ui-common`) | 后端 (`core-script`) |
|------|---------------------|----------------------|
| 快速命令定义/存储 | ✘ | ✔ |
| 触发器引擎 | ✘ | ✔ |
| Rhai 脚本执行 | ✘ | ✔ |
| 脚本录制 | ✘ | ✔ |
| 撰写窗格文本编辑 | ✔ | ✘ |
| 快速命令按钮展示 | ✔ | ✘ |
| 触发器配置 UI | ✔ | ✘ |
| 脚本编辑器 UI | ✔ | ✘ |

#### 3.7.2 模块职责（后端）

- 快速命令管理（常用命令按钮化）
- 触发器引擎（基于终端输出自动执行动作）
- 脚本引擎（嵌入式脚本语言支持）
- 脚本录制（基于终端 I/O 自动生成脚本）
- 撰写窗格（多行文本批量发送）

#### 3.7.2 快速命令

```rust
// 快速命令定义
struct QuickCommand {
    id: Uuid,
    name: String,
    command: String,
    send_enter: bool,       // 是否自动发送回车
    description: String,
    // 作用范围
    scope: QuickCommandScope,
    // 快捷键（可选）
    hotkey: Option<KeyBinding>,
    // 分组
    group: Option<String>,
}

enum QuickCommandScope {
    CurrentSession,         // 仅当前会话
    AllSessions,            // 所有会话
    SelectedSessions,       // 选中的会话
}

// 快速命令管理器
struct QuickCommandManager {
    commands: Vec<QuickCommand>,
    // 在工具栏显示为可点击按钮
}
```

#### 3.7.3 触发器引擎

```rust
// 触发器定义
struct Trigger {
    id: Uuid,
    name: String,
    enabled: bool,
    // 匹配条件
    condition: TriggerCondition,
    // 执行动作
    action: TriggerAction,
}

enum TriggerCondition {
    RegexAppear(String),     // 终端出现匹配正则的文本
    RegexDisappear(String),  // 终端中匹配文本消失
    ExactMatch(String),      // 精确匹配
}

enum TriggerAction {
    SendText(String),        // 发送文本到终端
    RunCommand(String),      // 执行本地命令
    ShowNotification(String),// 显示通知
    PlaySound(String),       // 播放声音
    Disconnect,              // 断开连接
    LogToFile(PathBuf),      // 记录到文件
}

// 触发器引擎
struct TriggerEngine {
    triggers: Vec<Trigger>,
    // 在每次终端输出时检查所有活跃触发器
}

impl TriggerEngine {
    fn on_terminal_output(&mut self, text: &str, session_id: Uuid) -> Vec<TriggerAction>;
}
```

#### 3.7.4 脚本引擎

```rust
// 使用 Rhai 作为嵌入式脚本语言
// Rhai 语法类似 JavaScript/VB，学习成本低

// 脚本可访问的 API
// rshell.connect("session_name")           - 连接会话
// rshell.disconnect("session_name")        - 断开会话
// rshell.send("session_name", "command\n") - 发送文本
// rshell.expect("pattern", timeout_ms)     - 等待匹配输出
// rshell.sleep(ms)                         - 等待
// rshell.log("message")                    - 记录日志
// rshell.get_clipboard()                   - 获取剪贴板
// rshell.set_clipboard("text")             - 设置剪贴板

// 脚本引擎
struct ScriptEngine {
    runtime: rhai::Engine,
}

impl ScriptEngine {
    /// 执行脚本文件
    fn execute_file(&self, path: &Path, context: ScriptContext) -> Result<ScriptResult>;
    /// 执行脚本字符串
    fn execute_string(&self, code: &str, context: ScriptContext) -> Result<ScriptResult>;
    /// 录制脚本（基于终端 I/O 生成）
    fn generate_from_recording(&self, recording: &TerminalRecording) -> String;
}

// 脚本上下文
struct ScriptContext {
    session_id: Uuid,
    target_sessions: Vec<Uuid>,  // 多会话脚本目标
    variables: HashMap<String, String>,
}
```

#### 3.7.5 撰写窗格

```rust
// 撰写窗格模型
struct ComposePane {
    content: String,
    target: ComposeTarget,
    history: Vec<String>,  // 发送历史
}

enum ComposeTarget {
    CurrentSession,
    AllSessions,
    SelectedSessions(Vec<Uuid>),
}

impl ComposePane {
    /// 发送内容到目标会话
    fn send(&self, sessions: &SessionManager) -> Result<()>;
    /// 追加换行后发送
    fn send_with_newline(&self, sessions: &SessionManager) -> Result<()>;
}
```

---

### 3.8 插件化扩展模块 (`rshell-plugin-sdk`)

#### 3.8.1 设计目标

RShell 采用插件化架构，允许第三方开发者在不修改核心代码的前提下扩展以下能力：

- **协议插件**：新增自定义连接协议（如 MQTT、WebSocket Shell 等）
- **主题插件**：新增终端配色方案和应用主题
- **工具插件**：在侧边栏/工具栏添加自定义工具面板
- **命令插件**：注册自定义命令（快速命令、脚本函数）
- **文件操作插件**：扩展文件管理器右键菜单和操作
- **触发器动作插件**：新增触发器动作类型

#### 3.8.2 插件架构概览

```
┌──────────────────────────────────────────────────────────────┐
│                        RShell 主程序                          │
│  ┌────────────────────────────────────────────────────────┐ │
│  │                 Plugin Host (插件宿主)                   │ │
│  │  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐ │ │
│  │  │ PluginLoader │ │PluginRegistry│ │PluginSandbox │ │ │
│  │  └──────┬───────┘ └──────┬───────┘ └──────────────┘ │ │
│  └─────────┼────────────────┼─────────────────────────────┘ │
│            │                │                               │
│  ┌─────────┴────────────────┴─────────────────────────────┐ │
│  │              Plugin API (rshell-plugin-sdk)             │ │
│  │  - ProtocolExtension    - ThemeExtension               │ │
│  │  - ToolPanelExtension   - CommandExtension             │ │
│  │  - FileActionExtension  - TriggerActionExtension       │ │
│  └─────────┬────────────────┬─────────────────────────────┘ │
└────────────┼────────────────┼───────────────────────────────┘
             │                │
   ┌─────────┴──────┐  ┌─────┴────────┐  ┌──────────────┐
   │ 内置插件         │  │ WASM 插件     │  │ 动态库插件    │
   │ (编译进主程序)  │  │ (沙箱执行)    │  │ (原生性能)    │
   └────────────────┘  └──────────────┘  └──────────────┘
```

#### 3.8.3 插件类型与扩展点

```rust
/// 插件清单文件（plugin.toml）
struct PluginManifest {
    name: String,
    version: String,
    author: String,
    description: String,
    // 插件类型
    plugin_type: PluginType,
    // 声明的扩展点
    extensions: Vec<ExtensionPoint>,
    // 所需权限
    permissions: Vec<PluginPermission>,
    // 兼容性
    min_rshell_version: String,
}

enum PluginType {
    Builtin,        // 编译进主程序
    Wasm,           // WASM 沙箱插件
    DynamicLib,     // 动态库插件 (.dll/.so/.dylib)
}

/// 扩展点声明
enum ExtensionPoint {
    Protocol { scheme: String },           // 新增协议
    Theme { name: String },                // 新增主题
    ColorScheme { name: String },          // 新增终端配色
    ToolPanel { id: String, title: String },// 新增工具面板
    QuickCommand { commands: Vec<String> },// 新增快速命令
    FileAction { name: String },           // 新增文件操作
    TriggerAction { name: String },        // 新增触发器动作
    StatusBar { position: StatusBarPos },  // 新增状态栏组件
}

/// 插件权限
enum PluginPermission {
    NetworkAccess,        // 网络访问
    FileSystemAccess,     // 本地文件系统访问
    SessionAccess,        // 会话数据访问
    TerminalAccess,       // 终端输入输出访问
    ClipboardAccess,      // 剪贴板访问
    ProcessExecution,     // 本地进程执行
}
```

#### 3.8.4 Plugin API（前后端分离）

插件 API 同样遵循前后端分离原则，插件只能与后端交互，不能直接操作 UI：

```rust
/// 插件 Trait（所有插件实现此接口）
trait RShellPlugin: Send + Sync {
    /// 插件初始化
    fn init(&mut self, ctx: PluginContext) -> Result<()>;
    /// 插件卸载
    fn shutdown(&mut self) -> Result<()>;
    /// 获取插件清单
    fn manifest(&self) -> &PluginManifest;
}

/// 插件上下文（插件通过此与宿主交互）
struct PluginContext {
    /// 事件总线（插件可订阅/发布事件）
    event_bus: PluginEventBus,
    /// 命令注册器
    command_registry: PluginCommandRegistry,
    /// 配置存储（插件私有配置）
    config: PluginConfigStore,
    /// 日志
    logger: PluginLogger,
}

/// 协议扩展插件
trait ProtocolPlugin: RShellPlugin {
    /// 创建连接实例
    fn create_connection(&self, config: &PluginConnectionConfig)
        -> Result<Box<dyn Connection>>;
    /// 获取协议配置 UI 描述（用于生成设置表单）
    fn config_schema(&self) -> PluginConfigSchema;
}

/// 主题扩展插件
trait ThemePlugin: RShellPlugin {
    /// 注册主题
    fn register_themes(&self) -> Vec<AppTheme>;
    /// 注册终端配色方案
    fn register_color_schemes(&self) -> Vec<TerminalColorScheme>;
}

/// 工具面板扩展插件（后端部分）
trait ToolPanelPlugin: RShellPlugin {
    /// 面板数据提供（后端逻辑）
    fn provide_data(&self, panel_id: &str) -> Result<PanelData>;
    /// 处理面板命令
    fn handle_command(&self, panel_id: &str, cmd: &str) -> Result<()>;
}

/// 文件操作扩展插件
trait FileActionPlugin: RShellPlugin {
    /// 获取文件操作列表（显示在右键菜单）
    fn get_actions(&self, file: &RemoteFileEntry) -> Vec<FileAction>;
    /// 执行文件操作
    fn execute_action(&self, action_id: &str, files: &[RemoteFileEntry])
        -> Result<FileActionResult>;
}
```

#### 3.8.5 WASM 插件沙箱

```rust
/// WASM 插件运行时
struct WasmPluginRuntime {
    engine: wasmtime::Engine,
    store: wasmtime::Store<PluginHostState>,
    instance: wasmtime::Instance,
}

/// WASM 插件可访问的宿主函数（受限 API）
/// 插件无法直接访问系统资源，必须通过宿主函数
impl WasmPluginRuntime {
    /// 注册宿主函数
    fn register_host_functions(&self, linker: &mut wasmtime::Linker<PluginHostState>) {
        // 事件相关
        linker.func("rshell_event_publish", ...);
        linker.func("rshell_event_subscribe", ...);
        // 配置相关
        linker.func("rshell_config_get", ...);
        linker.func("rshell_config_set", ...);
        // 日志
        linker.func("rshell_log", ...);
        // 网络（需权限）
        linker.func("rshell_http_get", ...);
        // 会话（需权限）
        linker.func("rshell_session_list", ...);
        linker.func("rshell_session_send", ...);
    }
}

// WASM 插件内存隔离：
// - 插件只能访问自己分配的内存
// - 无法直接访问文件系统/网络/进程
// - 所有系统交互必须通过宿主函数（受权限控制）
```

#### 3.8.6 插件生命周期

```
发现插件 (Discovery)
    │  扫描插件目录: ~/.rshell/plugins/
    │  解析 plugin.toml 清单
    ▼
验证插件 (Validation)
    │  检查兼容性 (min_rshell_version)
    │  验证签名 (可选)
    │  检查权限声明
    ▼
加载插件 (Loading)
    │  内置插件: 直接初始化
    │  WASM 插件: 加载 .wasm 文件 → 实例化
    │  动态库插件: dlopen → 查找入口函数
    ▼
初始化插件 (Initialization)
    │  调用 Plugin::init(ctx)
    │  注册扩展点
    │  订阅事件
    ▼
运行中 (Running)
    │  响应事件
    │  处理命令
    │  提供数据
    ▼
卸载插件 (Shutdown)
    │  调用 Plugin::shutdown()
    │  取消事件订阅
    │  释放资源
```

#### 3.8.7 插件配置存储

```rust
/// 插件私有配置存储（每个插件独立，互不干扰）
struct PluginConfigStore {
    plugin_id: String,
    config_dir: PathBuf,  // ~/.rshell/plugins/<plugin_id>/config/
}

impl PluginConfigStore {
    /// 读取配置
    fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>>;
    /// 写入配置
    fn set<T: Serialize>(&self, key: &str, value: &T) -> Result<()>;
    /// 删除配置
    fn remove(&self, key: &str) -> Result<()>;
}
```

#### 3.8.8 插件示例：自定义协议插件

```rust
// websocket_shell_plugin/src/lib.rs

use rshell_plugin_sdk::*;

struct WebSocketShellPlugin;

impl RShellPlugin for WebSocketShellPlugin {
    fn init(&mut self, ctx: PluginContext) -> Result<()> {
        ctx.logger.info("WebSocket Shell plugin initialized");
        Ok(())
    }
    fn shutdown(&mut self) -> Result<()> { Ok(()) }
    fn manifest(&self) -> &PluginManifest { &MANIFEST }
}

impl ProtocolPlugin for WebSocketShellPlugin {
    fn create_connection(&self, config: &PluginConnectionConfig)
        -> Result<Box<dyn Connection>>
    {
        // 建立 WebSocket 连接
        let ws_url = format!("ws://{}:{}/shell", config.host, config.port);
        let ws_conn = WebSocketConnection::connect(&ws_url).await?;
        Ok(Box::new(ws_conn))
    }

    fn config_schema(&self) -> PluginConfigSchema {
        PluginConfigSchema {
            fields: vec![
                ConfigField::text("ws_path", "WebSocket Path", "/shell"),
                ConfigField::select("subprotocol", "Subprotocol",
                    vec!["shell-v1", "shell-v2"]),
            ],
        }
    }
}

rshell_plugin_export!(WebSocketShellPlugin);
```

#### 3.8.9 插件示例：主题插件

```rust
// catppuccin_theme_plugin/src/lib.rs

use rshell_plugin_sdk::*;

struct CatppuccinThemePlugin;

impl RShellPlugin for CatppuccinThemePlugin {
    fn init(&mut self, _ctx: PluginContext) -> Result<()> { Ok(()) }
    fn shutdown(&mut self) -> Result<()> { Ok(()) }
    fn manifest(&self) -> &PluginManifest { &MANIFEST }
}

impl ThemePlugin for CatppuccinThemePlugin {
    fn register_themes(&self) -> Vec<AppTheme> {
        vec![
            AppTheme {
                name: "Catppuccin Mocha".into(),
                mode: ThemeMode::Dark,
                colors: ThemeColors {
                    background: rgba("#1e1e2e"),
                    foreground: rgba("#cdd6f4"),
                    accent: rgba("#cba6f7"),
                    // ...
                },
            },
            AppTheme {
                name: "Catppuccin Latte".into(),
                mode: ThemeMode::Light,
                colors: ThemeColors { /* ... */ },
            },
        ]
    }

    fn register_color_schemes(&self) -> Vec<TerminalColorScheme> {
        vec![
            TerminalColorScheme {
                name: "Catppuccin Mocha".into(),
                ansi_colors: [ /* 16 色 */ ],
                default_fg: rgba("#cdd6f4"),
                default_bg: rgba("#1e1e2e"),
                // ...
            },
        ]
    }
}

rshell_plugin_export!(CatppuccinThemePlugin);
```

#### 3.8.10 插件目录结构

```
~/.rshell/
├── plugins/
│   ├── catppuccin-theme/          # 主题插件
│   │   ├── plugin.toml            # 插件清单
│   │   ├── catppuccin.wasm        # WASM 插件二进制
│   │   └── config/                # 插件私有配置
│   │       └── settings.json
│   ├── mqtt-protocol/             # 协议插件
│   │   ├── plugin.toml
│   │   └── mqtt_protocol.so      # 动态库插件
│   └── docker-tools/              # 工具插件
│       ├── plugin.toml
│       └── docker_tools.wasm
├── plugin-cache/                  # 插件缓存
│   └── registry.json              # 已加载插件索引
└── marketplace/                   # 市场缓存
    └── index.json                 # 可用插件列表
```

#### 3.8.11 前后端分离在插件系统中的体现

| 组件 | 所属层 | 说明 |
|------|--------|------|
| `RShellPlugin` trait | 后端 (`rshell-plugin-sdk`) | 插件核心逻辑，不依赖 GPUI |
| `ProtocolPlugin` trait | 后端 | 协议扩展，实现 `Connection` trait |
| `ThemePlugin` trait | 后端 | 主题数据提供（纯数据，不含 UI） |
| `ToolPanelPlugin` trait | 后端 | 面板数据提供，前端通过 Event 获取数据 |
| 插件配置 UI | 前端 (`rshell-ui`) | 根据 `PluginConfigSchema` 动态生成表单 |
| 插件工具面板渲染 | 前端 (`rshell-ui`) | 根据 `PanelData` 渲染面板内容 |
| 插件菜单项 | 前端 (`rshell-ui`) | 根据插件注册的操作动态生成菜单 |

---

## 4. 核心数据结构设计

### 4.1 数据分层原则

```
后端 (rshell-core):                    前端 (rshell-ui):
┌─────────────────────────────┐          ┌─────────────────────────────┐
│  领域模型 (Domain Models)    │          │  视图模型 (ViewModels)       │
│  - SessionConfig             │          │  - TerminalViewModel         │
│  - ResolvedConnectionConfig  │          │  - FileMgrViewModel          │
│  - TransferTask              │          │  - SessionViewModel          │
│  - RemoteFileEntry           │          │  - TransferViewModel         │
│  - SshKey                    │          │  - TunnelViewModel           │
│  - ...                       │          │  - ...                       │
└─────────────────────────────┘          └─────────────────────────────┘
  纯业务数据，不依赖 GPUI            包含 UI 状态，订阅 Event 更新
  可序列化/持久化                    不可序列化，仅运行时存在
  通过 Event 传递给前端              通过 Command 携带数据发送给后端
```

### 4.2 后端领域模型（rshell-core 定义）

```rust
// 应用全局状态（GPUI Entity）
struct AppState {
    // 会话
    session_manager: Entity<SessionManager>,
    // 活跃连接
    active_connections: HashMap<Uuid, Entity<ConnectionHandle>>,
    // 选项卡
    tab_manager: Entity<TabManager>,
    // 传输队列
    transfer_queue: Entity<TransferQueue>,
    // 隧道
    tunnel_manager: Entity<TunnelManager>,
    // 密钥
    key_manager: Entity<KeyManager>,
    // 快速命令
    quick_commands: Entity<QuickCommandManager>,
    // 触发器
    trigger_engine: Entity<TriggerEngine>,
    // 主题
    app_theme: Entity<AppTheme>,
    // 终端配色
    terminal_schemes: Vec<TerminalColorScheme>,
    // 全局设置
    settings: Entity<GlobalSettings>,
}
```

### 4.3 后端服务注册表（后端内部使用）

```rust
/// 后端服务注册表（后端内部使用，前端不感知）
/// 前端通过 CommandDispatcher 间接调用这些服务
struct BackendServices {
    session_manager: Arc<SessionManager>,
    connection_pool: Arc<ConnectionPool>,
    transfer_queue: Arc<TransferQueue>,
    tunnel_manager: Arc<TunnelManager>,
    key_manager: Arc<KeyManager>,
    quick_commands: Arc<QuickCommandManager>,
    trigger_engine: Arc<TriggerEngine>,
    settings: Arc<GlobalSettings>,
    event_bus: Arc<EventBus>,
}
```

### 4.4 前端视图模型（rshell-ui 定义）

```rust
/// 前端视图模型是后端状态在前端的“投影”
/// 每个 ViewModel 订阅 Event 并维护本地 UI 状态

/// 应用根 ViewModel
struct AppViewModel {
    // 子 ViewModel
    session_vm: SessionViewModel,
    terminal_vms: HashMap<Uuid, TerminalViewModel>,
    filemgr_vm: HashMap<Uuid, FileMgrViewModel>,
    transfer_vm: TransferViewModel,
    tunnel_vm: TunnelViewModel,

    // 全局 UI 状态
    active_tab: Option<Uuid>,
    is_fullscreen: bool,
    sidebar_visible: bool,
}

/// 会话 ViewModel
struct SessionViewModel {
    // 后端状态投影
    sessions: Vec<SessionNodeView>,  // 从后端 SessionListChanged 事件刷新
    // 本地 UI 状态
    expanded_folders: HashSet<Uuid>,
    selected_session: Option<Uuid>,
    search_query: String,
}

/// 文件管理器 ViewModel
struct FileMgrViewModel {
    // 后端状态投影
    remote_entries: Vec<RemoteFileEntry>,
    current_remote_path: String,
    transfer_tasks: Vec<TransferTaskView>,
    // 本地 UI 状态
    current_local_path: PathBuf,
    selected_local_files: HashSet<usize>,
    selected_remote_files: HashSet<usize>,
    local_scroll_offset: usize,
    remote_scroll_offset: usize,
}

/// 传输 ViewModel
struct TransferViewModel {
    // 后端状态投影
    tasks: Vec<TransferTaskView>,
    total_speed_bps: f64,
    // 本地 UI 状态
    is_panel_expanded: bool,
}

/// 传输任务视图（后端 TransferTask 的投影）
struct TransferTaskView {
    task_id: Uuid,
    filename: String,
    direction: TransferDirection,
    progress: f32,         // 0.0 ~ 1.0
    speed_text: String,    // 后端计算，前端直接展示
    state_text: String,    // “传输中”/“已暂停”/“已完成”
}
```

### 4.5 会话数据模型（后端领域模型）

```rust
// 会话节点（文件树中的节点）
enum SessionNode {
    Folder(SessionFolder),
    Session(SessionConfig),
}

// 会话文件夹
struct SessionFolder {
    id: Uuid,
    name: String,
    parent_id: Option<Uuid>,
    properties: InheritableProperties,  // 可继承属性
    children: Vec<Uuid>,                // 子节点 ID
    is_expanded: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

// 会话配置
struct SessionConfig {
    id: Uuid,
    name: String,
    folder_id: Option<Uuid>,
    // 连接信息
    host: String,
    port: u16,
    protocol: Protocol,
    // 认证
    auth_profile_id: Option<Uuid>,
    // 可覆盖的属性（None 表示继承父文件夹）
    properties: InheritableProperties,
    // 元数据
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    last_connected: Option<DateTime<Utc>>,
    connect_count: u32,
}

enum Protocol {
    SSH,
    Telnet,
    Rlogin,
    Serial,
    RDP,
    LocalShell,
}
```

### 4.6 连接配置（后端领域模型）

```rust
// 完整连接配置（解析继承后的最终配置）
struct ResolvedConnectionConfig {
    // 基本信息
    host: String,
    port: u16,
    protocol: Protocol,
    
    // SSH 特有
    ssh_config: Option<SshConfig>,
    
    // Telnet 特有
    telnet_config: Option<TelnetConfig>,
    
    // Serial 特有
    serial_config: Option<SerialConfig>,
    
    // 终端配置
    terminal_config: TerminalConfig,
    
    // 外观配置
    appearance_config: AppearanceConfig,
    
    // 安全配置
    security_config: SecurityConfig,
    
    // 代理配置
    proxy_config: Option<ProxyConfig>,
    
    // 跳转主机链
    jump_hosts: Vec<JumpHostConfig>,
    
    // 重连策略
    reconnect_policy: ReconnectPolicy,
}

struct SshConfig {
    compression: bool,
    keep_alive_interval: Duration,
    keep_alive_count_max: u32,
    x11_forwarding: bool,
    x11_display_offset: u32,
    agent_forwarding: bool,
    port_forwards: Vec<PortForwardRule>,
}

struct TelnetConfig {
    terminal_type: String,
    username: Option<String>,
}

struct SerialConfig {
    port_name: String,           // COM1, /dev/ttyUSB0
    baud_rate: u32,              // 9600, 115200, ...
    data_bits: DataBits,         // 5, 6, 7, 8
    stop_bits: StopBits,         // 1, 1.5, 2
    parity: Parity,              // None, Even, Odd, Mark, Space
    flow_control: FlowControl,   // None, Software, Hardware
}

struct TerminalConfig {
    terminal_type: String,       // xterm-256color, vt100, ...
    encoding: Encoding,          // UTF-8, GBK, EUC-JP, ...
    scrollback_lines: u32,       // 32767
    auto_wrap: bool,
    new_line_mode: NewLineMode,  // LF, CR+LF
    cursor_style: CursorStyle,   // Block, Underline, Bar
    cursor_blink: bool,
}

struct AppearanceConfig {
    color_scheme: String,
    font_family: String,
    font_size: f32,
    line_height: f32,
    opacity: f32,                // 窗口透明度
}

struct SecurityConfig {
    auth_method: AuthMethod,
    master_password_required: bool,
    auto_lock_timeout: Option<Duration>,
    host_key_policy: HostKeyPolicy,
}

struct ProxyConfig {
    proxy_type: ProxyType,       // HTTP, SOCKS4, SOCKS5
    host: String,
    port: u16,
    username: Option<String>,
    password: Option<String>,
}

struct JumpHostConfig {
    host: String,
    port: u16,
    auth_profile_id: Option<Uuid>,
}
```

### 4.7 文件传输数据模型（后端领域模型）

```rust
// 远程文件条目
struct RemoteFileEntry {
    name: String,
    file_type: FileType,
    size: u64,
    permissions: FilePermissions,
    owner: String,
    group: String,
    modified_at: DateTime<Utc>,
    accessed_at: Option<DateTime<Utc>>,
    is_symlink: bool,
    symlink_target: Option<String>,
}

enum FileType {
    RegularFile,
    Directory,
    Symlink,
    BlockDevice,
    CharDevice,
    Fifo,
    Socket,
}

struct FilePermissions {
    owner_read: bool,
    owner_write: bool,
    owner_execute: bool,
    group_read: bool,
    group_write: bool,
    group_execute: bool,
    other_read: bool,
    other_write: bool,
    other_execute: bool,
    setuid: bool,
    setgid: bool,
    sticky: bool,
}

// 文件过滤器
struct FileFilter {
    patterns: Vec<String>,       // glob 模式: *.log, *.tmp
    show_hidden: bool,
    min_size: Option<u64>,
    max_size: Option<u64>,
    modified_after: Option<DateTime<Utc>>,
    modified_before: Option<DateTime<Utc>>,
}
```

### 4.8 全局设置（后端领域模型）

```rust
struct GlobalSettings {
    // 通用
    language: String,            // "zh-CN", "en-US"
    auto_check_update: bool,
    confirm_on_exit: bool,
    
    // 外观
    app_theme: String,           // "light", "dark", "system"
    default_color_scheme: String,
    default_font_family: String,
    default_font_size: f32,
    
    // 终端
    default_terminal_type: String,
    default_encoding: Encoding,
    default_scrollback_lines: u32,
    copy_on_select: bool,
    paste_with_right_click: bool,
    
    // 连接
    default_port: u16,           // 22
    connect_timeout_ms: u64,     // 10000
    default_reconnect_policy: ReconnectPolicy,
    
    // 文件传输
    default_transfer_buffer_size: usize,
    max_concurrent_transfers: usize,
    default_sync_mode: SyncMode,
    
    // 安全
    master_password_enabled: bool,
    auto_lock_timeout_minutes: Option<u32>,
    
    // 日志
    log_enabled: bool,
    log_dir: PathBuf,
    log_level: LogLevel,
    
    // 编辑器
    external_editor: Option<String>,
}
```

---

## 5. 关键接口设计

### 5.1 接口分层概览

```
┌─────────────────────────────────────────────────────────────┐
│  前端接口层 (rshell-ui)                                       │
│  - View 组件接口（GPUI View trait）                         │
│  - ViewModel 接口（handle_event / on_user_action）          │
├─────────────────────────────────────────────────────────────┤
│  ★ 前后端边界 ★                                              │
│  - AppCommand (前端 → 后端)                                 │
│  - AppEvent (后端 → 前端)                                   │
│  - CommandDispatcher / EventBus                             │
├─────────────────────────────────────────────────────────────┤
│  后端服务接口层 (rshell-core)                                │
│  - SessionService trait                                     │
│  - TerminalService trait                                    │
│  - TransferService trait                                    │
│  - SecurityService trait                                    │
├─────────────────────────────────────────────────────────────┤
│  协议接口层 (rshell-protocol)                                │
│  - Connection trait                                         │
│  - SftpClient trait                                         │
└─────────────────────────────────────────────────────────────┘
```

### 5.2 连接抽象 Trait（协议层）

```rust
/// 统一的连接抽象，所有协议实现此 trait
#[async_trait]
trait Connection: Send + Sync {
    /// 建立连接
    async fn connect(&mut self, config: &ResolvedConnectionConfig) -> Result<()>;
    /// 断开连接
    async fn disconnect(&mut self) -> Result<()>;
    /// 是否已连接
    fn is_connected(&self) -> bool;
    /// 获取连接信息
    fn connection_info(&self) -> ConnectionInfo;
    
    /// 读取输出数据
    async fn read_output(&mut self, buf: &mut [u8]) -> Result<usize>;
    /// 写入输入数据
    async fn write_input(&mut self, data: &[u8]) -> Result<usize>;
    
    /// 调整终端尺寸
    async fn resize_terminal(&mut self, cols: u16, rows: u16) -> Result<()>;
    
    /// 获取 SFTP 子系统（仅 SSH 支持）
    fn sftp_subsystem(&self) -> Option<Box<dyn SftpClient>>;
    
    /// 创建端口转发通道
    async fn create_port_forward(&self, rule: &PortForwardRule) -> Result<Box<dyn AsyncReadWrite>>;
}

/// 连接信息
struct ConnectionInfo {
    protocol: Protocol,
    host: String,
    port: u16,
    state: ConnectionState,
    bytes_sent: u64,
    bytes_received: u64,
    connected_since: Option<Instant>,
    latency_ms: Option<u64>,
}

enum ConnectionState {
    Connecting,
    Connected,
    Authenticating,
    Disconnecting,
    Disconnected,
    Reconnecting,
}
```

### 5.3 SFTP 客户端 Trait（协议层）

```rust
/// SFTP 文件操作抽象
#[async_trait]
trait SftpClient: Send + Sync {
    /// 列出目录内容
    async fn read_dir(&self, path: &str) -> Result<Vec<RemoteFileEntry>>;
    /// 获取文件/目录属性
    async fn stat(&self, path: &str) -> Result<RemoteFileEntry>;
    /// 获取文件属性（不跟随符号链接）
    async fn lstat(&self, path: &str) -> Result<RemoteFileEntry>;
    /// 设置文件属性
    async fn set_stat(&self, path: &str, attrs: SetAttrs) -> Result<()>;
    
    /// 打开文件
    async fn open_file(&self, path: &str, flags: OpenFlags) -> Result<Box<dyn SftpFile>>;
    /// 读取文件内容
    async fn read_file(&self, path: &str) -> Result<Vec<u8>>;
    /// 写入文件内容
    async fn write_file(&self, path: &str, data: &[u8]) -> Result<()>;
    
    /// 创建目录
    async fn create_dir(&self, path: &str) -> Result<()>;
    /// 删除文件
    async fn remove_file(&self, path: &str) -> Result<()>;
    /// 删除目录
    async fn remove_dir(&self, path: &str) -> Result<()>;
    /// 重命名
    async fn rename(&self, from: &str, to: &str) -> Result<()>;
    /// 创建符号链接
    async fn symlink(&self, src: &str, dst: &str) -> Result<()>;
    /// 读取符号链接目标
    async fn read_link(&self, path: &str) -> Result<String>;
    /// 获取当前工作目录
    async fn current_dir(&self) -> Result<String>;
    /// 更改工作目录
    async fn change_dir(&self, path: &str) -> Result<()>;
    
    /// 获取文件系统统计
    async fn statvfs(&self, path: &str) -> Result<StatVfs>;
}

/// SFTP 文件操作
#[async_trait]
trait SftpFile: Send + Sync {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    async fn write(&mut self, data: &[u8]) -> Result<usize>;
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64>;
    async fn flush(&mut self) -> Result<()>;
    async fn close(self) -> Result<()>;
}
```

### 5.4 终端服务 Trait（后端服务层）

```rust
/// 终端服务接口（后端提供，前端不直接调用）
/// 前端通过 CommandDispatcher 间接调用
trait TerminalService: Send + Sync {
    /// 创建新终端实例
    fn create_terminal(&self, config: TerminalConfig) -> Result<Uuid>;
    /// 销毁终端实例
    fn destroy_terminal(&self, terminal_id: Uuid);
    /// 发送用户输入到终端
    async fn send_input(&self, terminal_id: Uuid, data: &[u8]) -> Result<()>;
    /// 调整终端尺寸
    async fn resize_terminal(&self, terminal_id: Uuid, cols: u16, rows: u16) -> Result<()>;
    /// 获取终端缓冲区内容（供前端渲染）
    fn get_buffer_snapshot(&self, terminal_id: Uuid) -> Result<TerminalBufferSnapshot>;
    /// 搜索终端内容
    fn search(&self, terminal_id: Uuid, pattern: &str) -> Result<Vec<SearchMatch>>;
}

/// 终端缓冲区快照（后端传递给前端的数据结构）
/// 前端用此数据渲染终端，不包含任何后端引用
struct TerminalBufferSnapshot {
    rows: usize,
    cols: usize,
    cells: Vec<CellView>,  // 扁平数组，row * cols
    cursor_row: usize,
    cursor_col: usize,
    cursor_visible: bool,
    title: String,
}

/// 单元格视图数据（后端→前端的纯数据）
struct CellView {
    character: char,
    fg_color: [u8; 4],   // RGBA，前端直接使用
    bg_color: [u8; 4],
    flags: CellFlags,
}
```

### 5.5 会话服务 Trait（后端服务层）

```rust
/// 会话管理服务
trait SessionService {
    /// 创建会话
    fn create_session(&self, config: SessionConfig) -> Result<Uuid>;
    /// 更新会话
    fn update_session(&self, id: Uuid, config: SessionConfig) -> Result<()>;
    /// 删除会话
    fn delete_session(&self, id: Uuid) -> Result<()>;
    /// 连接会话
    async fn connect_session(&self, id: Uuid) -> Result<Entity<ConnectionHandle>>;
    /// 断开会话
    async fn disconnect_session(&self, id: Uuid) -> Result<()>;
    /// 获取解析后的配置（含继承）
    fn resolve_config(&self, id: Uuid) -> Result<ResolvedConnectionConfig>;
    /// 列出所有会话
    fn list_sessions(&self) -> Vec<SessionConfig>;
    /// 搜索会话
    fn search_sessions(&self, query: &str) -> Vec<SessionConfig>;
    /// 导入会话（兼容 Xshell .xsh 格式）
    fn import_session(&self, path: &Path) -> Result<Uuid>;
    /// 导出会话
    fn export_session(&self, id: Uuid, path: &Path) -> Result<()>;
}
```

### 5.6 传输服务 Trait（后端服务层）

```rust
/// 文件传输服务
#[async_trait]
trait TransferService {
    /// 添加上传任务
    fn enqueue_upload(&self, local: &Path, remote: &str, sftp: Entity<SftpHandle>) -> Result<Uuid>;
    /// 添加下载任务
    fn enqueue_download(&self, remote: &str, local: &Path, sftp: Entity<SftpHandle>) -> Result<Uuid>;
    /// 添加目录同步任务
    fn enqueue_sync(&self, local: &Path, remote: &str, mode: SyncMode, sftp: Entity<SftpHandle>) -> Result<Uuid>;
    /// 暂停任务
    fn pause_task(&self, task_id: Uuid) -> Result<()>;
    /// 恢复任务
    fn resume_task(&self, task_id: Uuid) -> Result<()>;
    /// 取消任务
    fn cancel_task(&self, task_id: Uuid) -> Result<()>;
    /// 获取传输状态
    fn task_status(&self, task_id: Uuid) -> Result<TransferTaskStatus>;
    /// 获取队列概览
    fn queue_overview(&self) -> TransferQueueOverview;
}
```

### 5.7 事件系统（前后端通信核心）

```rust
/// 应用事件（模块间通信）
enum AppEvent {
    // 连接事件
    ConnectionEstablished { session_id: Uuid, info: ConnectionInfo },
    ConnectionLost { session_id: Uuid, reason: DisconnectReason },
    ConnectionReconnecting { session_id: Uuid, attempt: u32 },
    
    // 终端事件
    TerminalOutput { session_id: Uuid, data: Vec<u8> },
    TerminalTitleChanged { session_id: Uuid, title: String },
    TerminalResized { session_id: Uuid, cols: u16, rows: u16 },
    
    // 传输事件
    TransferStarted { task_id: Uuid },
    TransferProgress { task_id: Uuid, bytes: u64, total: u64, speed_bps: f64 },
    TransferCompleted { task_id: Uuid, duration: Duration },
    TransferFailed { task_id: Uuid, error: TransferError },
    
    // 会话事件
    SessionCreated { session_id: Uuid },
    SessionUpdated { session_id: Uuid },
    SessionDeleted { session_id: Uuid },
    
    // 触发器事件
    TriggerFired { trigger_id: Uuid, session_id: Uuid, action: TriggerAction },
    
    // 隧道事件
    TunnelCreated { tunnel_id: Uuid },
    TunnelClosed { tunnel_id: Uuid },
    TunnelError { tunnel_id: Uuid, error: String },
    
    // 安全事件
    HostKeyMismatch { host: String, expected: String, received: String },
    MasterPasswordRequired,
    AutoLockTriggered,
}
```

---

## 6. 第三方依赖选型

### 6.1 Crate 分层与依赖规则

```
Cargo Workspace (rshell)
├── rshell-infra        ← 基础设施层（不依赖 GPUI）
├── rshell-protocol     ← 协议层（不依赖 GPUI）
├── rshell-core         ← 后端层（不依赖 GPUI）
├── rshell-api          ← API 边界层（不依赖 GPUI）
├── rshell-plugin-sdk   ← 插件 SDK（不依赖 GPUI）
└── rshell-ui           ← 前端层（依赖 GPUI + rshell-api）
```

**依赖规则**：
- `rshell-ui` 可依赖 `rshell-api`，**不可**直接依赖 `rshell-core` / `rshell-protocol`
- `rshell-core` 可依赖 `rshell-protocol` 和 `rshell-infra`，**不可**依赖 GPUI
- `rshell-protocol` 可依赖 `rshell-infra`，**不可**依赖 GPUI
- `rshell-api` 是纯数据类型定义（Command/Event），仅依赖 serde/uuid，**不依赖运行时 crate**
- `rshell-plugin-sdk` 可依赖 `rshell-core` 和 `rshell-api`，**不可**依赖 GPUI

### 6.2 核心依赖

| Crate | 版本 | 用途 | 所属层 | License |
|-------|------|------|--------|---------|
| `gpui` | 0.2 | GPU 加速 UI 框架 | rshell-ui | Apache-2.0 |
| `gpui-component` | 0.5 | 桌面 UI 组件库 | rshell-ui | Apache-2.0 |
| `russh` | 0.48+ | 纯 Rust SSH 客户端/服务端 | rshell-protocol | Apache-2.0 |
| `russh-sftp` | 0.14+ | SFTP 子系统（基于 russh） | rshell-protocol | Apache-2.0 |
| `russh-keys` | 0.48+ | SSH 密钥管理 | rshell-protocol | Apache-2.0 |
| `alacritty_terminal` | 0.24+ | 终端仿真引擎（VT 解析） | rshell-core | Apache-2.0 |
| `tokio` | 1.x | 异步运行时 | rshell-core, protocol | MIT |
| `serde` | 1.x | 序列化/反序列化 | 全层 | MIT/Apache-2.0 |
| `toml` | 0.8+ | TOML 配置解析 | rshell-infra | MIT/Apache-2.0 |

### 6.3 协议相关

| Crate | 版本 | 用途 | License |
|-------|------|------|---------|
| `suppaftp` | 6.x | FTP/FTPS 客户端 | Apache-2.0 |
| `serialport` | 4.x | 跨平台串口通信 | MIT |
| `ironrdp` | 最新 | RDP 协议实现 | MIT/Apache-2.0 |
| `async-ssh2-tokio` | 最新 | 备选 SSH 异步封装 | MIT |

### 6.4 基础设施

| Crate | 版本 | 用途 | License |
|-------|------|------|---------|
| `ring` | 0.17+ | 加密算法 | ISC/OpenSSL |
| `rustls` | 0.23+ | TLS 实现 | Apache-2.0/ISC |
| `ssh-key` | 0.6+ | SSH 密钥格式解析 | Apache-2.0/MIT |
| `rhai` | 1.x | 嵌入式脚本引擎 | MIT/Apache-2.0 |
| `keyring` | 3.x | 系统密钥环访问 | MIT/Apache-2.0 |
| `portable-pty` | 0.8+ | 跨平台 PTY 抽象 | MIT/Apache-2.0 |
| `tracing` | 0.1+ | 结构化日志 | MIT |
| `tracing-subscriber` | 0.3+ | 日志输出 | MIT |
| `chrono` | 0.4+ | 日期时间处理 | MIT/Apache-2.0 |
| `uuid` | 1.x | UUID 生成 | Apache-2.0/MIT |
| `regex` | 1.x | 正则表达式 | MIT/Apache-2.0 |
| `glob` | 0.3+ | 文件模式匹配 | MIT/Apache-2.0 |
| `dirs` | 5.x | 跨平台目录路径 | MIT/Apache-2.0 |
| `open` | 5.x | 打开外部程序/URL | MIT |
| `font-kit` | 0.14+ | 字体加载与回退 | MIT/Apache-2.0 |

### 6.5 插件系统依赖

| Crate | 版本 | 用途 | 所属层 | License |
|-------|------|------|--------|---------|
| `wasmtime` | 28+ | WASM 插件运行时 | rshell-plugin-sdk | Apache-2.0 |
| `libloading` | 0.8+ | 动态库插件加载 | rshell-plugin-sdk | ISC |
| `sha2` | 0.10+ | 插件签名验证 | rshell-plugin-sdk | MIT/Apache-2.0 |
| `jsonschema` | 0.28+ | 插件配置 Schema 验证 | rshell-plugin-sdk | MIT |

### 6.6 开发依赖

| Crate | 版本 | 用途 |
|-------|------|------|
| `cargo-watch` | 最新 | 开发热重载 |
| `criterion` | 0.5+ | 性能基准测试 |
| `tokio-test` | 最新 | 异步测试工具 |
| `mockall` | 0.13+ | Mock 生成器（单元测试） |

---

## 7. 开发路线图

### Phase 1：MVP — 核心终端（8-10 周）

**目标**：实现可用的 SSH 终端，可替代 Xshell 基本功能

| 周次 | 任务 | 交付物 |
|------|------|--------|
| W1-2 | 项目脚手架搭建，GPUI 窗口/布局框架 | 可运行的空壳应用 |
| W3-4 | PTY 集成 + alacritty_terminal VT 解析 | 终端可显示命令输出 |
| W5-6 | russh SSH 连接 + 密码/公钥认证 | 可 SSH 登录远程主机 |
| W7 | 终端 GPUI 渲染（字形图集 + 颜色） | 终端视觉可用 |
| W8 | 会话管理（CRUD + 持久化） | 会话可保存/加载 |
| W9 | 标签式多会话 + 基本设置 | 多标签并行工作 |
| W10 | 测试、Bug 修复、性能优化 | Phase 1 发布 |

**Phase 1 功能清单**：
- ✅ SSH2 连接（密码 + 公钥认证）
- ✅ 终端仿真（xterm-256color，VT100/220）
- ✅ 标签式多会话
- ✅ 会话管理器（创建/编辑/删除/连接）
- ✅ 基本终端功能（滚动、搜索、复制粘贴）
- ✅ 基本设置（字体、配色方案）

### Phase 2：文件传输（6-8 周）

**目标**：实现 SFTP 文件管理器，替代 Xftp 核心功能

| 周次 | 任务 | 交付物 |
|------|------|--------|
| W11-12 | russh-sftp 集成 + 远程文件浏览 | 可浏览远程目录 |
| W13-14 | 双窗格文件管理器 UI | 本地+远程并排显示 |
| W15-16 | 文件上传/下载 + 传输队列 | 文件可传输 |
| W17 | 断点续传 + 暂停/恢复 | 大文件可靠传输 |
| W18 | 文件夹同步 + 同步浏览 | 目录同步功能 |

**Phase 2 功能清单**：
- ✅ SFTP 远程文件浏览（目录列表、文件属性）
- ✅ 双窗格文件管理器
- ✅ 文件上传/下载（拖放）
- ✅ 传输队列管理（暂停/恢复/取消）
- ✅ 断点续传
- ✅ 文件夹同步

### Phase 3：效率工具（6-8 周）

**目标**：实现 Xshell 高级效率功能

| 周次 | 任务 | 交付物 |
|------|------|--------|
| W19-20 | 快速命令管理器 | 命令按钮一键执行 |
| W21-22 | 撰写窗格 + 同步输入 | 多会话批量命令 |
| W23-24 | 触发器引擎 | 自动响应终端输出 |
| W25-26 | Rhai 脚本引擎 + 脚本录制 | 自动化脚本 |

**Phase 3 功能清单**：
- ✅ 快速命令管理器
- ✅ 撰写窗格（多行编辑发送）
- ✅ 同步输入（多终端同步）
- ✅ 触发器（自动执行动作）
- ✅ Rhai 脚本引擎
- ✅ 脚本录制

### Phase 4：安全与隧道（4-6 周）

**目标**：完善安全功能与隧道管理

| 周次 | 任务 | 交付物 |
|------|------|--------|
| W27-28 | 密钥管理器（生成/导入/导出） | 密钥管理 UI |
| W29-30 | 主密码系统 | 密码加密存储 |
| W31-32 | 端口转发 + 隧道管理 | SSH 隧道功能 |

**Phase 4 功能清单**：
- ✅ SSH 密钥生成/导入/导出
- ✅ 主密码加密
- ✅ 本地/远程端口转发
- ✅ 动态端口转发（SOCKS）
- ✅ 即时隧道创建/管理
- ✅ 主机密钥管理

### Phase 5：多协议 & 完善（6-8 周）

**目标**：补全所有协议，完善体验

| 周次 | 任务 | 交付物 |
|------|------|--------|
| W33-34 | Telnet 协议实现 | Telnet 连接可用 |
| W35 | Serial 串口连接 | IoT 设备连接 |
| W36 | RDP 远程桌面（ironrdp） | RDP 连接可用 |
| W37 | 主题系统 + 配色方案导入/导出 | 完整外观定制 |
| W38 | 会话导入（Xshell .xsh 格式兼容） | 迁移便利 |
| W39-40 | 全面测试、性能优化、发布 | 正式版发布 |

**Phase 5 功能清单**：
- ✅ Telnet 协议
- ✅ Serial 串口
- ✅ RDP 远程桌面
- ✅ 完整主题系统
- ✅ 配色方案导入/导出
- ✅ Xshell 会话文件兼容
- ✅ Zmodem 文件传输
- ✅ FTP/FTPS 支持

### Phase 6：插件化扩展（4-6 周）

**目标**：建立插件生态，允许第三方扩展 RShell 功能

| 周次 | 任务 | 交付物 |
|------|------|--------|
| W41-42 | Plugin SDK 设计 + Plugin Host 实现 | 插件加载/卸载框架 |
| W43 | WASM 插件沙箱 + 宿主函数注册 | WASM 插件可运行 |
| W44 | 插件 API 完善（协议/主题/工具面板扩展点） | 扩展点可用 |
| W45 | 插件管理器 UI + 插件目录 | 插件安装/管理 |

**Phase 6 功能清单**：
- ✅ 插件 SDK（Rust trait + WASM 支持）
- ✅ 插件沙箱（内存隔离、权限控制）
- ✅ 协议扩展插件
- ✅ 主题/配色方案插件
- ✅ 工具面板插件
- ✅ 插件管理器 UI
- ✅ 插件目录/市场

### 里程碑总览

| 里程碑 | 时间 | 功能覆盖度 |
|--------|------|------------|
| Phase 1 完成 | ~10 周 | Xshell 核心终端功能 (60%) |
| Phase 2 完成 | ~18 周 | + Xftp 核心文件传输 (80%) |
| Phase 3 完成 | ~26 周 | + 效率工具 (90%) |
| Phase 4 完成 | ~32 周 | + 安全与隧道 (95%) |
| Phase 5 完成 | ~40 周 | 全部功能 (100%) |
| Phase 6 完成 | ~46 周 | + 插件生态 (100% + 可扩展) |

---

## 附录

### A. 术语表

| 术语 | 说明 |
|------|------|
| PTY | Pseudo Terminal，伪终端 |
| VT | Video Terminal，视频终端（VT100/220/320 等标准） |
| CSI | Control Sequence Introducer，ANSI 控制序列引导符 |
| OSC | Operating System Command，操作系统命令序列 |
| SFTP | SSH File Transfer Protocol，SSH 文件传输协议 |
| FXP | File eXchange Protocol，服务器间文件传输 |
| KEX | Key Exchange，密钥交换 |
| MAC | Message Authentication Code，消息认证码 |
| ConPTY | Windows Console Pseudo-Terminal API |
| GPUI | GPU-accelerated UI Framework |
| WASM | WebAssembly，插件沙箱执行环境 |

### B. 参考资料

- Xshell 官方功能列表：https://www.xshell.com/zh/xshell/
- Xftp 官方功能列表：https://www.xshell.com/zh/xftp/
- GPUI 框架：https://github.com/zed-industries/zed
- GPUI Component：https://github.com/longbridge/gpui-component
- russh SSH 库：https://github.com/warp-tech/russh
- alacritty_terminal：https://github.com/alacritty/alacritty
- ironrdp RDP 库：https://github.com/Devolutions/IronRDP

---

## 8. UI 视觉与交互设计规范

> 本节定义 RShell 的完整 UI 视觉语言、交互规范与组件库设计原则，确保整体视觉风格专业统一，交互体验流畅自然。

### 8.1 设计语言与视觉基础

#### 8.1.1 设计理念

RShell 的视觉设计遵循以下原则：

- **专业克制**：面向开发者和运维人员，视觉风格简洁专业，避免过度装饰
- **信息密度优先**：在保证可读性的前提下，最大化信息展示密度
- **层次清晰**：通过色彩、间距、阴影建立清晰的视觉层次
- **一致性**：全应用统一的视觉语言，降低用户认知负担
- **可访问性**：支持高对比度模式，符合 WCAG 2.1 AA 标准

#### 8.1.2 色彩系统

##### 8.1.2.1 基础色板

```rust
// 品牌色
const BRAND_PRIMARY: Rgba = rgba("#2563EB");      // 主品牌蓝
const BRAND_SECONDARY: Rgba = rgba("#7C3AED");    // 辅助紫
const BRAND_ACCENT: Rgba = rgba("#06B6D4");       // 强调青

// 中性色阶（Dark 模式）
const DARK_BG_BASE: Rgba = rgba("#1E1E1E");       // 基础背景
const DARK_BG_ELEVATED: Rgba = rgba("#252525");   // 提升背景（卡片、面板）
const DARK_BG_SURFACE: Rgba = rgba("#2D2D2D");    // 表面背景（输入框、按钮）
const DARK_BG_HOVER: Rgba = rgba("#3A3A3A");      // 悬停背景
const DARK_BG_ACTIVE: Rgba = rgba("#4A4A4A");     // 激活背景
const DARK_BORDER: Rgba = rgba("#404040");         // 边框
const DARK_TEXT_PRIMARY: Rgba = rgba("#E5E5E5");   // 主文本
const DARK_TEXT_SECONDARY: Rgba = rgba("#A3A3A3"); // 次要文本
const DARK_TEXT_DISABLED: Rgba = rgba("#737373");  // 禁用文本

// 中性色阶（Light 模式）
const LIGHT_BG_BASE: Rgba = rgba("#FFFFFF");      // 基础背景
const LIGHT_BG_ELEVATED: Rgba = rgba("#F9FAFB"); // 提升背景
const LIGHT_BG_SURFACE: Rgba = rgba("#F3F4F6");   // 表面背景
const LIGHT_BG_HOVER: Rgba = rgba("#E5E7EB");     // 悬停背景
const LIGHT_BG_ACTIVE: Rgba = rgba("#D1D5DB");    // 激活背景
const LIGHT_BORDER: Rgba = rgba("#E5E7EB");        // 边框
const LIGHT_TEXT_PRIMARY: Rgba = rgba("#111827");  // 主文本
const LIGHT_TEXT_SECONDARY: Rgba = rgba("#6B7280"); // 次要文本
const LIGHT_TEXT_DISABLED: Rgba = rgba("#9CA3AF");  // 禁用文本

// 语义色
const SUCCESS: Rgba = rgba("#10B981");            // 成功/已连接
const WARNING: Rgba = rgba("#F59E0B");            // 警告/注意
const ERROR: Rgba = rgba("#EF4444");              // 错误/断开
const INFO: Rgba = rgba("#3B82F6");               // 信息/提示
```

##### 8.1.2.2 色彩应用规范

| 应用场景 | Dark 模式 | Light 模式 | 说明 |
|----------|-----------|------------|------|
| 主窗口背景 | `DARK_BG_BASE` | `LIGHT_BG_BASE` | 应用整体背景 |
| 侧边栏背景 | `DARK_BG_ELEVATED` | `LIGHT_BG_ELEVATED` | 会话管理器、工具面板 |
| 工具栏/状态栏 | `DARK_BG_SURFACE` | `LIGHT_BG_SURFACE` | 顶部工具栏、底部状态栏 |
| 标签栏背景 | `DARK_BG_SURFACE` | `LIGHT_BG_SURFACE` | 活动标签下方区域 |
| 活动标签 | `DARK_BG_BASE` | `LIGHT_BG_BASE` | 当前选中标签 |
| 非活动标签 | `DARK_BG_ELEVATED` | `LIGHT_BG_ELEVATED` | 未选中标签 |
| 按钮默认 | `DARK_BG_SURFACE` | `LIGHT_BG_SURFACE` | 普通按钮背景 |
| 按钮主要 | `BRAND_PRIMARY` | `BRAND_PRIMARY` | 主要操作按钮（连接、保存） |
| 按钮危险 | `ERROR` | `ERROR` | 危险操作（删除、断开） |
| 边框 | `DARK_BORDER` | `LIGHT_BORDER` | 分隔线、输入框边框 |
| 主文本 | `DARK_TEXT_PRIMARY` | `LIGHT_TEXT_PRIMARY` | 标题、正文 |
| 次要文本 | `DARK_TEXT_SECONDARY` | `LIGHT_TEXT_SECONDARY` | 说明文字、时间戳 |
| 禁用文本 | `DARK_TEXT_DISABLED` | `LIGHT_TEXT_DISABLED` | 不可交互元素 |
| 选区背景 | `BRAND_PRIMARY` @ 30% | `BRAND_PRIMARY` @ 20% | 文本选中背景 |
| 悬停高亮 | `DARK_BG_HOVER` | `LIGHT_BG_HOVER` | 列表项、按钮悬停 |

#### 8.1.3 字体排版系统

##### 8.1.3.1 字体族

```rust
// UI 字体（界面元素）
const UI_FONT_FAMILY: &str = "Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', 
                               'PingFang SC', 'Microsoft YaHei', sans-serif";

// 等宽字体（终端、代码）
const MONO_FONT_FAMILY: &str = "'JetBrains Mono', 'Fira Code', 'Cascadia Code', 
                                 'Consolas', 'Courier New', monospace";

// 字体回退链
// 1. 首选字体
// 2. 系统默认 UI 字体
// 3. 中文字体（PingFang SC / Microsoft YaHei）
// 4. 通用 sans-serif
```

##### 8.1.3.2 字号层级

| 层级 | 字号 | 行高 | 字重 | 应用场景 |
|------|------|------|------|----------|
| Display | 20px | 28px | SemiBold (600) | 模态对话框标题 |
| Title | 16px | 24px | SemiBold (600) | 页面标题、面板标题 |
| Heading | 14px | 20px | Medium (500) | 分区标题、表格表头 |
| Body | 13px | 18px | Regular (400) | 正文、列表项、表单标签 |
| Caption | 12px | 16px | Regular (400) | 说明文字、时间戳、状态栏 |
| Mini | 11px | 14px | Regular (400) | 角标、徽标、极小文本 |
| Terminal | 13px | 1.3x | Regular (400) | 终端默认字号（用户可调） |

##### 8.1.3.3 排版规则

- 标题与正文之间至少保持 8px 间距
- 段落间距使用 1em（当前字号的倍数）
- 文本对齐：左对齐为主，数字右对齐，居中仅用于模态框标题
- 长文本截断：使用省略号（...）+ Tooltip 显示完整内容
- 多行文本：最多显示 3 行，超出截断并显示"更多"按钮

#### 8.1.4 间距与栅格系统

##### 8.1.4.1 基础间距单位

```rust
// 基础间距单位：4px
const SPACING_XS: f32 = 4.0;    // 紧凑元素内部间距
const SPACING_SM: f32 = 8.0;    // 相关元素间距
const SPACING_MD: f32 = 12.0;   // 标准元素间距
const SPACING_LG: f32 = 16.0;   // 分组元素间距
const SPACING_XL: f32 = 24.0;   // 区域间距
const SPACING_2XL: f32 = 32.0;  // 大区域间距
```

##### 8.1.4.2 应用布局尺寸

| 元素 | 尺寸 | 说明 |
|------|------|------|
| 菜单栏高度 | 32px | 标准应用菜单 |
| 工具栏高度 | 40px | 可自定义按钮工具栏 |
| 侧边栏宽度 | 240px（最小 180px，最大 400px） | 会话管理器面板 |
| 标签栏高度 | 36px | 选项卡高度 |
| 状态栏高度 | 28px | 底部状态信息栏 |
| 面板标题栏 | 32px | 可停靠面板标题 |
| 按钮高度 | 28px（小）、32px（中）、36px（大） | 按钮尺寸 |
| 输入框高度 | 28px（小）、32px（中） | 表单输入框 |
| 图标尺寸 | 12px（小）、16px（中）、20px（大）、24px（特大） | 图标统一尺寸 |

#### 8.1.5 图标系统

##### 8.1.5.1 图标库选型

- **主图标库**：Lucide Icons（MIT 许可，24px 基准，线条风格）
- **终端图标**：自定义 SVG（终端特有概念，如 SSH/Telnet/Serial 协议图标）
- **文件类型图标**：vscode-icons（文件类型识别）

##### 8.1.5.2 图标规范

- 所有图标使用 SVG 格式，支持任意缩放
- 线条宽度统一为 1.5px（24px 基准）
- 图标颜色继承文本颜色（`currentColor`）
- 状态色：默认使用次要文本色，悬停使用主文本色，激活使用品牌色

##### 8.1.5.3 核心图标清单

| 图标名称 | 用途 | 尺寸 |
|----------|------|------|
| `icon-ssh` | SSH 协议标识 | 16px |
| `icon-telnet` | Telnet 协议标识 | 16px |
| `icon-serial` | Serial 协议标识 | 16px |
| `icon-rdp` | RDP 协议标识 | 16px |
| `icon-folder` | 文件夹（会话分组） | 16px |
| `icon-folder-open` | 展开的文件夹 | 16px |
| `icon-terminal` | 终端标签 | 16px |
| `icon-file` | 普通文件 | 16px |
| `icon-file-directory` | 远程目录 | 16px |
| `icon-upload` | 上传操作 | 16px |
| `icon-download` | 下载操作 | 16px |
| `icon-transfer` | 传输队列 | 16px |
| `icon-tunnel` | 隧道面板 | 16px |
| `icon-lock` | 安全连接标识 | 12px |
| `icon-unlock` | 不安全连接标识 | 12px |
| `icon-close` | 关闭标签/对话框 | 12px |
| `icon-add` | 新建会话/文件夹 | 16px |
| `icon-refresh` | 刷新/重连 | 16px |
| `icon-search` | 搜索功能 | 16px |
| `icon-settings` | 设置 | 16px |
| `icon-more-vertical` | 更多操作菜单 | 16px |

#### 8.1.6 阴影与圆角

##### 8.1.6.1 阴影层级

```rust
// 阴影定义（仅 Light 模式使用，Dark 模式使用边框区分层次）
const SHADOW_SM: Shadow = Shadow {
    offset_y: 1.0,
    blur: 2.0,
    color: rgba("#000000", 0.05),
};  // 按钮、输入框

const SHADOW_MD: Shadow = Shadow {
    offset_y: 2.0,
    blur: 8.0,
    color: rgba("#000000", 0.1),
};  // 下拉菜单、弹出层

const SHADOW_LG: Shadow = Shadow {
    offset_y: 4.0,
    blur: 16.0,
    color: rgba("#000000", 0.15),
};  // 模态对话框

const SHADOW_XL: Shadow = Shadow {
    offset_y: 8.0,
    blur: 24.0,
    color: rgba("#000000", 0.2),
};  // 拖拽中的面板
```

##### 8.1.6.2 圆角规范

| 元素 | 圆角 | 说明 |
|------|------|------|
| 按钮 | 6px | 标准按钮、输入框 |
| 卡片/面板 | 8px | 模态框、下拉菜单 |
| 标签 | 4px | 选项卡、徽标 |
| 头像/图标容器 | 50% | 圆形裁剪 |
| 进度条 | 4px | 传输进度条 |
| Tooltip | 4px | 提示气泡 |

---

### 8.2 核心界面详细设计

#### 8.2.1 主应用布局

##### 8.2.1.1 整体布局结构

```
┌─────────────────────────────────────────────────────────────────────┐
│  菜单栏 (32px)                                                       │
│  文件 | 编辑 | 查看 | 会话 | 工具 | 帮助                            │
├─────────────────────────────────────────────────────────────────────┤
│  工具栏 (40px)                                                        │
│  [🔗连接▼] [📁文件] [⚡快速命令▼] [🔧隧道] [⚙设置]               │
├────────┬────────────────────────────────────────────────┬───────────┤
│        │              主工作区                            │           │
│  会话  │  ┌─────────────────────────────────────────┐ │   隧道    │
│  管理  │  │  标签栏 (36px)                            │ │   面板    │
│  器    │  │  [web-01 ×] [db-master ×] [+SFTP]       │ │  (可停靠) │
│        │  ├─────────────────────────────────────────┤ │           │
│ (左侧  │  │                                          │ │  本地端口 │
│  面板) │  │  终端视图 / 文件管理器 / 设置            │ │  远程端口 │
│        │  │                                          │ │  动态SOCKS│
│ 240px  │  │                                          │ │           │
│ 可调  │  │                                          │ │           │
│        │  │                                          │ │           │
├────────┴────────────────────────────────────────────────┴───────────┤
│  状态栏 (28px)                                                        │
│  🔒 已连接 | web-01:22 | SSH2 | ↑ 1.2KB/s ↓ 3.4KB/s | UTF-8      │
└─────────────────────────────────────────────────────────────────────┘
```

##### 8.2.1.2 布局规范

- **菜单栏**：固定在顶部，高度 32px，背景使用 `DARK_BG_SURFACE` / `LIGHT_BG_SURFACE`
- **工具栏**：紧贴菜单栏下方，高度 40px，按钮间距 8px
- **侧边栏**：左侧固定宽度 240px，可通过拖拽边缘调整（180px-400px），双击边缘恢复默认宽度
- **主工作区**：占据剩余空间，支持多标签、分屏布局
- **状态栏**：固定在底部，高度 28px，显示当前连接信息
- **可停靠面板**：如隧道面板，可自由拖拽到任意位置

##### 8.2.1.3 布局交互规则

- 侧边栏显示/隐藏：点击菜单"查看 → 会话管理器"或快捷键 `Ctrl+Shift+S`
- 工具栏显示/隐藏：点击菜单"查看 → 工具栏"或快捷键 `Ctrl+Shift+T`
- 状态栏显示/隐藏：点击菜单"查看 → 状态栏"或快捷键 `Ctrl+Shift+B`
- 全屏模式：`Alt+Enter`，隐藏所有面板，仅保留终端区域
- 面板拖拽：按住面板标题栏拖拽，显示停靠预览区域，释放后停靠

#### 8.2.2 会话管理器面板

##### 8.2.2.1 面板结构

```
┌─────────────────────────────────────┐
│  会话管理器                    [⚙] │  ← 标题栏 (32px)
├─────────────────────────────────────┤
│  🔍 搜索会话...                     │  ← 搜索框 (28px)
├─────────────────────────────────────┤
│  📁 生产环境                   [▼] │  ← 文件夹 (可展开/折叠)
│     ├─ 📁 Web 服务器           [▼] │
│     │  ├─ 🔗 web-prod-01       [SSH]│  ← 会话项
│     │  ├─ 🔗 web-prod-02       [SSH]│
│     │  └─ 🔗 web-prod-03       [SSH]│
│     └─ 📁 数据库              [▼] │
│        ├─ 🔗 db-master          [SSH]│
│        └─ 🔗 db-slave-01        [SSH]│
│  📁 测试环境                   [▼] │
│     ├─ 🔗 test-server-01        [SSH]│
│     └─ 🔗 test-server-02      [Telnet]│
│  📁 IoT 设备                   [▼] │
│     └─ 🔗 sensor-gateway     [Serial]│
├─────────────────────────────────────┤
│  [＋新建会话]  [＋新建文件夹]       │  ← 底部操作栏
└─────────────────────────────────────┘
```

##### 8.2.2.2 会话项设计规范

| 元素 | 尺寸/样式 | 说明 |
|------|-----------|------|
| 会话项高度 | 28px | 单行会话项 |
| 缩进 | 每级 16px | 树形层级缩进 |
| 协议图标 | 16px | SSH/Telnet/Serial 等协议标识 |
| 会话名称 | Body 字号，主文本色 | 左对齐，超长截断 |
| 连接状态指示 | 8px 圆点 | 绿色=已连接，灰色=未连接，黄色=连接中 |
| 悬停背景 | `DARK_BG_HOVER` / `LIGHT_BG_HOVER` | 鼠标悬停 |
| 选中背景 | `BRAND_PRIMARY` @ 20% | 当前选中项 |
| 右键菜单 | ContextMenu | 连接、编辑、删除、复制等 |

##### 8.2.2.3 会话管理器交互

- **展开/折叠文件夹**：点击文件夹左侧箭头图标或双击文件夹名称
- **选中会话**：单击会话项，高亮显示
- **连接会话**：双击会话项或右键菜单"连接"
- **新建会话**：点击底部"新建会话"按钮或右键文件夹"新建会话"
- **编辑会话**：右键菜单"属性"或选中后按 `Enter`
- **删除会话**：右键菜单"删除"或按 `Delete` 键，弹出确认对话框
- **拖放排序**：长按会话项 0.5s 后进入拖拽模式，可拖拽到其他文件夹
- **搜索过滤**：在搜索框输入关键词，实时过滤会话树，匹配项高亮

##### 8.2.2.4 会话状态指示

| 状态 | 图标/颜色 | 说明 |
|------|-----------|------|
| 未连接 | 灰色圆点 `#737373` | 默认状态 |
| 连接中 | 黄色圆点 `#F59E0B` + 旋转动画 | 正在建立连接 |
| 已连接 | 绿色圆点 `#10B981` | 连接成功 |
| 连接断开 | 红色圆点 `#EF4444` | 连接意外断开 |
| 重连中 | 黄色圆点 + 脉冲动画 | 正在尝试重连 |

#### 8.2.3 终端工作区

##### 8.2.3.1 终端标签栏

```
┌─────────────────────────────────────────────────────────────┐
│  [web-01 ×] [db-master ×] [+SFTP ×] [本地Shell ×]  [+]   │
└─────────────────────────────────────────────────────────────┘
```

**标签设计规范**：

| 元素 | 尺寸/样式 | 说明 |
|------|-----------|------|
| 标签高度 | 36px | 标签栏总高度 |
| 标签最小宽度 | 120px | 标签最小宽度 |
| 标签最大宽度 | 200px | 标签最大宽度，超出截断 |
| 标签间距 | 2px | 标签之间间距 |
| 标签内边距 | 左右 12px | 标签内容内边距 |
| 活动标签 | 底部 2px 品牌色边框 | 当前选中标签 |
| 非活动标签 | 背景色加深 5% | 未选中标签 |
| 关闭按钮 | 12px × 12px | 标签右侧关闭按钮 |
| 关闭按钮悬停 | 红色圆点背景 | 鼠标悬停关闭按钮 |
| 新建标签按钮 | 24px × 24px | 标签栏右侧"+"按钮 |

**标签交互**：

- **切换标签**：单击标签切换，支持 `Ctrl+Tab` 快捷键
- **关闭标签**：点击标签右侧"×"按钮或 `Ctrl+W`
- **拖拽排序**：长按标签 0.3s 后进入拖拽模式，可调整标签顺序
- **拖出窗口**：拖拽标签到主窗口外，创建独立窗口
- **双击标签**：重命名标签（仅限手动设置的标签）
- **右键菜单**：关闭、关闭其他、关闭右侧、复制会话、移动到新建窗口

##### 8.2.3.2 终端视图

```
┌─────────────────────────────────────────────────────────────┐
│  user@web-prod-01:~$                                         │
│  Last login: Tue Jul 29 10:23:45 2026 from 192.168.1.100   │
│                                                              │
│  [web-prod-01 ~]$ ls -la                                    │
│  total 32                                                   │
│  drwxr-xr-x  5 user user 4096 Jul 29 10:20 .               │
│  drwxr-xr-x  3 root root 4096 Jul 28 14:30 ..              │
│  -rw-r--r--  1 user user  220 Jul 28 14:30 .bash_logout    │
│  -rw-r--r--  1 user user 3771 Jul 28 14:30 .bashrc         │
│  drwxr-xr-x  2 user user 4096 Jul 29 10:20 logs            │
│                                                              │
│  [web-prod-01 ~]$ █                                         │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

**终端渲染规范**：

| 元素 | 规范 | 说明 |
|------|------|------|
| 字符间距 | 字体等宽，字距 0 | 严格等宽对齐 |
| 行高 | 字号 × 1.3 | 默认行高倍数 |
| 光标样式 | 块状/下划线/竖线 | 用户可配置 |
| 光标闪烁 | 500ms 间隔 | 用户可关闭 |
| 选区背景 | 品牌色 @ 20-30% | 文本选中高亮 |
| 搜索高亮 | 黄色背景 `#F59E0B` @ 30% | 搜索结果高亮 |
| 当前搜索 | 橙色背景 `#F59E0B` @ 50% | 当前匹配项 |

**终端交互**：

- **文本选择**：鼠标左键拖拽选择，支持列选择（`Alt+拖拽`）
- **复制粘贴**：选中自动复制（可配置），右键粘贴或 `Ctrl+Shift+V`
- **滚动**：鼠标滚轮滚动历史缓冲区，`Shift+PageUp/PageDown` 翻页
- **搜索**：`Ctrl+Shift+F` 打开搜索栏，支持正则表达式
- **字体缩放**：`Ctrl+加号/减号` 调整字体大小
- **全屏**：`Alt+Enter` 进入全屏模式

##### 8.2.3.3 终端搜索栏

```
┌─────────────────────────────────────────────────────────────┐
│  🔍 [搜索内容...        ] [▲] [▼] [12/45] [×] [正则] [大小写]│
└─────────────────────────────────────────────────────────────┘
```

**搜索栏规范**：

| 元素 | 尺寸/样式 | 说明 |
|------|-----------|------|
| 搜索栏高度 | 32px | 固定在终端顶部 |
| 搜索框宽度 | 200-400px | 可拖拽调整 |
| 匹配计数 | Caption 字号 | "当前/总数"格式 |
| 上/下按钮 | 24px × 24px | 跳转到上/下一个匹配 |
| 正则开关 | 切换按钮 | 启用正则表达式 |
| 大小写开关 | 切换按钮 | 区分大小写 |
| 关闭按钮 | 12px × 12px | 关闭搜索栏 |

#### 8.2.4 SFTP 文件管理器

##### 8.2.4.1 双窗格布局

```
┌─────────────────────────────────────────────────────────────┐
│  SFTP - web-prod-01                                          │
├──────────────────────────────┬──────────────────────────────┤
│  本地文件                     │  远程文件                     │
│  [/home/user/projects] [🔖] │  [/var/www/html]        [🔖] │
├──────────────────────────────┼──────────────────────────────┤
│  名称          大小   修改时间  │  名称          大小   修改时间  │
│  📁 src        -      Jul 28   │  📁 css         -      Jul 28  │
│  📁 tests      -      Jul 27   │  📁 js          -      Jul 28  │
│  📄 .env       1.2KB  Jul 29   │  📄 index.html  4.5KB  Jul 29  │
│  📄 README.md  2.3KB  Jul 25   │  📄 style.css   12KB   Jul 28  │
│  📄 package.json 800B  Jul 20   │  📄 app.js      28KB   Jul 29  │
├──────────────────────────────┴──────────────────────────────┤
│  传输队列 (3/10)  ↑ 1.2MB/s  ↓ 3.4MB/s                      │
│  📄 backup.tar.gz  ↑ 45%  [暂停] [取消]                      │
│  📄 config.json    ↓ 完成  [移除]                            │
└─────────────────────────────────────────────────────────────┘
```

##### 8.2.4.2 文件列表规范

| 元素 | 尺寸/样式 | 说明 |
|------|-----------|------|
| 路径栏高度 | 32px | 显示当前路径，可编辑 |
| 书签按钮 | 24px × 24px | 收藏当前路径 |
| 表头高度 | 28px | 列标题行 |
| 文件行高 | 24px | 单行文件项 |
| 文件图标 | 16px | 文件类型图标 |
| 文件名称 | Body 字号 | 左对齐 |
| 文件大小 | Caption 字号 | 右对齐，自动格式化（KB/MB/GB） |
| 修改时间 | Caption 字号 | 右对齐，相对时间（2小时前） |
| 悬停背景 | `DARK_BG_HOVER` / `LIGHT_BG_HOVER` | 鼠标悬停 |
| 选中背景 | `BRAND_PRIMARY` @ 20% | 选中文件 |
| 多选 | `Ctrl+点击` 或 `Shift+点击` | 多选文件 |

##### 8.2.4.3 文件操作交互

- **打开目录**：双击目录项进入
- **返回上级**：点击路径栏左侧"⬆"按钮或 `Backspace`
- **刷新**：`F5` 或点击刷新按钮
- **上传文件**：从本地窗格拖拽到远程窗格，或工具栏"上传"按钮
- **下载文件**：从远程窗格拖拽到本地窗格，或工具栏"下载"按钮
- **删除文件**：选中后按 `Delete` 键，弹出确认对话框
- **重命名**：选中后按 `F2` 键，进入编辑模式
- **右键菜单**：上传、下载、删除、重命名、查看属性、编辑（远程文件）

##### 8.2.4.4 传输队列面板

```
┌─────────────────────────────────────────────────────────────┐
│  传输队列 (3/10)  ↑ 1.2MB/s  ↓ 3.4MB/s              [▲▼] │
├─────────────────────────────────────────────────────────────┤
│  📄 backup.tar.gz    ↑ 45%  ████████░░░░░░░  1.2MB/s  [⏸❌]│
│  📄 config.json      ↓ 完成  ███████████████  完成     [❌]│
│  📁 logs/            ↑ 等待  ░░░░░░░░░░░░░░░  -        [⏸❌]│
└─────────────────────────────────────────────────────────────┘
```

**传输队列规范**：

| 元素 | 尺寸/样式 | 说明 |
|------|-----------|------|
| 队列面板高度 | 120-200px | 可拖拽调整 |
| 任务行高 | 28px | 单个传输任务 |
| 进度条高度 | 8px | 圆角进度条 |
| 进度条颜色 | 品牌色（传输中）、绿色（完成）、红色（失败） | 状态指示 |
| 速度显示 | Caption 字号 | 实时传输速度 |
| 暂停/恢复按钮 | 20px × 20px | 控制传输 |
| 取消按钮 | 20px × 20px | 取消任务 |
| 展开/折叠按钮 | 24px × 24px | 队列面板右上角 |

**传输状态**：

| 状态 | 图标/颜色 | 说明 |
|------|-----------|------|
| 等待中 | 灰色时钟图标 | 排队等待 |
| 传输中 | 品牌色进度条 + 速度 | 正在传输 |
| 已暂停 | 黄色暂停图标 | 用户暂停 |
| 已完成 | 绿色对勾图标 | 传输成功 |
| 失败 | 红色叉号图标 + 错误提示 | 传输失败 |

#### 8.2.5 对话框与模态框

##### 8.2.5.1 会话属性对话框

```
┌─────────────────────────────────────────────────────────────┐
│  会话属性 - web-prod-01                                [×] │
├─────────────────────────────────────────────────────────────┤
│  [基本信息] [连接] [终端] [外观] [安全] [高级]              │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  会话名称:  [web-prod-01        ]                           │
│  主机地址:  [192.168.1.100      ]                           │
│  端口:      [22                  ]                           │
│  协议:      [SSH                ▼]                           │
│                                                              │
│  认证方式:  [密码认证          ▼]                           │
│  用户名:    [root                ]                           │
│  密码:      [••••••••••          ] [显示]                   │
│                                                              │
├─────────────────────────────────────────────────────────────┤
│                              [取消]  [保存]  [连接]         │
└─────────────────────────────────────────────────────────────┘
```

**对话框规范**：

| 元素 | 尺寸/样式 | 说明 |
|------|-----------|------|
| 对话框宽度 | 600-800px | 根据内容自适应 |
| 对话框最小高度 | 400px | 最小高度 |
| 标题栏高度 | 48px | 对话框标题 |
| 标签栏高度 | 40px | 分类标签 |
| 表单标签宽度 | 120px | 左对齐 |
| 输入框宽度 | 剩余空间 | 自动填充 |
| 按钮高度 | 32px | 底部操作按钮 |
| 按钮间距 | 8px | 按钮之间 |

##### 8.2.5.2 确认对话框

```
┌─────────────────────────────────────────┐
│  确认删除                          [×] │
├─────────────────────────────────────────┤
│                                         │
│  ⚠️ 确定要删除会话 "web-prod-01" 吗？  │
│                                         │
│  此操作不可撤销。                       │
│                                         │
├─────────────────────────────────────────┤
│                    [取消]  [删除]       │
└─────────────────────────────────────────┘
```

**确认对话框规范**：

- 宽度：400-500px，高度自适应
- 图标：警告图标（黄色三角）或信息图标（蓝色圆圈）
- 标题：Display 字号，加粗
- 正文：Body 字号，主文本色
- 按钮：主要操作使用品牌色或危险色，次要操作使用默认按钮

---

### 8.3 交互状态与动效规范

#### 8.3.1 组件交互状态

##### 8.3.1.1 按钮状态

| 状态 | 视觉样式 | 说明 |
|------|-----------|------|
| 默认 | 背景色 + 主文本色 | 正常状态 |
| 悬停 | 背景色加深 10% | 鼠标悬停 |
| 按下 | 背景色加深 20% | 鼠标按下 |
| 焦点 | 2px 品牌色外边框 | 键盘焦点 |
| 禁用 | 背景色变浅 + 禁用文本色 | 不可交互 |
| 加载 | 按钮内显示旋转图标 | 操作中 |

##### 8.3.1.2 输入框状态

| 状态 | 视觉样式 | 说明 |
|------|-----------|------|
| 默认 | 边框 `DARK_BORDER` / `LIGHT_BORDER` | 正常状态 |
| 悬停 | 边框加深 20% | 鼠标悬停 |
| 焦点 | 边框变为品牌色 + 2px 外发光 | 输入焦点 |
| 错误 | 边框变为红色 + 错误提示文本 | 输入验证失败 |
| 禁用 | 背景色变浅 + 禁用文本色 | 不可编辑 |
| 只读 | 背景色变浅 + 主文本色 | 可查看不可编辑 |

##### 8.3.1.3 列表项状态

| 状态 | 视觉样式 | 说明 |
|------|-----------|------|
| 默认 | 无背景 | 正常状态 |
| 悬停 | 背景色 `DARK_BG_HOVER` / `LIGHT_BG_HOVER` | 鼠标悬停 |
| 选中 | 背景色 `BRAND_PRIMARY` @ 20% | 当前选中 |
| 选中+悬停 | 背景色 `BRAND_PRIMARY` @ 25% | 选中项悬停 |
| 禁用 | 禁用文本色 | 不可交互 |

##### 8.3.1.4 标签页状态

| 状态 | 视觉样式 | 说明 |
|------|-----------|------|
| 活动 | 底部 2px 品牌色边框 + 主背景色 | 当前标签 |
| 非活动 | 无底部边框 + 加深背景色 | 未选中标签 |
| 悬停 | 背景色加深 5% | 鼠标悬停 |
| 未读 | 标签名称前显示圆点 | 有新输出 |
| 关闭按钮悬停 | 红色圆点背景 | 鼠标悬停关闭按钮 |

#### 8.3.2 过渡动画

##### 8.3.2.1 动画时长

```rust
// 动画时长规范
const ANIMATION_INSTANT: Duration = Duration::from_millis(100);   // 即时反馈
const ANIMATION_FAST: Duration = Duration::from_millis(150);      // 快速过渡
const ANIMATION_NORMAL: Duration = Duration::from_millis(250);    // 标准过渡
const ANIMATION_SLOW: Duration = Duration::from_millis(400);      // 慢速过渡
```

##### 8.3.2.2 动画曲线

```rust
// 动画缓动曲线
const EASING_EASE_OUT: CubicBezier = CubicBezier::new(0.0, 0.0, 0.2, 1.0);  // 淡出
const EASING_EASE_IN: CubicBezier = CubicBezier::new(0.4, 0.0, 1.0, 1.0);   // 淡入
const EASING_EASE_IN_OUT: CubicBezier = CubicBezier::new(0.4, 0.0, 0.2, 1.0); // 淡入淡出
```

##### 8.3.2.3 常见动画场景

| 场景 | 时长 | 曲线 | 说明 |
|------|------|------|------|
| 按钮悬停 | 100ms | EASE_OUT | 背景色过渡 |
| 输入框焦点 | 150ms | EASE_OUT | 边框颜色 + 外发光 |
| 标签切换 | 250ms | EASE_IN_OUT | 内容淡入淡出 |
| 面板展开/折叠 | 250ms | EASE_IN_OUT | 高度过渡 |
| 侧边栏显示/隐藏 | 250ms | EASE_IN_OUT | 宽度过渡 |
| 对话框打开 | 250ms | EASE_OUT | 缩放 + 淡入 |
| 对话框关闭 | 150ms | EASE_IN | 缩放 + 淡出 |
| 下拉菜单打开 | 150ms | EASE_OUT | 淡入 + 向下展开 |
| 下拉菜单关闭 | 100ms | EASE_IN | 淡出 |
| Tooltip 显示 | 200ms 延迟 + 150ms 淡入 | EASE_OUT | 延迟显示 |
| Tooltip 隐藏 | 100ms 淡出 | EASE_IN | 快速消失 |
| 通知弹出 | 400ms | EASE_OUT | 从右侧滑入 |
| 通知消失 | 250ms | EASE_IN | 淡出 |

#### 8.3.3 加载状态

##### 8.3.3.1 加载动画类型

| 类型 | 样式 | 应用场景 |
|------|------|----------|
| 旋转图标 | 16px 圆形旋转动画 | 按钮、标签页 |
| 脉冲圆点 | 3 个圆点依次脉冲 | 对话框、面板 |
| 进度条 | 不确定进度条（来回移动） | 长时间操作 |
| 骨架屏 | 灰色矩形闪烁 | 列表、表格加载 |

##### 8.3.3.2 加载状态规范

- **连接中**：会话项显示黄色圆点 + 旋转动画，状态栏显示"连接中..."
- **文件列表加载**：文件列表区域显示骨架屏，数据到达后淡入替换
- **传输中**：进度条显示确定进度，右侧显示速度和剩余时间
- **按钮操作中**：按钮内显示旋转图标，按钮文本变为"处理中..."
- **页面切换**：新页面内容淡入，旧页面内容淡出

#### 8.3.4 错误提示

##### 8.3.4.1 错误提示类型

| 类型 | 样式 | 持续时间 | 应用场景 |
|------|------|----------|----------|
| 内联错误 | 输入框变红 + 下方红色文本 | 直到修正 | 表单验证失败 |
| Toast 通知 | 右上角弹出，红色背景 | 5 秒自动消失 | 操作失败 |
| 模态对话框 | 居中对话框，警告图标 | 用户确认 | 严重错误 |
| 状态栏提示 | 状态栏文本变红 | 直到解决 | 连接断开 |

##### 8.3.4.2 错误提示规范

- **内联错误**：输入框边框变为红色，下方显示红色错误文本（Caption 字号）
- **Toast 通知**：右上角弹出，红色背景 + 白色文本，5 秒后自动消失，可手动关闭
- **模态对话框**：居中显示，警告图标（黄色三角）或错误图标（红色圆圈），用户必须点击"确定"关闭
- **状态栏提示**：状态栏文本变为红色，显示错误摘要，鼠标悬停显示详细信息

##### 8.3.4.3 常见错误场景

| 场景 | 提示类型 | 提示内容 |
|------|----------|----------|
| 连接失败 | Toast + 状态栏 | "连接失败: 主机不可达" |
| 认证失败 | 模态对话框 | "认证失败: 用户名或密码错误" |
| 主机密钥变更 | 模态对话框 | "警告: 主机密钥已变更，可能遭受中间人攻击" |
| 文件传输失败 | Toast | "传输失败: 权限不足" |
| 表单验证失败 | 内联错误 | "端口号必须在 1-65535 之间" |
| 删除确认 | 模态对话框 | "确定要删除会话 'web-prod-01' 吗？此操作不可撤销" |

#### 8.3.5 快捷键规范

##### 8.3.5.1 全局快捷键

| 快捷键 | 功能 | 说明 |
|--------|------|------|
| `Ctrl+N` | 新建会话 | 打开新建会话对话框 |
| `Ctrl+O` | 打开会话 | 打开会话管理器 |
| `Ctrl+W` | 关闭当前标签 | 关闭当前活动标签 |
| `Ctrl+Shift+W` | 关闭窗口 | 关闭整个应用窗口 |
| `Ctrl+Tab` | 切换到下一个标签 | 循环切换标签 |
| `Ctrl+Shift+Tab` | 切换到上一个标签 | 反向循环切换 |
| `Ctrl+1-9` | 切换到第 N 个标签 | 快速切换 |
| `Ctrl+F` | 搜索 | 打开搜索栏 |
| `Ctrl+Shift+F` | 终端搜索 | 在终端中搜索 |
| `Ctrl+Shift+S` | 显示/隐藏会话管理器 | 切换侧边栏 |
| `Ctrl+Shift+T` | 显示/隐藏工具栏 | 切换工具栏 |
| `Ctrl+Shift+B` | 显示/隐藏状态栏 | 切换状态栏 |
| `Alt+Enter` | 全屏模式 | 进入/退出全屏 |
| `F11` | 全屏模式 | 备用全屏快捷键 |
| `Ctrl+加号` | 放大字体 | 终端字体放大 |
| `Ctrl+减号` | 缩小字体 | 终端字体缩小 |
| `Ctrl+0` | 重置字体大小 | 恢复默认字号 |

##### 8.3.5.2 终端快捷键

| 快捷键 | 功能 | 说明 |
|--------|------|------|
| `Ctrl+Shift+C` | 复制 | 复制选中文本 |
| `Ctrl+Shift+V` | 粘贴 | 粘贴剪贴板内容 |
| `Ctrl+Shift+D` | 复制并粘贴 | 复制选中文本并粘贴 |
| `Shift+PageUp` | 上翻一页 | 滚动历史缓冲区 |
| `Shift+PageDown` | 下翻一页 | 滚动历史缓冲区 |
| `Shift+Home` | 滚动到顶部 | 跳转到历史缓冲区顶部 |
| `Shift+End` | 滚动到底部 | 跳转到历史缓冲区底部 |
| `Alt+拖拽` | 列选择 | 矩形区域选择 |

##### 8.3.5.3 文件管理器快捷键

| 快捷键 | 功能 | 说明 |
|--------|------|------|
| `Enter` | 打开/进入 | 打开选中的目录或文件 |
| `Backspace` | 返回上级 | 返回父目录 |
| `F2` | 重命名 | 重命名选中文件 |
| `Delete` | 删除 | 删除选中文件 |
| `F5` | 刷新 | 刷新文件列表 |
| `Ctrl+A` | 全选 | 选中当前目录所有文件 |
| `Ctrl+C` | 复制 | 复制选中文件路径 |
| `Ctrl+X` | 剪切 | 剪切选中文件 |
| `Ctrl+V` | 粘贴 | 粘贴文件到当前目录 |

---

### 8.4 可复用 UI 组件库

#### 8.4.1 组件库设计原则

- **一致性**：所有组件遵循统一的视觉语言（色彩、字体、间距、圆角）
- **可组合**：组件可自由组合，构建复杂界面
- **可访问**：支持键盘导航、屏幕阅读器、高对比度模式
- **可主题化**：组件样式通过主题变量控制，支持动态切换主题
- **高性能**：列表、表格等组件使用虚拟化渲染，支持大数据集

#### 8.4.2 基础组件

##### 8.4.2.1 按钮 (Button)

```rust
// 按钮组件定义
struct Button {
    label: String,
    variant: ButtonVariant,
    size: ButtonSize,
    icon: Option<Icon>,
    disabled: bool,
    loading: bool,
    on_click: Callback<()>,
}

enum ButtonVariant {
    Primary,    // 品牌色背景，白色文本
    Secondary,  // 默认背景，主文本色
    Danger,     // 红色背景，白色文本
    Ghost,      // 透明背景，主文本色，悬停显示背景
    Link,       // 无背景，品牌色文本，下划线
}

enum ButtonSize {
    Small,      // 28px 高
    Medium,     // 32px 高
    Large,      // 36px 高
}
```

**按钮使用规范**：

- 主要操作使用 `Primary` 变体（如"保存"、"连接"）
- 危险操作使用 `Danger` 变体（如"删除"、"断开"）
- 次要操作使用 `Secondary` 变体（如"取消"、"关闭"）
- 工具栏按钮使用 `Ghost` 变体
- 文本链接使用 `Link` 变体

##### 8.4.2.2 输入框 (TextInput)

```rust
// 输入框组件定义
struct TextInput {
    value: String,
    placeholder: String,
    input_type: InputType,
    disabled: bool,
    readonly: bool,
    error: Option<String>,
    on_change: Callback<String>,
    on_submit: Callback<()>,
}

enum InputType {
    Text,           // 普通文本
    Password,       // 密码（隐藏字符）
    Number,         // 数字（带上下箭头）
    Search,         // 搜索（带搜索图标）
    Multiline,      // 多行文本（TextArea）
}
```

**输入框规范**：

- 高度：28px（小）、32px（中）
- 边框：1px，圆角 6px
- 内边距：左右 12px
- 占位符文本：次要文本色
- 焦点状态：边框变为品牌色 + 2px 外发光
- 错误状态：边框变为红色 + 下方显示红色错误文本

##### 8.4.2.3 下拉选择 (Dropdown)

```rust
// 下拉选择组件定义
struct Dropdown<T> {
    value: T,
    options: Vec<DropdownOption<T>>,
    placeholder: String,
    disabled: bool,
    searchable: bool,
    on_change: Callback<T>,
}

struct DropdownOption<T> {
    value: T,
    label: String,
    icon: Option<Icon>,
    disabled: bool,
}
```

**下拉选择规范**：

- 高度：28px（小）、32px（中）
- 边框：1px，圆角 6px
- 下拉菜单：最大高度 300px，超出显示滚动条
- 搜索型下拉：支持输入过滤选项
- 键盘导航：`上/下` 选择选项，`Enter` 确认，`Esc` 关闭

##### 8.4.2.4 复选框 (Checkbox)

```rust
// 复选框组件定义
struct Checkbox {
    checked: bool,
    label: String,
    disabled: bool,
    on_change: Callback<bool>,
}
```

**复选框规范**：

- 尺寸：16px × 16px
- 未选中：边框 1px，圆角 3px
- 选中：品牌色背景 + 白色对勾
- 悬停：边框加深
- 焦点：2px 品牌色外边框

##### 8.4.2.5 单选按钮 (Radio)

```rust
// 单选按钮组件定义
struct Radio<T> {
    value: T,
    label: String,
    selected: bool,
    disabled: bool,
    on_change: Callback<T>,
}
```

**单选按钮规范**：

- 尺寸：16px × 16px，圆形
- 未选中：边框 1px
- 选中：品牌色外圈 + 内部圆点
- 悬停：边框加深

##### 8.4.2.6 开关 (Toggle)

```rust
// 开关组件定义
struct Toggle {
    enabled: bool,
    label: String,
    disabled: bool,
    on_change: Callback<bool>,
}
```

**开关规范**：

- 尺寸：36px × 20px
- 未启用：灰色背景，圆点在左侧
- 启用：品牌色背景，圆点在右侧
- 切换动画：200ms，EASE_IN_OUT

#### 8.4.3 数据展示组件

##### 8.4.3.1 表格 (Table)

```rust
// 表格组件定义（虚拟化渲染）
struct Table<T> {
    columns: Vec<TableColumn>,
    rows: Vec<T>,
    row_height: f32,
    selectable: bool,
    sortable: bool,
    on_row_click: Callback<usize>,
    on_row_double_click: Callback<usize>,
    on_sort: Callback<(String, SortDirection)>,
}

struct TableColumn {
    id: String,
    title: String,
    width: ColumnWidth,
    sortable: bool,
    align: TextAlign,
}

enum ColumnWidth {
    Fixed(f32),
    Flex(f32),  // 弹性宽度，按比例分配
}
```

**表格规范**：

- 表头高度：28px，背景色加深 5%
- 行高：24px（紧凑）、28px（标准）、32px（宽松）
- 单元格内边距：左右 12px
- 列宽：可拖拽调整，最小宽度 60px
- 排序：点击表头排序，显示排序箭头
- 选中：行背景变为品牌色 @ 20%
- 悬停：行背景变为 `DARK_BG_HOVER` / `LIGHT_BG_HOVER`
- 虚拟化：仅渲染可见行，支持 10 万行数据

##### 8.4.3.2 树形视图 (TreeView)

```rust
// 树形视图组件定义
struct TreeView<T> {
    nodes: Vec<TreeNode<T>>,
    selectable: bool,
    expandable: bool,
    on_select: Callback<usize>,
    on_expand: Callback<usize>,
    on_collapse: Callback<usize>,
}

struct TreeNode<T> {
    id: usize,
    label: String,
    icon: Option<Icon>,
    children: Vec<TreeNode<T>>,
    expanded: bool,
    selected: bool,
    data: T,
}
```

**树形视图规范**：

- 节点高度：28px
- 缩进：每级 16px
- 展开图标：8px 箭头，旋转动画
- 选中：行背景变为品牌色 @ 20%
- 悬停：行背景变为 `DARK_BG_HOVER` / `LIGHT_BG_HOVER`
- 键盘导航：`上/下` 移动，`左/右` 折叠/展开，`Enter` 选中

##### 8.4.3.3 列表 (List)

```rust
// 列表组件定义（虚拟化渲染）
struct List<T> {
    items: Vec<ListItem<T>>,
    item_height: f32,
    selectable: bool,
    on_click: Callback<usize>,
}

struct ListItem<T> {
    label: String,
    icon: Option<Icon>,
    description: Option<String>,
    data: T,
}
```

**列表规范**：

- 项高度：28px（单行）、48px（双行）
- 图标尺寸：16px
- 内边距：左右 12px
- 选中：背景色 `BRAND_PRIMARY` @ 20%
- 悬停：背景色 `DARK_BG_HOVER` / `LIGHT_BG_HOVER`

#### 8.4.4 反馈组件

##### 8.4.4.1 进度条 (Progress)

```rust
// 进度条组件定义
struct Progress {
    value: f32,           // 0.0 - 1.0
    variant: ProgressVariant,
    show_label: bool,
    indeterminate: bool,
}

enum ProgressVariant {
    Default,    // 品牌色
    Success,    // 绿色
    Warning,    // 黄色
    Error,      // 红色
}
```

**进度条规范**：

- 高度：8px
- 圆角：4px
- 背景色：`DARK_BG_SURFACE` / `LIGHT_BG_SURFACE`
- 进度色：根据 variant 决定
- 不确定进度：来回移动的动画条
- 标签：右侧显示百分比（可选）

##### 8.4.4.2 通知 (Notification)

```rust
// 通知组件定义
struct Notification {
    message: String,
    variant: NotificationVariant,
    duration: Duration,
    closable: bool,
}

enum NotificationVariant {
    Info,       // 蓝色图标
    Success,    // 绿色图标
    Warning,    // 黄色图标
    Error,      // 红色图标
}
```

**通知规范**：

- 位置：右上角，距离顶部 16px，距离右侧 16px
- 宽度：300-400px
- 高度：自适应
- 背景色：根据 variant 决定（浅色背景 + 深色文本）
- 图标：左侧 20px 图标
- 关闭按钮：右上角 12px × 12px
- 动画：从右侧滑入，淡出消失
- 持续时间：默认 5 秒，可配置

##### 8.4.4.3 Tooltip (Tooltip)

```rust
// Tooltip 组件定义
struct Tooltip {
    content: String,
    position: TooltipPosition,
    delay: Duration,
}

enum TooltipPosition {
    Top,
    Bottom,
    Left,
    Right,
}
```

**Tooltip 规范**：

- 背景色：`DARK_BG_ELEVATED` / `LIGHT_BG_ELEVATED`
- 文本色：主文本色
- 内边距：8px 12px
- 圆角：4px
- 阴影：SHADOW_MD
- 延迟显示：200ms
- 动画：150ms 淡入

##### 8.4.4.4 模态对话框 (Modal)

```rust
// 模态对话框组件定义
struct Modal {
    title: String,
    content: Box<dyn View>,
    footer: Vec<Button>,
    width: f32,
    closable: bool,
    on_close: Callback<()>,
}
```

**模态对话框规范**：

- 背景遮罩：黑色 @ 50% 透明度
- 对话框宽度：400-800px，根据内容自适应
- 对话框圆角：8px
- 阴影：SHADOW_LG
- 标题栏：48px 高，Display 字号，加粗
- 内容区：内边距 24px
- 底部按钮区：内边距 16px 24px，按钮右对齐
- 动画：打开时缩放 + 淡入（250ms），关闭时缩放 + 淡出（150ms）

#### 8.4.5 导航组件

##### 8.4.5.1 标签页 (Tabs)

```rust
// 标签页组件定义
struct Tabs {
    tabs: Vec<Tab>,
    active_tab: usize,
    on_change: Callback<usize>,
    closable: bool,
}

struct Tab {
    label: String,
    icon: Option<Icon>,
    content: Box<dyn View>,
    closable: bool,
}
```

**标签页规范**：

- 标签高度：36px
- 标签最小宽度：120px
- 标签最大宽度：200px
- 活动标签：底部 2px 品牌色边框
- 非活动标签：背景色加深 5%
- 关闭按钮：12px × 12px，悬停显示
- 新建标签按钮：标签栏右侧"+"按钮

##### 8.4.5.2 面包屑 (Breadcrumb)

```rust
// 面包屑组件定义
struct Breadcrumb {
    items: Vec<BreadcrumbItem>,
    on_click: Callback<usize>,
}

struct BreadcrumbItem {
    label: String,
    icon: Option<Icon>,
}
```

**面包屑规范**：

- 高度：28px
- 项间距：8px
- 分隔符："/" 或 ">"，次要文本色
- 当前项：主文本色，不可点击
- 其他项：次要文本色，悬停变为主文本色，可点击

#### 8.4.6 组件库使用规范

##### 8.4.6.1 组件导入

```rust
// 统一从 ui-common crate 导入组件
use rshell_ui_common::components::{
    Button, TextInput, Dropdown, Checkbox, Radio, Toggle,
    Table, TreeView, List,
    Progress, Notification, Tooltip, Modal,
    Tabs, Breadcrumb,
};
```

##### 8.4.6.2 组件主题化

```rust
// 组件样式通过主题变量控制
struct ComponentTheme {
    button: ButtonTheme,
    input: InputTheme,
    table: TableTheme,
    // ... 其他组件主题
}

// 主题变量示例
struct ButtonTheme {
    primary_bg: Rgba,
    primary_fg: Rgba,
    primary_hover_bg: Rgba,
    primary_active_bg: Rgba,
    border_radius: f32,
    // ... 更多变量
}
```

**主题化规范**：

- 所有组件样式通过主题变量控制，不硬编码颜色值
- 主题切换时，所有组件自动更新样式
- 支持自定义主题（用户或插件提供）
- 主题变量命名规范：`组件名_属性_状态`（如 `button_primary_hover_bg`）

##### 8.4.6.3 组件可访问性

- 所有交互组件支持键盘导航（`Tab` 键切换焦点）
- 按钮、链接等可交互元素显示焦点边框
- 图标按钮提供 `aria-label` 或 Tooltip
- 表单组件关联 `<label>` 标签
- 颜色对比度符合 WCAG 2.1 AA 标准（4.5:1）

---

> **文档维护**：本文档随项目演进持续更新，各模块详细设计在开发阶段可能调整。