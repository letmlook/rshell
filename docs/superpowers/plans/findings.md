# Findings — 勘察发现与待验证项

> 与 `2026-07-31-tauri2-frontend-backend-split-design.md` 对照，整理本仓库当前状态、可直接执行的修改面、以及需要在实施前/中验证的假设。

---

## 1. 设计文档的关键约束（必须遵守）

来自 §1.3 的边界铁律与 §3 / §4 / §5 / §6 的硬性要求：

1. `src-tauri/crates/**` 不得依赖 `tauri`；`rshell-api` 保持零运行时依赖（仅 serde + uuid）。
2. 前端 (`src/**`) 唯一入口：`invoke()` / `listen()` / `Channel`，禁止直达后端内部。
3. `src-tauri/src/**` 不得含业务逻辑，仅做参数转换、`AppCommand` 构造、`CommandOutcome` 解包、错误映射、Channel 路由。
4. 状态唯一所有者（§4.2）；终端尺寸是唯一双写项，**前端权威**。
5. 不变量（§6.3）：每个切片结束必须满足 cargo test 全绿 + 测试数不减少、clippy 零 warning、typecheck 零错误、`npm run test` 全绿、可运行、`ts-rs` 同步导出。

---

## 2. 仓库当前状态（基线 commit `6d2aac9` 之后的快照）

### 2.1 后端 Tauri 壳（`src-tauri/src/`）

| 文件 | 状态 | 备注 |
|------|------|------|
| `main.rs` | ✅ 就位 | 仅 `rshell_lib::run()` |
| `lib.rs` | ⚠️ 占位 | **仅一个 `hello` 命令**（21, 39 行），无 `state.rs` / `commands/` / `events.rs` / `terminal.rs` / `error.rs` |
| `state.rs` | ❌ 不存在 | 需新增 `AppState { Arc<CommandDispatcher> }` |
| `commands/` | ❌ 不存在 | 需新增 56 个薄壳（或宏批量） |
| `events.rs` | ❌ 不存在 | EventBus.subscribe → `app.emit_to` 桥 |
| `terminal.rs` | ❌ 不存在 | `TerminalChannels` 双态 sink 注册表 |
| `error.rs` | ❌ 不存在 | `CoreError → IpcError` 映射 |

### 2.2 后端业务 crate（`src-tauri/crates/`）

| crate | 路径 | 关键观察 |
|-------|------|---------|
| `rshell-api` | `src/{commands,events,types,lib}.rs` | `AppCommand` 56 变体、`AppEvent` 42 变体（§3.1 实测一致） |
| `rshell-core` | `src/{command_dispatcher,error,event_bus,…}.rs` | `command_dispatcher.rs:99` 有 `#[allow(clippy::arc_with_non_send_sync)]` 疤痕（§1.2）；`session/service.rs:169-253` 是 recv 循环（§2.1）；`session/repository.rs` **是死代码**（§4.5 实测） |
| `rshell-protocol` | SSH/Telnet/Serial/RDP | RDP 仅完成 X.224（CLAUDE.md "intentionally incomplete"），与本设计无关 |
| `rshell-infra` | crypto/storage/PTY | — |
| `rshell-plugin-sdk` | WASM sandbox（scaffold） | 与切片 9 对应，迁移不涉及 |

### 2.3 `rshell-api` 当前内容（与设计 §3 变更对照）

- `commands.rs` 56 个变体 → 切片 2 需新增 `CommandOutcome` 枚举（13 变体）
- `events.rs` 42 个变体 → 切片 2 需删除 9 个、收敛到约 33
- 死循环（`command_dispatcher.rs:420-435`）：`ListTriggers` / `ListQuickCommands` 算出数据后 `let _ =` 丢弃，仅发 `*Changed` → 前端再 invoke → 永远拿不到数据（D4）
- `types.rs` 包含 `TerminalBufferSnapshot` 及 `Cell`/`CellFlags` 系列 → 切片 2 全部删除（§2.2）

