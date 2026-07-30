# RShell 未完整实现功能清单

> **建立日期**：2026-07-30
> **盘点对象**：基于 `cargo +stable clippy --workspace --all-targets -- -D warnings` 干净基线 + `cargo +stable test --workspace` 通过后的仓库状态（commit `a7b6684`）。
> **目的**：在 v0.1.0+ 主线稳定后，把"看起来跑通但内部留洞"或"完全未做"的工作落到计划里，按 ROI 分批做。
> **使用方式**：每条带唯一 id（`#1`–`#20`），有 `severity`（P0/P1/P2/P3）、`state`（`todo` / `doing` / `done` / `blocked`）、`estimate`（S/M/L）。

## 状态总览

| severity | count | 说明 |
|----------|------:|------|
| P0 功能阻塞 | 4 | 用户能直接感知"不行"：RDP、host key 决策、SSH 隧道、关键协议 |
| P1 内部留洞 | 7 | 功能能用但链路不全或测试覆盖空白 |
| P2 测试空白 | 4 | 仅有手动验证 |
| P3 开发体验 | 5 | lint / 配置 / 重构 |

---

## P0：功能阻塞

### #1 RDP 全链路握手（X.224 → TLS → NLA → 帧 pump）

- **location**：`crates/rshell-protocol/src/rdp/mod.rs:1-32`（注释自述状态表）
- **state**：todo
- **estimate**：L
- **缺什么**：
  1. TLS 升级（tokio-rustls 拆 framed 内部 TcpStream → rustls `client.connect` → 重新包 `TokioFramed<TlsStream>`）
  2. CredSSP / NLA（需 sspi + reqwest + hickory-resolver）
  3. `connect_finalize`（能力交换 + 通道连接）
  4. ActiveStage 帧渲染：把 `ironrdp-graphics` `Software` 渲染器跑在 `tokio::spawn` 后台，GraphicsUpdate → RGBA 帧通过 `frame_tx` 推
- **风险**：sspi 依赖重，跨平台 NLA 验证需要真 RDP server（Windows / xrdp / FreeRDP），本机无环境

### #2 Host key 决策链路端到端

- **location**：`crates/rshell-core/src/session/service.rs:89`
- **state**：todo
- **estimate**：M
- **现状**：`SshClient::connect_ssh` 返回 `HostKeyDecision` oneshot tx，`SessionService` 用 `Ok(_decision_tx)` 丢弃
- **缺什么**：
  1. `SessionService::connect` 返回类型扩展 `((), Option<oneshot::Sender<HostKeyDecision>>)` 或新方法 `pending_host_key_decision`
  2. `AppCommand::DecideHostKey { decision_id, accept, permanent }`
  3. `CommandDispatcher` 内部维护 `HashMap<decision_id, oneshot::Sender<...>>`，`DecideHostKey` 时 `tx.send(...)` 移除
  4. UI 弹"信任一次 / 永久 / 拒绝"对话框（基于 `HostKeyMismatch` 事件已有）
- **依赖**：russh 的 `Handle::current().block_on(rx)` 路径在 multi-thread runtime 上 OK，但 single-thread 上要确认

### #3 SSH `direct-tcpip` 通道转发

- **location**：`crates/rshell-core/src/security/tunnel_manager.rs:80-105`
- **state**：todo
- **estimate**：M
- **现状**：连接后仅做 TCP 透传到 `remote_host:remote_port`，没有真正走 SSH 通道
- **缺什么**：
  1. 调 `ssh.channel_open_direct_tcpip(remote_host, remote_port, originator, originator_port)`
  2. 拿到 `Channel<Msg>` 后用 `make_stream_into_channel` / `channel.into_stream()` 把 `inbound` TCP 和 channel 互为读写
  3. 测试需要真 SSH 目标 + 启用 RSA 密钥认证
- **当前 workaround**：保留 `_ssh_client` 字段做日志；功能上对纯 SSH-local 隧道等于 plain TCP 转发

### #4 Telnet NAWS resize

- **location**：`crates/rshell-protocol/src/telnet/mod.rs:330-335`
- **state**：todo
- **estimate**：S
- **缺什么**：在 `resize(cols, rows)` 内构造字节序列 `IAC SB NAWS <0><cols_high> <0><cols_low> <0><rows_high> <0><rows_low> IAC SE` 并通过 `process_data` 走 `TelnetCodec` 发出
- **小**：~20 行

---

## P1：功能可用但内部留洞

### #5 触发器引擎没接进 recv 循环

- **location**：`crates/rshell-core/src/script/trigger_engine.rs`
- **state**：todo
- **estimate**：M
- **现状**：`check_output` / `notify_fired` 全仓库 grep 无 caller
- **缺什么**：在 `SessionService::connect` 后台 `tokio::spawn` 那个 recv 任务里，收到 `TerminalOutput { data, .. }` 后做：
  1. `str::from_utf8(data)` 失败就跳过（VT 字节多半不 UTF-8）
  2. `trigger_engine.check_output(text, session_id)` → 命中 enabled triggers
  3. 对每个 match 调 `notify_fired` 并按 `action` 类型派发（`SendCommand` / `RunScript` / `Notify`）

