# Tauri 2.0 架构下 RShell 前后端功能划分设计

> **文档版本**：v1.0
> **编写日期**：2026-07-31
> **基线 commit**：`6d2aac9`（`chore(docs): update CLAUDE.md, build scripts, .gitignore for Tauri`）
> **关联文档**：`docs/03-detailed-design.md`、`docs/05-development-standards.md`、`docs/06-test-strategy.md`、`docs/08-incomplete-features.md`
> **状态**：设计已确认，尚未开始实施

---

## 0. 背景与本文档的范围

RShell 正在从 GPUI 迁移到 Tauri 2.0。截至基线 commit，迁移已完成的部分：

| 已完成 | 证据 |
|--------|------|
| Rust workspace 迁入 `src-tauri/` | `src-tauri/Cargo.toml` 为 workspace 根，根 `Cargo.toml` 已删除 |
| GPUI 前端已删除 | `crates/rshell-ui/`（4671 行）不再存在 |
| Tauri 壳骨架 | `src-tauri/src/{main,lib}.rs`，注册了三个官方插件 |
| React 前端脚手架 | `src/`（27 个文件）+ `package.json` + `vite.config.ts` |
| TS 类型手工镜像 | `src/ipc/types.ts`（547 行） |
| invoke 包装层 | `src/ipc/client.ts`（208 行，覆盖全部命令） |
| 事件订阅层 | `src/ipc/events.ts`（160 行，17 个事件分支） |

**尚未完成的核心工作**：`src-tauri/src/lib.rs` 仍只有一个占位 `hello` 命令（`lib.rs:21,39`），没有 `AppState`、没有 `CommandDispatcher` 接入、没有任何真实命令、没有 emit 桥、没有 Channel。前端 `TerminalPanel.tsx:54-62` 写死了假的 `ls -la` 输出。

**换句话说：两端的壳都立起来了，中间的桥一根没搭。**

本文档定义这座桥的形状，以及桥两端各自承担什么职责。不含实施代码。

### 0.1 划分总原则

> 凡是"用户看到的样子"归前端；凡是"系统真实状态 + OS/网络/磁盘/密码学"归后端。

可操作判据：**断电重启后该状态是否必须还在？**
- 必须还在 → 后端（会话配置、密钥、known_hosts、传输队列、主题名）
- 不必须 → 前端（终端屏幕内容、排序、展开态、滚动位置、搜索词）

### 0.2 已确认的六项关键决策

| # | 决策 | 理由摘要 |
|---|------|----------|
| D1 | 终端渲染由前端 xterm.js 全接管 | 生态成熟、IPC 量最小；后端 alacritty 缓冲净删除（见 §2） |
| D2 | 细粒度 `#[tauri::command]`（非单一 dispatch） | 请求/响应天然对应，可用 capabilities 细粒度授权 |
| D3 | 保留 `CommandDispatcher`，Tauri 命令做薄壳 | core 可脱离 Tauri 独立集成测试，未来可做 CLI/headless |
| D4 | `dispatch` 返回 `CommandOutcome` 枚举 | 修掉现存死循环（见 §3.2），读命令直接拿数据 |
| D5 | 开 `rhai` 的 `sync` feature，dispatcher 直接当 `State` | 代价近零（见 §1.2），换来真并发 |
| D6 | 终端字节走 `Channel`，其余低频事件走统一 `emit` | 高频路径专用通道，不污染全局事件总线 |

---

## 1. 架构分层

### 1.1 三层职责

```
┌─ 前端 (React + TS, 仓库根 src/) ─────────────────────────┐
│  渲染 · 交互 · 呈现态                                      │
│  · xterm.js：网格 / 选区 / 搜索 / 滚动回看                  │
│  · 布局：标签页 / 分屏 / 面板尺寸（localStorage 持久化）      │
│  · zustand：后端快照 + 本地 UI 态（排序 / 过滤 / 展开折叠）    │
│  · 剪贴板读写 · 快捷键分发 · scheme → ITheme/CSS 变量映射    │
└──────────────────────────┬───────────────────────────────┘
        invoke(细粒度命令)   │   Channel<Vec<u8>>（每 session 一条）
        ↕                   │   emit("rshell://event")（低频事件）
┌──────────────────────────┴───────────────────────────────┐
│  src-tauri/src/  ← Tauri 壳（薄，无业务逻辑）               │
│  · state.rs      AppState { Arc<CommandDispatcher> }      │
│  · commands/     命令薄壳 → dispatch(AppCommand)           │
│  · events.rs     EventBus.subscribe → app.emit            │
│  · terminal.rs   Channel 注册表 (session_id → TermSink)    │
│  · error.rs      CoreError → IpcError 映射                 │
└──────────────────────────┬───────────────────────────────┘
              AppCommand / CommandOutcome / AppEvent
┌──────────────────────────┴───────────────────────────────┐
│  src-tauri/crates/  ← 后端（业务逻辑，几乎不动）            │
│  rshell-api        契约层（零运行时依赖）                   │
│  rshell-core       session / transfer / security /         │
│                    script / theme / event_bus / dispatcher │
│  rshell-protocol   SSH / Telnet / Serial / RDP            │
│  rshell-infra      crypto / storage / PTY                 │
│  rshell-plugin-sdk 插件加载 / WASM sandbox（scaffold）      │
└──────────────────────────────────────────────────────────┘
```