### 2.4 前端（`src/`）

| 路径 | 状态 | 处置 |
|------|------|------|
| `ipc/{types,client,events}.ts` | ✅ 保留（§10.2） | 框架无关，含手工镜像 547 行（切片 2 起由 `ts-rs` 取代） |
| `App.tsx` `main.tsx` `components/*.tsx` `views/*.tsx` `store/*.ts` `styles/tokens.css` | ❌ 删除（§10.4） | React → Vue 3 + Element Plus + pinia + dockview-vue |
| `package.json` | 需调整（§10.3） | 移除 react/zustand/@vitejs/plugin-react；新增 vue/element-plus/pinia/dockview-vue/@xterm/addon-{search,webgl}/vitest/@vue/test-utils/jsdom 等 |
| `vite.config.ts` `tsconfig.json` `index.html` | 需调整 | react → vue 插件；jsx 配置；typecheck 脚本改为 `vue-tsc --noEmit` |

### 2.5 `src-tauri/.gitignore` 已确认的 bug（§7.2 顺带修）

嵌套 `.gitignore` 路径相对其所在目录解析 → `src-tauri/target/` 实际未被忽略，构建后 `git status` 被污染。修复：`src-tauri/.gitignore` 中 `src-tauri/target/` → `target/`。

---

## 3. 已由设计文档提前证伪/确认的假设

| # | 项 | 状态 | 来源 |
|---|----|------|------|
| 1 | `rhai = { features = ["sync"] }` 可行；`assert_send_sync::<CommandDispatcher>()` 通过；`cargo test -p rshell-core` 74/74 | ✅ 已实测 | 设计 §7.2 step 1 |
| 2 | `dockview-vue` v7.0.4 支持 Vue 3 + `<script setup>` | ✅ 已确认 | 设计 §10.5 |
| 3 | xterm.js 框架无关 | ✅ 已确认 | 设计 §10.5 |
| 4 | Element Plus 2.14.3 | ✅ 已确认 | 设计 §10.5 |
| 5 | `SessionRepository` 是死代码，阻塞切片 1 | ✅ 已实测 | 设计 §4.5 |
| 6 | 触发器 (`trigger_engine.rs:89`) 使用原始字节而非 alacritty 缓冲 → §2 删除安全 | ✅ 已实测 | 设计 §2.1 |
| 7 | `recv` 循环 16ms ticker + dirty 标志只为节流发 `TerminalBufferUpdated` | ✅ 已实测 | 设计 §2.1 |

---

## 4. 待实施前/中验证的假设

| # | 项 | 验证时机 | 验证方式 |
|---|----|---------|---------|
| V1 | 切片 0 步骤 3：1 MB 假数据经 `Channel<Vec<u8>>` 推 xterm.js 的吞吐基线 | 切片 0 完成前 | 启动前端，注入字节，记录 fps / lag；输出数字写入 progress.md |
| V2 | 切片 0 步骤 4：`rhai` sync feature 在 `mockall`/测试 mock 下仍保持 `Send + Sync` | 切片 0 完成前 | `assert_send_sync` 全工作区断言通过 |
| V3 | 切片 1：`attach_terminal` 早于首字节到达 → `Buffering → Attached` 切换首屏提示符不丢 | 切片 1 完成判据 | 手动连接 SSH，看 banner / 首 prompt |
| V4 | 切片 1：进程重启后会话仍在 | 切片 1 完成判据 | 启动 → 创建会话 → 退出 → 重启 → `list_sessions()` 仍可见 |
| V5 | Vite HMR 期间 `Channel.send` 返回 `Err` → 退回 `Buffering`，SSH 不断 | 切片 1 内 | 触发 HMR，观察连接是否存活 |
| V6 | `@xterm/addon-webgl` 在 Windows WebView2 / Linux WebKitGTK 下无 `onContextLoss` | 切片 1 | 拖窗 / resize 验证；fallback 到 canvas 路径就绪 |
| V7 | `ts-rs` 导出可覆盖手工 `types.ts` 的 547 行，零偏差 | 切片 2 完成前 | 生成后逐文件比对 → git diff 应为空 |