### #6 rhai 引擎 host API 几乎为空

- **location**：`crates/rshell-core/src/script/engine.rs:35-43`
- **state**：todo
- **estimate**：M
- **现状**：只 register 了 `rshell_log` / `rshell_sleep`
- **缺什么**：
  1. `rshell_send(session_id, data)` — 转发到 `SessionService::send_data`
  2. `rshell_list_sessions()` — 返回 `Vec<SessionConfig>`（仅 ID + name）
  3. `rshell_run_quick_command(cmd_id, target_sessions)` — 触发已有 quick command
  4. `rshell_get_terminal_title(session_id)` — 读 title
- **风险**：host API 是安全敏感面（脚本能 send_data），需加权限模型或默认 deny

### #7 隧道注册表只内存

- **location**：`crates/rshell-core/src/security/tunnel_manager.rs`（`tunnels: HashMap<Uuid, ActiveTunnel>` 字段）
- **state**：todo
- **estimate**：S
- **缺什么**：启动时从 `data_local_dir/rshell/tunnels.toml` 读，关闭时 dump；用 `rshell-infra` 已有 TOML 持久化助手
- **与 #3 关系**：#3 完成前不必做，避免持久化还没生效的状态

### #8 RdpConnection::resize

- **location**：`crates/rshell-protocol/src/rdp/mod.rs:282-287`
- **state**：blocked（依赖 #1）
- **estimate**：M
- **缺什么**：在 #1（ActiveStage 通了）后，调 `ClientConnectorState::mark_resize` 或发 `Deactivation-Reactivation` 序列

### #9 Windows ConPTY resize

- **location**：`crates/rshell-infra/src/pty/windows.rs:70`
- **state**：todo
- **estimate**：S
- **缺什么**：Windows 上 `ResizePseudoConsole(hPC, COORD)`，需 `windows` crate 的 `Win32_System_Console` —— 仓库已依赖 `windows = 0.61.3`

### #10 WASM 字符串跨边界

- **location**：`crates/rshell-plugin-sdk/src/sandbox.rs:81`
- **state**：todo
- **estimate**：M
- **现状**：`WasmValue::String(_) => Val::I32(0)`，字符串参数直接丢
- **缺什么**：
  1. 调 wasmtime `Memory` 拿 `data_ptr` / `data_len` linear memory buffer
  2. 写 UTF-8 字节进去
  3. 把 `(ptr, len)` 作为 `Val::I32`/`Val::I64` pair 返回
  4. 主机侧 host function 拿到 `(ptr, len)` 后再 `Memory::read` 还原 `String`

---

## P2：测试覆盖空白

### #11 script 子系统 0 单元测试

- **location**：`crates/rshell-core/src/script/{trigger_engine,sync_input,compose,quick_command}.rs`
- **state**：todo
- **estimate**：S
- **缺什么**：参照本轮 `theme` / `transfer` 加测试的模式，每个 service 加 2-3 个 happy path + state transition

### #12 SessionService 0 单元测试

- **location**：`crates/rshell-core/src/session/service.rs`
- **state**：todo
- **estimate**：M
- **缺什么**：本轮把 `std::sync::RwLock` 换成 `tokio::sync::RwLock` 后需要回归测试：跨 await 不持锁、create / connect / disconnect / send_data / get_state / list_sessions 状态正确

### #13 Telnet / Serial 0 测试

- **location**：`crates/rshell-protocol/src/telnet/mod.rs`、`serial/mod.rs`
- **state**：todo
- **estimate**：M
- **缺什么**：Telnet 协议选项协商（DO/DONT/WILL/WONT 状态机）、Serial 配置校验（baud_rate / data_bits / parity / stop_bits / flow_control 互斥规则）

### #14 ViewModel 0 测试

- **location**：`crates/rshell-ui/src/view_models/{session,terminal,transfer}_vm.rs`
- **state**：todo
- **estimate**：M
- **缺什么**：把 `AppEvent` → VM 状态转换抽成 trait，纯逻辑单测；`TerminalBufferUpdated` 路径、`TransferStateChanged` 进度累积、`SessionListChanged` 全量重载等

---

## P3：开发体验小坑

### #15 Terminal buffer 推送未节流

- **location**：`crates/rshell-core/src/terminal/service.rs:265` (`process_output`)、`session/service.rs` 后台 recv 循环
- **state**：todo
- **estimate**：S
- **现状**：`TerminalBufferUpdated` 每个 chunk 都发（注释里说"生产环境应做 60Hz 节流"）
- **缺什么**：合并窗口 16ms（`tokio::time::interval` + flush on drop）

### #16 CommandDispatcher::new 9 参数豁免