### 1.2 线程模型：开 rhai `sync` feature（D5）

**现状**：`rhai::Engine` 是 `!Send + !Sync`，GPUI 时代靠"专用后台 OS 线程 + 单线程 tokio runtime + `LocalSet`"绕开。`command_dispatcher.rs:99` 留有 `#[allow(clippy::arc_with_non_send_sync)]` 作为疤痕。

**问题**：Tauri 的 `State<T>` 要求 `Send + Sync`，这套绕法不能原样搬。

**决策**：`rhai = { version = "1", features = ["sync"] }`，`Arc<CommandDispatcher>` 直接放进 `app.manage()`，使用 Tauri 自带的多线程 tokio runtime。

**代价评估（这是选它的关键）**：

```
$ grep -rn "Rc<|RefCell|Cell<" src-tauri/crates/{rshell-core,rshell-protocol,rshell-infra,rshell-plugin-sdk}/src
(无输出)
```

- 后端**除 `rhai::Engine` 外无任何非 `Send` 类型**；所有 service 都是 `Arc<T>`，`EventBus` 的 `Subscriber` 已是 `Box<dyn Fn + Send + Sync>`（`event_bus.rs:14`）
- `ScriptEngine` 注册的 host 函数只有 6 个（`rshell_log` / `rshell_log_level` / `rshell_sleep` / `rshell_now_ms` / `rshell_uuid_v4` / `rshell_parse_uuid` / `rshell_version_compare`），**全部是纯函数，无一捕获共享可变状态**
- rhai 官方文档指出 `sync` 的主要迁移痛点是"注册函数捕获了 `Rc`/`RefCell`/MPSC 需改成 `Arc`/`Mutex`"——本项目一个都不涉及
- `sync` 使 `Dynamic` 内部由 `Rc` 换为 `Arc`：若将来给脚本注册带状态的 host 函数（如 `rshell_send` 需持 `Arc<SessionService>`），`Arc` 本就是必需的，反而更顺

**附带收益**：`docs/08` #2 记录的隐患"russh 的 `block_on(rx)` 在单线程 runtime 上行为待确认"，因迁移到多线程 runtime 而消失。但仍需在切片 1 实测。

**被否决的备选**：

| 备选 | 否决理由 |
|------|----------|
| 保留专用后台线程 + `oneshot` 回注 | **全局串行**：所有命令挤一个单线程 runtime，一次 SFTP 大传输或 SSH 握手超时会阻住终端输入，等于把 Tauri 的并发 IPC 白白降级 |
| `ScriptEngine` 独立 actor 线程 | 为躲一个 feature flag 引入额外 mpsc + 生命周期管理 + 独立错误回传路径，复杂度换来的收益不可测量 |

**降级路径**：若 `sync` 编译不通过（切片 0 会在数小时内证伪），退回 "ScriptEngine 独立 actor" 方案。

### 1.3 前后端边界铁律

沿用 `docs/05-development-standards.md` §2 的精神，在 Tauri 下重述：

1. `src-tauri/crates/**` 不得依赖 `tauri` crate（`rshell-api` 继续保持零运行时依赖，仅 serde + uuid）
2. `src/**`（前端）不得假设后端内部结构，唯一入口是 `invoke()` / `listen()` / `Channel`
3. `src-tauri/src/**`（壳）不得含业务逻辑，只做：参数转换、`AppCommand` 构造、`CommandOutcome` 解包、错误映射、Channel 路由
4. 任何状态只有**一个**所有者（§4.2）

---

## 2. 终端：alacritty 缓冲区净删除

### 2.1 关键发现

`session/service.rs:169-253` 的 recv 循环当前同时做四件事：

