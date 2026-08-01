# Task Plan — Tauri 2.0 前后端划分实施计划

> 基线 commit：`6d2aac9`
> 设计文档：`docs/superpowers/specs/2026-07-31-tauri2-frontend-backend-split-design.md`（v1.1）
> 工作流基线：CLAUDE.md、§6.3 不变量、本计划全程必须满足

---

## 0. 全局原则（来自设计 §1.3 + §6.3）

- **边界**：crates/** 零 tauri；src/** 零后端内部；src-tauri/src/** 零业务逻辑；状态唯一所有者。
- **不变量**：每个切片结束必须满足 cargo test 全绿且测试数不减少、clippy 零 warning、typecheck 零错误、npm run test 全绿、应用可运行、`ts-rs` 导出与 Rust 一致。
- **决策已锁**：D1–D6（设计 §0.2）+ 5 项裁定（设计 §9）+ Vue 3 + Element Plus（§10）。
- **降级路径**：仅 D5（`rhai sync`）保留降级方案——独立 actor 线程（§1.2）。

---

## 阶段总览

| 阶段 | 主题 | 完成判据 | 前置 |
|------|------|---------|------|
| **切片 0** | 可行性证伪 + 前端换栈 + 测试基座 | 5 项证伪全部通过；Vue 3 dev server 启动；1 MB 假数据 Channel 吞吐基线入库；Vitest 跑通 | — |
| **切片 1** | 连接 + 出字 + 持久化（首次运行时验证） | 真 SSH 跑通 `ls`；重启后会话仍在 | 切片 0 |
| **切片 2** | 清理：删 alacritty 残留 + 9 事件 + 引入 ts-rs + 搜索 addon | cargo test 全绿；types.ts 由 generated.ts 重导出；git diff 校验空 | 切片 1 |
| **切片 3** | 会话 CRUD + 主题 | `CommandOutcome` 全变体覆盖 | 切片 2 |
| **切片 4** | Host key 决策（修 docs/08 #2） | 首次主机弹窗可决策；后续直连 | 切片 1（连接非首次主机时撞上） |
| **切片 5** | SFTP 传输 + 文件浏览 | 大文件传输进度事件不停 | 切片 3 |
| **切片 6** | 密钥管理 + 主密码 | 私钥永不过 IPC（断言测试） | 切片 3 |
| **切片 7** | 快速命令 + 触发器 + 脚本 | rhai 真实负载下 SendText 链路通 | 切片 1 |
| **切片 8** | 隧道 | 受 docs/08 #3 限制，IPC 接入即可 | 切片 3 |
| **切片 9** | 插件 | WASM sandbox 仍 scaffold，IPC 接入即可 | 切片 3 |

---

## 切片 0 — 可行性证伪 + 前端换栈 + 测试基座

**目标**：把"能不能跑"从未知变成已知；建立前后端两套测试基座。

### 0.1 后端侧

- [ ] **D5 重测**：`src-tauri/crates/rshell-core/Cargo.toml` `rhai` 开 `sync` feature；`cargo check -p rshell-core`；`assert_send_sync::<CommandDispatcher>()` 通过；74 个测试仍全绿
- [ ] **修复 .gitignore**：`src-tauri/.gitignore` 内 `src-tauri/target/` → `target/`（设计 §7.2 末段）
- [ ] **AppState 雏形**：`src-tauri/src/state.rs` 最小骨架 `pub struct AppState { pub dispatcher: Arc<CommandDispatcher> }`，接入 `app.manage()`；`lib.rs` setup 阶段构造并注入
- [ ] **占位 hello → 真实命令**：用 `list_sessions` 替掉 hello（即便返回 `Vec::new()`），证明 invoke 真实往返
- [ ] **吞吐基线**：临时 `#[tauri::command] fn push_one_mb(channel: Channel<Vec<u8>>)` 推 1 MB 假数据；前端 xterm.js 接住；记录 fps / lag / 总量 → 写入 `progress.md` 与 `findings.md#V1`

### 0.2 前端侧（§10.3 / §10.4）

- [ ] 删除：`App.tsx`、`main.tsx`、`components/*.tsx`、`views/*.tsx`、`store/*.ts`、`styles/tokens.css`
- [ ] 保留：`src/ipc/{types,client,events}.ts`
- [ ] 调整：`package.json`（§10.3 依赖清单）、`vite.config.ts`（`@vitejs/plugin-vue`）、`tsconfig.json`（移除 `jsx: react-jsx`）、`package.json#typecheck` 改为 `vue-tsc --noEmit`
- [ ] 安装：`vue@^3.5`、`element-plus@^2.14`、`pinia@^3`、`dockview-vue@^7.0.4`、`@element-plus/icons-vue`、`@xterm/addon-search`、`@xterm/addon-webgl`、dev: `@vitejs/plugin-vue`、`vue-tsc`、`@vue/test-utils`、`vitest`、`jsdom`
- [ ] 新建最小 Vue 3 入口：`src/main.ts`（或 `.ts`，按 vue-tsc 习惯定）→ `App.vue` → 单组件渲染 Element Plus 按钮 → 点击调 `invoke('list_sessions')` 显示返回值，证明 invoke 真实往返
- [ ] **测试基座**：`vitest.config.ts`（jsdom 环境）；`tests/unit/sample-store.spec.ts` 跑通一个 pinia store 测试

### 0.3 文档同步

- [ ] `CLAUDE.md` §1.2 标注"rhai sync 已实测通过"；§1 补一句"前端栈 Vue 3"（与 §10 对齐）

### 0.4 完成判据

1. `cargo test --workspace` 全绿（74+ 测试不减少）
2. `cargo clippy --workspace --all-targets` 零 warning
3. `npm run typecheck`（`vue-tsc --noEmit`）零错误
4. `npm run test` 全绿（≥1 个样例）
5. `cargo tauri dev` 启动后看到 Element Plus UI；点击按钮拿到真实 sessions 列表
6. 1 MB Channel 吞吐基线已记录

---

## 切片 1 — 连接 + 出字 + 持久化（首次运行时验证）

**目标**：真 SSH 跑通 `ls`，重启后会话仍在。

### 1.0 前置修复（§4.5，阻塞整个切片）

- [ ] `SessionService::new` 增加 `Arc<SessionRepository>` 参数
- [ ] `SessionService::load_from_disk()`：setup 阶段调用，把 `repository.list_all()` 灌进 `sessions` HashMap
- [ ] `create_session` / `update_session` / `delete_session` 三处同步调 `repository.save()` / `delete()`
- [ ] 单元测试：保存 N 个 → 重新构造 `SessionService` → `list_all()` 返回 N 个

### 1.1 后端壳基础

- [ ] `src-tauri/src/state.rs`：`AppState { dispatcher: Arc<CommandDispatcher>, event_bus: Arc<EventBus>, terminal_channels: Arc<TerminalChannels> }`
- [ ] `src-tauri/src/error.rs`：`IpcError { kind, message, session_id }`；`From<CoreError> for IpcError`；7 个固定 kind 字符串
- [ ] `src-tauri/src/terminal.rs`：`TerminalChannels` 双态 sink（`Buffering(VecDeque<u8>, 256 KiB cap)` / `Attached(Channel<Vec<u8>>)`）；`push` / `attach` / `detach` 三个方法
- [ ] `src-tauri/src/events.rs`：启动时订阅 EventBus，循环 `recv` → `app.emit_to("main", "rshell://event", payload)`

### 1.2 命令薄壳（第一批 7 个）

- [ ] `cmd!` 宏定义于 `src-tauri/src/macros.rs`
- [ ] `create_session` / `list_sessions` / `connect_session` / `disconnect_session` / `send_input` / `resize_terminal` / `attach_terminal`
- [ ] `attach_terminal` 用 Tauri Channel API，参数 `session_id: Uuid`；首次注册 TerminalChannels

### 1.3 契约小步迭代

- [ ] `rshell-api` 新增 `CommandOutcome` 骨架（先 `None` / `Sessions` / `SessionId` 三变体）
- [ ] `CommandDispatcher::dispatch` 签名改 `async fn dispatch(&self, cmd: AppCommand) -> Result<CommandOutcome, CoreError>`
- [ ] 三处迁移：`create_session` → `SessionId(Uuid)`；`list_sessions` → `Sessions(Vec<SessionConfig>)`；其余 → `None`
- [ ] 单元测试：每个读命令的返回变体正确（含 §3.2 死循环相关的两个：triggers/quick_commands 本切片可暂不动）

### 1.4 前端（Vue 3 + dockview-vue + xterm.js）

- [ ] `src/components/TerminalPane.vue`：`<script setup>` 内 `onMounted` 建 `xterm.Terminal` + `FitAddon` + `WebglAddon`（处理 `onContextLoss` 回退 canvas）；`invoke('attach_terminal', { sessionId, onData: ch })` 接管 `ch.onmessage` → `term.write()`；`term.onData` → `invoke('send_input', …)`
- [ ] `src/components/SessionCreateDialog.vue`：Element Plus `el-dialog` + `el-form`，字段 host/port/username/auth_method；提交调 `invoke('create_session')` → 列表
- [ ] `src/App.vue`：dockview-vue 容器；一个面板渲染 SessionList，一个面板渲染 TerminalPane；选中 session → invoke('connect_session') → 收到 `ConnectionStateChanged` → 挂 TerminalPane
- [ ] `src/stores/sessions.ts`（pinia）：`sessions: SessionConfig[]`、`current: Uuid | null`、`connectionState: Map<Uuid, ConnectionState>`
- [ ] 订阅：监听 `app://event` → 分发到 store（按事件名 switch）

### 1.5 修触发器 SendText（§2.4）

- [ ] `session/service.rs:227-232` 改为直接 `SessionService::send_data` 发往远端，不再 publish `TerminalOutput`

### 1.6 完成判据

1. 真 SSH 服务器上敲 `ls` 看到真实输出（设计 §7.3 关键判据）
2. 进程退出 → 重启 → `list_sessions()` 仍可见（§4.5 修复证据）
3. 切片 0 不变量仍全绿
4. Host key 决策：若不是首次主机，本切片撞上 §4.3 B 流程的断链——记录到 progress.md，不在本切片闭环

---

## 切片 2 — 清理 + ts-rs 引入

**目标**：把"先证明新路能走"之后的旧桥拆掉；建立类型同步护栏。

### 2.1 后端删除清单（§2.2）

- [ ] 删除 `core/terminal/buffer.rs`
- [ ] 瘦身 `core/terminal/service.rs` 至 ~60 行（仅 `create_terminal` / `destroy_terminal` / `resize`）
- [ ] 删除 `TerminalService::{send_input, process_output, get_buffer_snapshot}`
- [ ] 删除 `session/service.rs` 内 60Hz ticker + `dirty` 标志
- [ ] 移除 `core/Cargo.toml` 的 `alacritty_terminal` 依赖
- [ ] 删除 `api/events.rs::TerminalBufferUpdated`、`api/types.rs::TerminalBufferSnapshot` 及 Cell/CellFlags 系列

### 2.2 事件收敛（§3.3）

- [ ] 删除 7 个 `*Snapshot` 事件 + `ClipboardCopy` + `TerminalBufferUpdated`
- [ ] 前端 `src/ipc/events.ts` 对应分支删除（`onSessionsSnapshot` 等 9 处）
- [ ] 删除 `src/ipc/client.ts:78 copySelection` 与对应后端 `CopySelection` 命令 + `core/theme/mod.rs` 392 行瘦身为配置读写
- [ ] `AppEvent` 收敛到约 33 个

### 2.3 ts-rs 引入（§3.6）

- [ ] `rshell-api/Cargo.toml` 加 `ts-rs = "0.25"`（或当前稳定版）；为每个枚举/结构 derive `TS`
- [ ] 构建脚本或 `cargo test` 钩子导出到 `src/ipc/generated.ts`
- [ ] `src/ipc/types.ts` 改为从 `generated.ts` 重导出（先比对零偏差后切换）
- [ ] CI 占位（本地 git hook 即可）：`git diff --exit-code src/ipc/generated.ts`

### 2.4 搜索 addon（§9 #2）

- [ ] 安装 `@xterm/addon-search`
- [ ] `TerminalPane.vue` 增加搜索栏 UI（Element Plus `el-input` + 上下条按钮）

### 2.5 文档同步（§8）

- [ ] `CLAUDE.md:96/101/86/121` 四处修改
- [ ] `docs/03-detailed-design.md`、`docs/05-development-standards.md` 集中清 GPUI 残留
- [ ] `docs/06-test-strategy.md` 增 Vitest / tauri-driver 章节

### 2.6 完成判据

1. cargo test 全绿 + clippy 零 warning
2. npm run typecheck + test 全绿
3. `git diff src/ipc/generated.ts` 为空（设计 §3.6 强制）
4. 真 SSH 上敲 `ls` 仍能跑（无回退）
5. 终端搜索可用

---

## 切片 3 — 会话 CRUD + 主题

**目标**：`CommandOutcome` 全变体覆盖；主题配置读写闭环。

### 3.1 后端

- [ ] 补齐 `CommandOutcome` 剩余变体（`Keys` / `Tunnels` / `Plugins` / `Themes{…}` / `PendingTunnels` / `RemoteDir` / `PublicKey` / `Verified`）
- [ ] 修 §3.2 死循环：`ListTriggers` / `ListQuickCommands` 改为 `Triggers(Vec<Trigger>)` / `QuickCommands(Vec<QuickCommand>)` 返回 `CommandOutcome`，不再仅 publish 事件
- [ ] 增薄壳：`update_session` / `delete_session` / `list_keys` / `verify_master_password` / `list_themes` / `set_theme` / 等
- [ ] `core/theme/mod.rs` 瘦身为"配置读写 + 当前主题/方案名持久化"

### 3.2 前端

- [ ] `SessionList.vue`：`el-table` + 排序/过滤/展开折叠（pinia 管）
- [ ] `ThemePanel.vue`：主题切换 → `set_theme` → 收到 `ThemeChanged` → 本地 CSS 变量重算
- [ ] scheme → CSS 变量映射：纯前端函数，Vitest 单测覆盖

### 3.3 完成判据

1. `CommandOutcome` 每个变体至少 1 个 dispatch 单元测试
2. 主题切换即时生效（颜色变量 / xterm ITheme 同步）
3. 列表排序/过滤/折叠刷新后丢失（应然）

---

## 切片 4 — Host key 决策

**目标**：闭环 §4.3 B 流程；修 `docs/08 #2`。

### 4.1 后端

- [ ] `protocol::ssh::host_key_decision`（或合适位置）：`HostKeyDecisionRegistry` 持有 `decision_id → oneshot::Sender<Decision>`
- [ ] SSH 握手线程注册 decision 后阻塞等待
- [ ] `decide_host_key(decision_id, accept, permanent)` 薄壳 → 取出 oneshot → send → 握手线程解除
- [ ] 命中预期指纹：自动接受（不发事件）；不匹配：emit `HostKeyMismatch { decision_id, expected, received, public_key_blob }`

### 4.2 前端

- [ ] `HostKeyMismatchDialog.vue`（Element Plus）：显示指纹对比 + 三按钮（信任一次/永久/拒绝）
- [ ] pinia store 收到事件 → 弹对话框 → 用户点击 → `invoke('decide_host_key')`

### 4.3 完成判据

1. 首次连接新主机：弹窗可决策
2. "永久信任"后第二次连接：直连不弹窗
3. "拒绝"：握手返回 `auth_failed` 类错误，会话标红

---

## 切片 5 — SFTP 传输 + 文件浏览

**目标**：第二大功能；验证 Channel 之外的高频事件。

### 5.1 后端

- [ ] `TransferService` 已存在：核对与 `CommandOutcome::RemoteDir` 的契约对齐
- [ ] 薄壳：`list_remote_dir` / `upload` / `download` / `cancel_transfer`
- [ ] `TransferProgress` 事件：节流（>10 Hz 合并），`session_id`/`transfer_id`/`bytes_done`/`bytes_total`

### 5.2 前端

- [ ] `FileBrowserPanel.vue`：双面板（本地 + 远端），`el-tree` 或 `el-table` + 懒加载目录
- [ ] `TransferQueue.vue`：进度条 + 暂停/取消
- [ ] 拖拽上传：`tauri-plugin-fs` + HTML5 drag events

### 5.3 完成判据

1. 100 MB 文件上传/下载进度事件不丢（节流后）
2. 取消传输：远端进程清理

---

## 切片 6 — 密钥管理 + 主密码

**目标**：固化"私钥永不过 IPC"边界。

### 6.1 后端

- [ ] `list_keys` 薄壳返回 `Vec<SshKeyInfo>`（仅元数据：name/fingerprint/type/created_at）
- [ ] `add_key(passphrase)` / `delete_key(id)` / `change_master_password`
- [ ] 私钥加载/解密走 `infra::crypto`，结果仅在内存用于 SSH 握手

### 6.2 前端

- [ ] `KeyManagerPanel.vue`：列表 + 导入（`tauri-plugin-dialog` 选文件）+ 删除
- [ ] `MasterPasswordDialog.vue`：启动时若未设置则引导设置

### 6.3 安全断言测试

- [ ] `grep -rn "private_key\|secret_key\|priv_key" src-tauri/src/` 必须为零匹配（除 `infra/crypto` 与 `protocol/ssh`）
- [ ] `SshKeyInfo` 序列化输出不含 PEM 块（schema 测试）

### 6.4 完成判据

1. 导入密钥 → list_keys 看到 → 用其连接真 SSH
2. 安全断言测试通过

---

## 切片 7 — 快速命令 + 触发器 + 脚本

**目标**：验证 rhai sync 在真实负载下的表现。

### 7.1 后端

- [ ] 修 §3.2 死循环（切片 3 已修 `ListTriggers` / `ListQuickCommands`；此处验证运行时）
- [ ] 触发器 SendText 改 `SessionService::send_data`（切片 1 已修，本切片验证）
- [ ] rhai 脚本：`script/list` / `script/run(name, args)` / `script/reload`
- [ ] 触发器：`TriggerFired` 事件 → 前端仅做提示

### 7.2 前端

- [ ] `QuickCommandPanel.vue`：列表 + 执行（弹输入框）
- [ ] `TriggerEditor.vue`：正则 + 动作（SendText / LogOnly）
- [ ] `ScriptEditor.vue`：Monaco / CodeMirror → `script/reload`

### 7.3 完成判据

1. 配置触发器 `^\$ ` → `clear` → 真实远端执行清屏
2. 快速命令执行：日志可见、命令真到远端
3. rhai 脚本 host 函数 6 个仍全部可调用（回归测试）

---

## 切片 8 — 隧道（受 docs/08 #3 限制）

### 8.1 后端

- [ ] 薄壳：`add_tunnel` / `list_tunnels` / `remove_tunnel` / `list_pending_tunnels`
- [ ] `CommandOutcome::Tunnels` / `PendingTunnels` 接入
- [ ] `direct-tcpip` 未实现项标注 TODO，IPC 接通但不保证可工作

### 8.2 前端

- [ ] `TunnelPanel.vue`：本地/远端/动态三栏

### 8.3 完成判据

1. 增/删隧道 → 重启应用 → 规则仍在
2. 本地端口转发：仅当 `direct-tcpip` 已实现时验证（否则跳过）

---

## 切片 9 — 插件（受 WASM scaffold 限制）

### 9.1 后端

- [ ] 薄壳：`list_plugins` / `enable_plugin` / `disable_plugin` / `plugin_command`
- [ ] `WasmSandbox` 仍是 scaffold——IPC 接通，运行时调用返回 `not_implemented`

### 9.2 前端

- [ ] `PluginPanel.vue`：插件列表 + 启用/禁用

### 9.3 完成判据

1. 启用/禁用状态重启后保留
2. 调用未实现命令返回稳定 `kind: "internal"` 错误

---

## 全局收尾

- [ ] 切片 9 完成后，回顾所有不变量；最终 cargo test + clippy + typecheck + vitest 全绿
- [ ] `findings.md` 与 `progress.md` 收尾
- [ ] 文档同步收口：`docs/03` `docs/05` `docs/06` `docs/08` 全部与新设计一致
- [ ] 一个 PR 一切片（或多切片小步合并），commit 风格遵循 CLAUDE.md Conventional Commits

---

## 进度表

| 阶段 | 状态 | 起止 | 备注 |
|------|------|------|------|
| 切片 0 | ✅ 完成 | 2026-07-31 | 见 progress.md 实测数据；D5 既成事实；前端栈 Vue 3 |
| 切片 1 | ✅ 完成 | 2026-07-31 | 见 progress.md；76+2 测试；CommandOutcome 骨架；首次运行时验证链路就绪 |
| 切片 2 | ✅ 完成 | 2026-07-31 | 见 progress.md；alacritty 缓冲删除；9 事件收敛；ts-rs 依赖就位；xterm 搜索；CLAUDE.md 实时改 |
| 切片 3 | ✅ 完成 | 2026-07-31 | 见 progress.md；5 个新薄壳 + ThemeInfo 类型 + 13 变体契约测试；前端 SessionList/ThemePanel；scheme→CSS 纯函数 + Vitest |
| 切片 4 | ✅ 完成 | 2026-07-31 | 见 progress.md；decide_host_key 薄壳 + HostKeyMismatchDialog + hostKey pinia store；permanent 持久化留切片 6 |
| 切片 5 | ✅ 完成 | 2026-07-31 | 见 progress.md；5 个 SFTP 薄壳 + 11 个 transfer 单元测试；TransferQueue/FileBrowserPanel 雏形；流式节流推迟
| 切片 6 | ✅ 完成 | 2026-07-31 | 见 progress.md；6 个密钥/主密码薄壳；私钥不过 IPC 安全断言 2/2；KeyManagerPanel/MasterPasswordDialog
| 切片 7 | 🟡 部分完成 | 2026-07-31 | 7 个新薄壳接通；前端 QuickCommandPanel/TriggerEditor；SendText 真发远端**已知 !Send 障碍**留待后续切片
| 切片 8 | ✅ 完成（受限） | 2026-07-31 | 5 个隧道 CRUD 薄壳 + TunnelPanel；direct-tcpip 未实现，IPC 接入即可 |
| 切片 9 | ✅ 完成（仅 IPC） | 2026-07-31 | 5 个插件薄壳 + PluginPanel；WasmSandbox 仍 scaffold，调用返回 kind=internal |

> 状态：⬜ 待开始 · 🟡 进行中 · ✅ 完成 · ⚠️ 受阻

---

## 已校核决策（2026-07-31 用户确认）

| 决策点 | 选择 | 落地方式 |
|--------|------|---------|
| 切片 1 真 SSH 目标 | **用现有开发服务器** | 切片 1 起步时向用户索取 host/port/username；不内置 docker sshd |
| 文档同步节奏 | **CLAUDE.md 实时改，docs/* 切片 2 集中改** | 每个含 CLAUDE.md 偏移的切片即时提交 doc fix；`docs/03/05/06/08` 在切片 2 统一收口 |
| 切片 9 范围 | **仅做 IPC 接入，运行时返回 not_implemented** | `enable_plugin` / `disable_plugin` / `plugin_command` 等薄壳接通；运行时返回 `IpcError { kind: "internal" }` |
| PR 粒度 | **一切片一 PR** | 切片 0 → PR-0；切片 1 → PR-1；依次；git history 线性可读，可独立回滚 |