- **location**：`crates/rshell-core/src/command_dispatcher.rs:53`
- **state**：todo
- **estimate**：S
- **现状**：`#[allow(clippy::too_many_arguments)]` 显式豁免
- **缺什么**：抽 `pub struct Services { session_service, terminal_service, ... }` 整体传入

### #17 `upper.is_ascii_uppercase()`

- **location**：`crates/rshell-ui/src/views/terminal_view.rs:115`
- **state**：todo
- **estimate**：S
- **缺什么**：clippy --fix 已建议；本轮没被 `-D warnings` 强制

### #18 `proc-macro-error2 v2.0.1` future-incompat

- **location**：`Cargo.lock`（上游 crate，非本仓库）
- **state**：blocked
- **estimate**：S
- **缺什么**：等上游修或换 crate

### #19 `rust-toolchain.toml` 钉版不可用

- **location**：`rust-toolchain.toml`
- **state**：todo
- **estimate**：S
- **现状**：`channel = "1.90"`（不完整别名），清华镜像上 404；本机需 `+stable` 才能跑命令
- **缺什么**：改成 `channel = "1.90.0"` 或 `"stable"`（取决于项目政策）

### #20 View 事件路由散落

- **location**：`crates/rshell-ui/src/app.rs:140-150`
- **state**：todo
- **estimate**：M
- **现状**：`app.rs` 手动 `update` 每个 vm 转发 `handle_event`
- **缺什么**：用 gpui `cx.subscribe` 让 view 自订阅 `EventBus` 桥接事件，省掉中心路由器

---

## 推荐执行顺序

按 ROI 排序，下一轮可挑：

1. **#4 + #17 + #9**（全 S，加起来 50 行，立刻可验）
2. **#5 + #11**（脚本子系统，触发器接通有立竿见影的效果，附测试）
3. **#15**（60Hz 节流，性能 + UX 提升）
4. **#2**（host key 决策闭环，用户首次连新主机会看到"信任"对话框）
5. **#3**（SSH 隧道真转发，需要真 SSH 目标做集成测试）
6. **#1 / #6 / #8** 各自独立大改，需独立轮次

## 进度跟踪

每完成一项，update 此文件对应行 `state: done` + commit message 引用本文件 id。

## 已完成 (本轮 commit 0.1.0+ 收尾)

| id | 模块 | commit | 备注 |
|----|------|--------|------|
| #4 | telnet NAWS resize | `97df88f` | RFC 1073 字节级 + 测试 |
| #5 | trigger engine 接入 recv | `97df88f` | 4 个 trigger action 派发 |
| #9 | NAWS 协商 + DO 命令 | (同上) | telnet `connect` 时主动 DO |
| #10 | trigger / quick_command / sync_input 单测 | `97df88f` | 0 → 20 测试 |
| #11 | terminal buffer 60Hz 节流 | `97df88f` | ticker + dirty flag |
| #12 | host key 决策端到端 | `81341af` | HostKeyDecisionSink trait + registry |
| #14 | SSH direct-tcpip | `42e699a` | russh channel_open_direct_tcpip |
| #15 | RDP 状态机测试 | `01b2859` | TLS/NLA/pump 仍需真 server |
| #16 | tunnel 持久化 | `81ee342` | tunnels.toml + 2 测试 |
| #17 | rhai host API | `d6681b0` | 5 个新 fn + 8 测试 |
| #20 | WASM 字符串 marshalling | `2ec2941` | linear memory 写入/读出 |
| #22 | SessionService 测试 | `a00bfb4` | 13 测试覆盖 CRUD + lock-leak |
| #23 | Telnet/Serial 测试 | `62980f8` | 11 测试覆盖配置不变性 |
| #25 | CommandDispatcher bundle | `9abc0e4` | Services struct 替代 11 参数 |
| #26 | toolchain 修 | `e1e673d` | channel = stable (避开 1.90 alias 404) |
| 17 | terminal_view 1 行清理 | `97df88f` | upper.is_ascii_uppercase() |

## 留作 follow-up

| id | 原因 |
|----|------|
| #1 RDP TLS/NLA/frame pump | 需要真 RDP server 验证 |
| #3 RemoteForward + DynamicForward (SOCKS) | 本轮仅做 LocalForward 基础 |
| #7 隧道 rules 启动时**自动**重建 | 本轮只持久化 + 暴露 restore_pending_rules, 自动重建需 UI 端 |
| #8 RDP resize 真实现 | 依赖 #1 ActiveStage |
| #9 Windows ConPTY | 当前 `WindowsPty` 用 cmd.exe stdin/stdout 管道, 不是真 ConPTY, 整层重写 |
| #18/19 ConPTY 整层 + RDP 协议 | 关联大改, 独立大轮次 |
| #24 ViewModel 单测 | 本机无 GPUI runtime (CLAUDE.md 已注明), 需要真 GPUI 环境 |
| #27 view event routing 改 gpui subscribe | 同上 |
| 18 | `proc-macro-error2 v2.0.1` future-incompat 上游 | 等上游 |