```rust
Ok(Some(data)) => {
    event_bus.publish(AppEvent::TerminalOutput { session_id, data: data.clone() });  // ① 原始字节
    terminal_service.process_output(session_id, &data);                              // ② 喂 alacritty
    dirty = true;                                                                    // ④ 标记待 flush
    if let Ok(text) = str::from_utf8(&data) {
        trigger_engine.check_output(text, session_id)                                 // ③ 触发器匹配
    }
}
// 另有 16ms ticker：dirty 时 get_buffer_snapshot() → publish TerminalBufferUpdated  // ④
```

**决定性事实**：③ 触发器吃的是 `str::from_utf8(&data)` 的**原始字节**，完全没有使用 alacritty 的缓冲区（`trigger_engine.rs:89` `check_output(&self, output: &str, _session_id: Uuid)`）。

因此 D1（xterm.js 全接管）落地后，**不需要保留任何后端影子缓冲**——② 和 ④ 是净删除，触发器功能零影响。

### 2.2 删除清单

| 对象 | 位置 | 处置 |
|------|------|------|
| alacritty 网格实现 | `core/terminal/buffer.rs` | 删除 |
| `TerminalService`（328 行） | `core/terminal/service.rs` | 瘦身至 ~60 行：仅保留 `create_terminal` / `destroy_terminal` 生命周期 + `resize` 记录尺寸 |
| `TerminalService::send_input` | `service.rs:286-301` | 删除（已标 `#[deprecated]`，实现是 publish `TerminalOutput` 造成回显死循环） |
| `TerminalService::process_output` | `service.rs:265` | 删除 |
| `TerminalService::get_buffer_snapshot` | `service.rs:320` | 删除 |
| 60Hz ticker + `dirty` 标志 | `session/service.rs:170-192` | 删除（Channel 直推，无需节流） |
| `AppEvent::TerminalBufferUpdated` | `api/events.rs:32` | 删除 |
| `TerminalBufferSnapshot` 及 Cell/CellFlags 系列类型 | `api/types.rs` | 删除 |
| `alacritty_terminal` 依赖 | `core/Cargo.toml` | 移除 |
| 前端 `onTerminalBufferUpdated` 分支 | `src/ipc/events.ts:42,85-89` | 删除 |
| 前端假输出 | `src/views/TerminalPanel.tsx:54-62` | 替换为真实 Channel 接入 |

### 2.3 简化后的 recv 循环

```rust
Ok(Some(data)) => {
    terminal_channels.push(session_id, &data);          // → xterm.js（Channel，非 emit）
    if let Ok(text) = str::from_utf8(&data) {
        trigger_engine.check_output(text, session_id)?   // 原样保留
    }
}
```

### 2.4 触发器 SendText 的顺带修正

`session/service.rs:227-232` 现在把 `TriggerAction::SendText` 伪装成 `TerminalOutput` 推回前端做假回显（注释自述"本轮先 echo"）。改为直接调用 `SessionService::send_data` 真正发往远端。

---

## 3. IPC 契约

### 3.1 三条通道

| 通道 | 承载 | 频率 | 数量 |
|------|------|------|------|
| `invoke(细粒度命令)` | 全部用户意图 + 读查询 | 用户操作级 | 56 个命令 |
| `Channel<Vec<u8>>` | 终端字节流，每 session 一条 | 高频（可达数千次/秒） | 每活动会话 1 条 |
| `emit("rshell://event")` | 低频状态广播 | 状态变化级 | ~33 个事件 |

**契约规模复核**（基于基线 commit 实测）：

```
$ grep -cE "^    [A-Z][A-Za-z]+( \{|,)" src-tauri/crates/rshell-api/src/{commands,events}.rs
commands.rs:56
events.rs:42
```

`AppCommand` 56 个变体、`AppEvent` 42 个变体。事件在本设计下降至约 33 个（§3.3）。

前端事件通道名已在 `src/ipc/events.ts:10` 固定为 `"rshell://event"`，后端 emit 时须与之一致。

### 3.2 `CommandOutcome`：修掉一个现存死循环

**Bug 现场**（`command_dispatcher.rs:420-435`）：

```rust
AppCommand::ListTriggers => {
    let triggers = self.trigger_engine.list_triggers()?;
    let _ = triggers;                                        // ← 算出来直接丢弃
    self.event_bus.publish(AppEvent::TriggerListChanged);     // ← 只喊"变了，你再查"
}
AppCommand::ListQuickCommands => {
    let cmds = self.quick_command_service.list_commands()?;
    let _ = cmds;                                            // ← 同样丢弃
    self.event_bus.publish(AppEvent::QuickCommandListChanged);
}
```