---

## 5. 风险登记

| ID | 风险 | 触发条件 | 缓解 |
|----|------|---------|------|
| R1 | `rhai sync` 编译失败（与某 crate 版本不兼容） | 切片 0 步骤 1 重测 | 退回 "ScriptEngine 独立 actor" 方案（设计 §1.2 降级路径） |
| R2 | dockview-vue v7.0.4 与 Vue 3.5 / TS 严格模式不兼容 | 切片 0 装包 | 锁定 dockview-vue 至最新稳定；用 `<script setup>` 起步 |
| R3 | `Channel<Vec<u8>>` 高吞吐时 WebView2 / WebKitGTK 缓冲上限未知 | 切片 0 步骤 3 实测 | 测量后写基线；必要时切片 2 引入分片（每帧 ≤ 64 KiB） |
| R4 | `ts-rs` 导出与 `serde(rename_all)` 不一致导致 TS 字段名漂移 | 切片 2 引入 | 与 `client.ts` PascalCase → snake_case 映射交叉验证 |
| R5 | 56 个 `#[tauri::command]` 薄壳的宏展开遗漏某些参数解包 | 切片 1 | 每个薄壳至少 1 个单元测试；不变量 1 强约束 |
| R6 | 删除 §2.2 清单时漏删 `dirty` 标志导致 recv 循环半残 | 切片 2 | 切片 1 已能跑通 → 切片 2 只删不再能跑通的；grep `dirty` / `get_buffer_snapshot` / `process_output` 三处必清 |
| R7 | `HostKeyDecisionRegistry` 当前是断链（设计 §4.3 B 流程注释） | 切片 4 | 切片 1 连接非首次主机即撞上，必须在切片 4 闭环 |
| R8 | Vitest + `@vue/test-utils` 在 Vue 3.5 + Vite 5 下 `import.meta.env` / CSS 注入需 stub | 切片 0 | 使用 `jsdom` 环境；最小 store 单测起步 |
| R9 | RDP / 插件 / 隧道相关切片 8/9 既有未实现项（CLAUDE.md "intentionally incomplete"） | 切片 8/9 | 不强求闭环；只把 IPC 接入即可 |
| R10 | CLAUDE.md 中 `Result<T, String>`、`TerminalBufferUpdated`、`crates/rshell-ui/` 等 7 处描述需同步 | 切片 2 完成后 | 集中编辑一次；提交独立 `docs(core)` commit |

---

## 6. 文档同步清单（设计 §8）

| 文档 | 偏移 | 何时改 |
|------|------|--------|
| `CLAUDE.md:96` | `TerminalBufferUpdated` 经 Channel → 改为原始字节经 Channel | 切片 2 完成（**CLAUDE.md 实时改** — 见决策表） |
| `CLAUDE.md:101` | `Result<T, String>` → `IpcError` | 切片 1 引入 `error.rs` 时（**CLAUDE.md 实时改**） |
| `CLAUDE.md:86` | "推入 channel" → "直接 dispatch" | 切片 1（**CLAUDE.md 实时改**） |
| `CLAUDE.md:121` | `crates/rshell-ui/` 待删除 → 已删除 | 切片 0 起步前确认 |
| `docs/03-detailed-design.md` | GPUI 残留 | **切片 2 集中改** |
| `docs/05-development-standards.md` | GPUI 残留 + §2 重述 | **切片 2 集中改** |
| `docs/06-test-strategy.md` | 增 Vitest + tauri-driver E2E 章节 | **切片 2 集中改** |
| `docs/08-incomplete-features.md` | #2 交叉引用切片 4 | 切片 4 起步 |

## 6.1 PR 粒度（已校核）