前端收到 `TriggerListChanged` → 发 `ListTriggers` → 又收到 `TriggerListChanged` → …… **数据永远到不了前端**。源码注释里那段自述（"实际 list 结果通过单独事件分发"）是未写完的 TODO。

D4 因此不是重构洁癖，而是修 bug。

**新增契约类型**（置于 `rshell-api`）：

```rust
pub enum CommandOutcome {
    None,                                    // 全部写操作
    Sessions(Vec<SessionConfig>),
    Keys(Vec<SshKeyInfo>),
    Tunnels(Vec<ActiveTunnelInfo>),
    Plugins(Vec<PluginInfo>),
    Triggers(Vec<Trigger>),                  // ← 修复点
    QuickCommands(Vec<QuickCommand>),        // ← 修复点
    Themes {
        current_theme: String,
        current_scheme: String,
        available_themes: Vec<String>,
        available_schemes: Vec<String>,
    },
    PendingTunnels(Vec<(Uuid, PortForwardRule)>),
    RemoteDir { path: String, entries: Vec<RemoteFileEntry> },
    PublicKey(String),
    SessionId(Uuid),                         // CreateSession 现返回值被丢弃
    Verified(bool),                          // VerifyMasterPassword
}
```

`CommandDispatcher::dispatch` 签名改为 `async fn dispatch(&self, cmd: AppCommand) -> Result<CommandOutcome, CoreError>`；所有写操作分支返回 `Ok(CommandOutcome::None)`。

### 3.3 可删除的事件与语义收紧

**删除**（9 个）：

- 7 个纯拉取响应：`SessionsSnapshot`、`KeysSnapshot`、`TunnelsSnapshot`、`PluginsSnapshot`、`ThemesSnapshot`、`PendingTunnelsSnapshot`、`RemoteDirListed` —— 改由 `CommandOutcome` 直接返回
- `TerminalBufferUpdated` —— §2.2
- `ClipboardCopy` —— §5（上移前端）

`AppEvent` 由 42 降至约 33。前端 `src/ipc/events.ts` 中对应的 7 个 `on*Snapshot` 分支（`events.ts:44-53,95-119`）与 `onRemoteDirListed`、`onClipboardCopy` 一并删除。

**语义收紧**：保留的 `*Changed` 事件只做**失效通知**，不再兼任数据搬运：

```
TriggerListChanged / QuickCommandListChanged / SessionListChanged
  → 前端 store 标记 stale，下次读取时重新 invoke
```

这条规则消除了"事件既通知又带数据"的二义性，也是死循环的根因治理。

### 3.4 命令薄壳的统一形状

56 个薄壳用宏消除样板：

```rust
macro_rules! cmd {
    ($name:ident($($arg:ident: $ty:ty),*) -> $out:ident($ret:ty) = $variant:expr) => {
        #[tauri::command]
        pub async fn $name(
            $($arg: $ty,)* state: State<'_, AppState>
        ) -> Result<$ret, IpcError> {
            match state.dispatcher.dispatch($variant).await? {
                CommandOutcome::$out(v) => Ok(v),
                other => Err(IpcError::outcome_mismatch(stringify!($out), other.kind())),
            }
        }
    };
}
```

`OutcomeMismatch` 显式兜住理论不可达分支——它只在 dispatcher 分支写错时触发，相当于运行时断言，比 `unreachable!()` 安全（`docs/05` 禁用 `unwrap`/`expect`）。

**命名约定**：前端 `src/ipc/client.ts:39-45` 已实现 `PascalCase → snake_case` 转换（`ConnectSession` → `connect_session`）。后端命令函数名必须严格遵循此映射。

### 3.5 错误跨 IPC

`CoreError` 不直接过 IPC（含 `anyhow::Error`、`io::Error` 等不可序列化内容）。新增：

```rust
#[derive(Debug, Serialize)]
pub struct IpcError {
    pub kind: String,              // 稳定的机器可读判别串
    pub message: String,           // 仅用于展示
    pub session_id: Option<Uuid>,
}
```

`kind` 取值集合固定：`"not_found"` / `"auth_failed"` / `"host_key_mismatch"` / `"connection"` / `"io"` / `"permission"` / `"outcome_mismatch"` / `"internal"`。前端按 `kind` 分支处理，`message` 只做展示。

> 注：`CLAUDE.md:101` 目前写的是 "Tauri commands return `Result<T, String>`"。本设计以 `IpcError` 取代裸 `String`，实施时需同步更新 CLAUDE.md。

### 3.6 类型同步：从手工镜像改为自动生成

**现状风险**：`src/ipc/types.ts`（547 行）是 `rshell-api` 的**手工镜像**。Rust 侧改了类型而 TS 侧忘记同步，编译期无任何保护——这是当前架构最脆弱的一环。

**决策**：为 `rshell-api` 的类型加 `#[derive(ts_rs::TS)]`，`cargo test` 时导出到 `src/ipc/generated.ts`；CI 增加 `git diff --exit-code src/ipc/generated.ts`。Rust 改类型忘了重新生成会直接挂 CI。

迁移路径：先生成 `generated.ts` 与现有 `types.ts` 逐一比对（暴露既存偏差），确认一致后 `types.ts` 改为从 `generated.ts` 重导出。

---

## 4. 数据流与状态所有权

### 4.1 Channel 生命周期（新增设计）

**时序缺口**：recv 循环在 `connect()` 内 `tokio::spawn` 启动（`session/service.rs:169`），而 Channel 由前端在组件 mount 时创建。两者时序无保证：

- `attach_terminal` 晚于 `connect` → 中间到达的字节丢失（少显示 banner / 首个提示符）
- 前端先 attach 也不行 → 那时 session 尚未连接

**解法**：后端持每会话双态 sink，未 attach 期间缓冲。

```rust
// src-tauri/src/terminal.rs
pub struct TerminalChannels {
    inner: RwLock<HashMap<Uuid, TermSink>>,
}

enum TermSink {
    Buffering(VecDeque<u8>),      // 上限 256 KiB；溢出则丢弃最旧字节并 warn!
    Attached(Channel<Vec<u8>>),
}
```

行为规则：

1. recv 循环无条件写入 `TerminalChannels`，不关心前端是否已 attach
2. `attach_terminal(session_id, ch)` 将 `Buffering` 积压一次性 flush 进 `ch`，随后转为 `Attached`
3. `ch.send()` 返回 `Err`（窗口重载 / dev HMR 导致 Channel 失效）时，退回 `Buffering` 等待重新 attach，**不断开 SSH 连接**

规则 3 顺带解决 dev 体验：Vite HMR 热更新不会踢掉 SSH 连接。

**可选简化**：若接受"attach 慢了就丢首屏字节"，可退化为 `Option<Channel>`。本设计选择保留双态，因为首屏提示符丢失对终端类应用是明显缺陷。

### 4.2 状态所有权表

| 状态 | 唯一所有者 | 前端获取方式 | 断电后 |
|------|-----------|-------------|--------|
| 会话配置 | 后端 `SessionRepository`（TOML） | `list_sessions()` | 保留 |
| 连接状态机 | 后端 `SessionService.sessions` | `ConnectionStateChanged` | 丢失（应然） |
| **终端屏幕内容** | **前端 xterm.js** | 自行维护 | 丢失（应然） |
| 终端尺寸 | 双方（**前端权威**） | 前端 fit → `resize_terminal` | 丢失 |
| 传输队列 | 后端 `TransferService` | `list_transfers()` + `TransferProgress` | 保留 |
| 私钥 / 主密码 | 后端（ring 加密） | 只出 `SshKeyInfo`，**私钥永不过 IPC** | 保留 |
| known_hosts | 后端 `HostKeyManager` | — | 保留 |
| 隧道 listener | 后端 `TunnelManager` | `list_tunnels()` | 规则保留、listener 丢失 |
| 当前主题名 / 配色名 | 后端 `ThemeManager` | `list_themes()` | 保留 |
| **主题颜色 → CSS 变量** | **前端** | 由 scheme 计算 | 丢失（应然） |
| **布局 / 分屏 / 标签顺序** | **前端** localStorage | — | 保留（前端自管） |
| **排序 / 过滤 / 展开折叠** | **前端** zustand | — | 丢失（应然） |

**铁律**：任何状态只有一个所有者。终端尺寸是唯一双写项，处理方式为"前端权威、后端仅记录并转发给 PTY/SSH"，冲突时以前端最后一次 `resize` 为准。

### 4.3 三个代表性流程

**A. 连接并出字**