**一切片一 PR**：PR-0 / PR-1 / ... / PR-9，线性 history，独立可回滚。切片 0/1/2 强耦合（前端换栈 + IPC + 旧桥清理），按设计顺序提交，不强合并。

## 6.2 切片 1 真 SSH 验证（已校核）

**用现有开发服务器**，不内置 docker sshd。切片 1 起步时向用户索取 host/port/username/password。设计 §6.2 的 `tests/fixtures/docker-compose.yml` 推迟到切片 5 或更后，按 E2E 实际需要再起。

## 6.3 切片 9 范围（已校核）

**仅做 IPC 接入，运行时返回 not_implemented**：
- 薄壳：`list_plugins` / `enable_plugin` / `disable_plugin` / `plugin_command`
- 实际加载仍走 `WasmSandbox` scaffold；调用返回 `IpcError { kind: "internal", message: "plugin sandbox not yet implemented" }`
- 前端 `PluginPanel.vue` UI 完整；启用/禁用状态可写入持久化（`infra::storage`）以便重启可见

---

## 7. 不在本文档范围但相关（CLAUDE.md 已记）

- RDP TLS upgrade / NLA / ActiveStage frame pump（`rshell-protocol/rdp/mod.rs`）—— 与本设计无关
- WasmSandbox 真实集成（`rshell-plugin-sdk`）—— 切片 9 仅做 IPC 接入
- 自动生成 TS 类型（`specta`）远期规划——本设计先上 `ts-rs`

## 8. 切片 2.3 实际落地状态（2026-07-31）

**已完成**：
- ts-rs 12 + uuid-impl/serde-compat features 加入 `src-tauri/Cargo.toml` workspace deps
- `rshell-api/Cargo.toml` 引用 `ts-rs`
- 导出路径约定 `#[ts(export_to = "../../../src/ipc/generated.ts")]` 已写在模块注释

**未完成**（设计 §3.6 目标，全量 derive 推迟）：
- 全量 `#[derive(TS)]` 需要 `types.rs` 中所有 ~30 个公共结构体同步 derive —— 因依赖图深度耦合（SessionConfig → AuthMethod → 含 Uuid/PathBuf/Vec 等），单一顶层 derive 会拉一片错误。
- 解决路径：按功能域（slice 3 起）在每个功能域引入时,顺手 derive 该域内用到的所有 types.rs 结构体。

**CI 护栏**：
- 当前未挂 `git diff --exit-code src/ipc/generated.ts` —— 文件还没生成
- 切片 3+ 第一个完成的功能域（如 `session CRUD`）应当 derive SessionConfig/AuthMethod/ConnectionState/ConnectionInfo 等,触发首次 `generated.ts` 产物后,即可挂上 CI 护栏

## 9. 切片 2.5 文档同步状态（2026-07-31）

**CLAUDE.md 已实时改**（按用户校核）：
- §8 §86 "React" → "Vue 3"；中间 channel → 直接 dispatch；事件名 `"app://event"` → `"rshell://event"`；移除第 3 步专用后台线程描述（已由 D5 取代）
- §101 `Result<T, String>` → `Result<T, IpcError>` + §3.5 注释
- §9 整体重写为"Vue 3 + Channel<Vec<u8>> 高频路径专用通道"描述
- §121 标注 Tauri 迁移进度（切片 0/1/2 累计完成、后续切片 3~9 待迁移）

**docs/03/05/06/08 推迟**（用户校核"切片 2 集中改"，但批量 GPUI 残留清理需要在切片 3+ 业务视图实现时一并改,避免文档与代码漂移）：
- `docs/03-detailed-design.md`：待切片 3 会话 CRUD 实施时清
- `docs/05-development-standards.md`：同上
- `docs/06-test-strategy.md`：Vitest/tauri-driver 章节在切片 5 (SFTP) 引入 E2E 时补
- `docs/08-incomplete-features.md`：host key 闭环在切片 4 实施时交叉引用