```
用户点击会话
  → invoke('connect_session', { session_id })
      → dispatch(ConnectSession) → SessionService::connect
          → 协议栈握手；成功后 spawn recv 循环 → 写 TerminalChannels
      → emit ConnectionStateChanged { Connected }
前端收到 Connected
  → 挂载 xterm → invoke('attach_terminal', { session_id, onData: ch })
      → flush 积压 → 转 Attached
  → 字节持续流入 ch.onmessage → term.write()
用户敲键 → term.onData → invoke('send_input', { session_id, data })
```

**B. Host key 决策**（修 `docs/08` #2，当前链路断裂）

```
握手中 → HostKeyDecisionRegistry 登记 decision_id + oneshot
  → emit HostKeyMismatch { decision_id, expected, received, public_key_blob }
前端弹对话框（信任一次 / 永久信任 / 拒绝）
  → invoke('decide_host_key', { decision_id, accept, permanent })
      → registry 取出 oneshot → send → 握手线程解除阻塞
```

前端 `src/ipc/client.ts:171` 的 `decideHostKey` 与 `src/ipc/events.ts:145-149` 的 `onHostKeyMismatch` 已就位，缺后端侧实现。

**C. 触发器**（保持在后端）

```
recv 循环 → check_output(原始字节) → 命中
  → emit TriggerFired { trigger_id, session_id, action_summary }
  → SendText 动作：直接调用 SessionService::send_data（不再假回显，见 §2.4）
```

触发器必须留在后端，因为它需要扫描**全部**字节流；前端只能看到 xterm 渲染后的结果。

### 4.4 错误与失败路径

| 失败场景 | 表现 | 处理 |
|---------|------|------|
| 连接失败 | `invoke` reject + `IpcError { kind: "connection" }` | 前端 toast，会话行标红 |
| 连接中途断开 | `emit ConnectionStateChanged { Disconnected }` | xterm 显示 `[连接已断开]`，**保留屏幕内容不清屏** |
| Channel 失效（窗口重载） | `ch.send()` 返回 `Err` | 退回 `Buffering`，**不断开 SSH** |
| 积压溢出 256 KiB | 丢弃最旧字节 + `warn!` | 前端 attach 后首屏可能缺一段，可接受 |
| 后端 service panic | Tauri 进程存活但对应 service 中毒 | `RwLock` 一律用 `map_err` 而非 `unwrap`（现有代码已合规） |
| 前端崩溃 / 重载 | 后端连接仍在 | 重载后重新 attach 即恢复 |

最后一行是本设计的附带收益：**前后端故障域隔离**——前端崩溃不影响 SSH 连接存活。

---

## 5. 灰色地带的归属裁定

以下能力现在都在后端，但在 web 前端中由前端承担更自然。已确认全部上移：

| 能力 | 上移理由 | 契约变更 |
|------|---------|---------|
| 剪贴板 / 复制选区 | xterm.js 自持选区，后端无需知道选区存在 | 删除 `AppCommand::CopySelection`、`AppEvent::ClipboardCopy`；前端直接 `navigator.clipboard` 或 `tauri-plugin-clipboard-manager` |
| 主题 / 配色的渲染映射 | 颜色值 → CSS 变量 / xterm `ITheme` 是纯呈现 | 后端只持久化"当前主题名 + 自定义方案列表"；`core/theme/mod.rs`（392 行）瘦身为配置读写 |
| 列表排序 / 过滤 / 展开折叠 | 纯呈现态，无需持久化 | 后端只返回原始 `Vec`；前端 zustand 管展开态 / 排序列 / 过滤词。无需新增契约 |
| 快捷键分发 | keydown → 对应 invoke，前端天然位置 | 后端只存 keymap 配置 JSON |

前端 `src/ipc/client.ts:78` 的 `copySelection` 需随之删除。

---

## 6. 测试策略

### 6.1 测试现状（不乐观，但正好支撑方案 A）

| 项 | 实测 | `docs/06` 目标 |
|----|------|---------------|
| 含 `#[cfg(test)]` 的后端文件 | 18 个 | — |
| 后端测试点 | ~100 个（全是单元测试） | 500+ 单元 |
| 集成测试 | 0 | 100+ |
| E2E 测试 | 0（`tests/e2e/` 为**空目录**） | 20+ |
| fixtures | `tests/fixtures/{keys,plugins,sessions,sftp}` 全为**空目录** | — |
| benches | 空目录 | — |
| 前端测试 | 无（`package.json` 无测试框架） | — |

**最关键的事实**：整个项目从未有过运行时验证。`CLAUDE.md:121` 记录 GPUI 层"只过了 typecheck，无法在此主机运行时测试"。当前 Tauri 壳也只有占位 `hello`。

因此测试策略的首要目标不是补齐金字塔，而是**先建立"能跑"这个事实**。

### 6.2 分层测试职责

| 层 | 测什么 | 工具 | 备注 |
|----|--------|------|------|
| 后端单元 | service 逻辑、加密、协议解析 | `cargo test` | 已有 ~100 个，**迁移全程必须保持全绿**（不回退的锚） |
| `CommandOutcome` 契约 | 每个读命令返回正确变体 | `cargo test -p rshell-core` | **新增**；直接覆盖 §3.2 的死循环 |
| IPC 薄壳 | 宏展开正确、`CoreError → IpcError` 映射 | `cargo test`（`src-tauri`） | 薄壳无逻辑，测试量小 |
| 类型同步 | TS 类型与 Rust 一致 | `ts-rs` + CI `git diff --exit-code` | 编译期保障，零维护成本 |
| 前端单元 | store reducer、scheme → ITheme 映射 | Vitest（**切片 0 引入**，§9 裁定 #5） | 纯函数，易测 |
| E2E | 连接真 SSH → 出字 → 传文件 | `tauri-driver` + WebDriver + Docker sshd | 填上现在空的 `tests/e2e/` |

**E2E 前置条件**：`tests/fixtures/` 放 `docker-compose.yml` 起 `linuxserver/openssh-server`，固定端口 + 固定测试密钥。没有真实 SSH 目标，`docs/08` 的 #2（host key 决策）、#3（direct-tcpip 隧道）永远无法验证。

### 6.3 全程不变量

每个切片结束时都必须满足：

1. `cargo test` 全绿，且**测试数不减少**
2. `cargo clippy --workspace --all-targets` 零 warning
3. `npm run typecheck` 零错误
4. `npm run test` 全绿（Vitest，切片 0 起生效）
5. 应用**可运行**，不留半成品状态
6. `AppCommand`/`AppEvent` 的每次增删同步更新 `ts-rs` 导出

---

## 7. 迁移顺序（方案 A：垂直切片优先）

### 7.1 为何选垂直切片

被否决的两个备选：

| 备选 | 否决理由 |
|------|---------|
| 先补全 IPC 层再写前端 | 有很长一段时间没有任何东西能跑；IPC 正确性只能靠编译器，而**编译通过 ≠ 链路通**（`CLAUDE.md:121` 已有前车之鉴）；56 个薄壳可能有一半设计错了，等前端接入才发现 |
| GPUI 与 Tauri 双前端并存 | 已不适用（`crates/rshell-ui` 在 `16720d3` 已删除）；且 GPUI 前端本就从未运行成功，"保底"是假的 |

**核心论据**：本项目最大风险不在代码量，而在**从未有过运行时验证**。垂直切片是唯一能在数天内把"能不能跑"变成已知的方案。

### 7.2 切片 0：可行性证伪（最优先，数小时内出结论）

按风险从高到低逐个证伪：

1. `rhai = { features = ["sync"] }` + `cargo check -p rshell-core` → 验证 `CommandDispatcher: Send + Sync`
2. 一个真实命令跑通 `invoke` 往返（替掉占位 `hello`）
3. 一条 `Channel<Vec<u8>>` 推 1 MB 假数据到 xterm.js，测量吞吐并**记录为基线**
4. 引入 Vitest 基座（§9 裁定 #5），跑通一个 store 的样例测试

任何一条不通，回到设计而非硬做。第 1 条失败 → 降级到 §1.2 的备选方案。

### 7.3 切片 1：连接 + 出字（最窄垂直路）

| 侧 | 工作 |
|----|------|
| 后端壳 | `state.rs`（`AppState`）、`error.rs`（`IpcError`）、`terminal.rs`（`TerminalChannels`）、`events.rs`（EventBus → emit 桥） |
| 命令 | `connect_session` / `disconnect_session` / `send_input` / `resize_terminal` / `attach_terminal` / `list_sessions` |
| 契约 | `CommandOutcome` 骨架（先只需 `None` / `Sessions`） |
| 前端 | `TerminalPanel.tsx` 去掉假输出（`:54-62`），接 Channel + `term.onData` → `send_input`；引入 `@xterm/addon-webgl`（§9 裁定 #3，含 `onContextLoss` 回退处理） |

**完成判据：真的连上一台 SSH 服务器，敲 `ls` 看到真实输出。** 这将是本项目第一次运行时验证。

### 7.4 切片 2：清理

- 删除 §2.2 全部清单（alacritty 缓冲、`TerminalBufferSnapshot`、60Hz ticker、废弃 `send_input`）
- 删除 §3.3 的 9 个事件及前端对应分支
- 删除 §5 的 `CopySelection` / `ClipboardCopy`
- 引入 `ts-rs`，比对并替换手工 `types.ts`
- 引入 `@xterm/addon-search`，接上 `TerminalPanel.tsx:93-105` 的静态搜索栏（§9 裁定 #2）

放在切片 1 之后是有意的：**先证明新路能走，再拆旧桥**。

### 7.5 切片 3+：按功能域横向铺开

| 顺序 | 功能域 | 排序理由 |
|------|--------|---------|
| 3 | 会话 CRUD + 主题 + `dockview` 布局 | 纯 CRUD，验证 `CommandOutcome` 全貌；同期引入 `dockview` 替换自实现 `TabBar`（§9 裁定 #1） |
| 4 | Host key 决策 | 修 `docs/08` #2；切片 1 连接非首次主机时就会撞上 |
| 5 | SFTP 传输 + 文件浏览 | 第二大功能；验证 Channel 之外的高频事件（进度） |
| 6 | 密钥管理 + 主密码 | 安全域；"私钥不过 IPC"的边界在此固化 |
| 7 | 快速命令 + 触发器 + 脚本 | 验证 rhai `sync` 在真实负载下的表现 |
| 8 | 隧道 | 依赖 `docs/08` #3（direct-tcpip 未实现） |
| 9 | 插件 | 依赖 WASM sandbox（仍是 scaffold） |

切片 8、9 卡在既有未实现项上——迁移不会让它们变好，但也不会更坏。

---

## 8. 需同步更新的既有文档

本设计落地时，以下文档与现状/本设计存在偏差，需同步：

| 文档 | 偏差 |
|------|------|
| `CLAUDE.md:96` | 称 `TerminalBufferUpdated` 经 Channel 传二进制；本设计删除该事件，改为原始字节经 Channel |
| `CLAUDE.md:101` | 称 Tauri 命令返回 `Result<T, String>`；本设计改为 `IpcError` |
| `CLAUDE.md:86` | 描述"转换 `AppCommand` 后推入 channel"；本设计为直接 `dispatch`（无中间 channel） |
| `CLAUDE.md:121` | 称 `crates/rshell-ui/` 待删除；实际已在 `16720d3` 删除 |
| `docs/03-detailed-design.md`、`docs/05-development-standards.md` 等 7 份 | 仍含 GPUI 相关描述 |
| `docs/06-test-strategy.md` | 测试目标基于 GPUI 架构，需增加前端 Vitest 层与 tauri-driver E2E 层 |
| `docs/08-incomplete-features.md` | #2（host key）与本设计切片 4 对应；应交叉引用 |

---

## 9. 已裁定的原未决问题

原列为未决的 5 项已于 2026-07-31 全部裁定：

| # | 问题 | 裁定 | 落到哪个切片 |
|---|------|------|------------|
| 1 | 前端布局组件选型 | **引入 `dockview`** 做多标签 + 分屏，替代自实现的 `TabBar.tsx` | 切片 3 |
| 2 | xterm 搜索 addon | **引入 `@xterm/addon-search`**，接上 `TerminalPanel.tsx:93-105` 的静态搜索栏 | 切片 2 |
| 3 | WebGL 渲染 | **引入 `@xterm/addon-webgl`**（不再取决于吞吐测量结果） | 切片 1 |
| 4 | capabilities 细粒度化 | **先按默认来**（`capabilities/default.json` 不动），后续开发中按需增加条目 | 按需 |
| 5 | Vitest 引入时机 | **一开始就引入**（切片 0 即建立前端测试基座） | 切片 0 |

裁定带来的两处设计调整：

- **#3 提前到切片 1**：WebGL addon 不再作为"吞吐不足时的补救"，而是初始配置的一部分。切片 0 步骤 3 的吞吐测量仍然做，但目的从"决定是否引入 WebGL"变为"记录基线性能数据"。需注意 `addon-webgl` 在部分环境下会 fallback 到 canvas，须处理 `onContextLoss` 事件。
- **#5 提前到切片 0**：前端测试基座与 Rust 侧的可行性证伪同期建立，使切片 1 起每个切片都能同时满足 §6.3 的不变量 1（`cargo test`）与新增的前端测试要求。

`package.json` 因此需新增依赖：`@xterm/addon-search`、`@xterm/addon-webgl`、`dockview`（切片 3 前）、`vitest` + `@testing-library/react` + `jsdom`（devDependencies，切片 0）。
