# RShell 问题清单

> **体检日期**：2026-09-30
> **基线 commit**：`7258309e7852`（完整哈希 `7258309e7852b2d7e83046ae1160f32124089809`，feat: wire transfer pause and resume controls）
> **体检方式**：四个审查方向（正确性审查、安全审查、契约审查、构建与文档审查）+ 基线检查，详见第一节。

## 一、概述

**问题总数与分布**：本次体检共立条 **30** 个问题——**high 3 条**（PROB-01～PROB-03）、**medium 11 条**（PROB-04～PROB-14）、**low 16 条**（PROB-15～PROB-30）。另记录**暂缓项 2 条**（第三节，均已证实成立、超出本轮修复容量）与**经复核未单独立条的原始发现 12 条**（第四节，多数并入已立条问题，其中 1 处子断言复核未能证实、留待人工确认）。PROB-01/02/03 为「门槛级」问题——它们决定后续检查能否作为验收门槛，应最先修复。

**修复结果统计**（2026-09-30 修复轮）：30 条问题**已修复 30 条，未解决 0 条**——high 3 条（PROB-01～03）、medium 11 条（PROB-04～14）、low 16 条（PROB-15～30）全部修复，各条目「状态」行已更新。验证方式：各问题先经 fmt/clippy/typecheck 快速检查，再以最终全量验证统一收口——`bash scripts/verify.sh --skip-install` 全部通过（typecheck、前端单测 21 文件/87 用例、生产构建、docs/bundle 契约检查、脚本自检 37 用例、cargo fmt/clippy、cargo test 全 workspace 231 用例；详见第五节）。第三节暂缓项 2 条与第四节未立条发现 12 条不随本轮关闭，状态不变。

**体检方式**：四个审查方向 + 基线检查。

- **正确性审查**：后端状态机与运行时行为（传输、会话、隧道、SSH 客户端、脚本引擎等）；
- **安全审查**：含密钥与凭据存储、主密码、主机密钥信任、隧道暴露面、Tauri capabilities 与配置最小权限、unsafe 与 WASM 插件沙箱、命令参数语义等专项；
- **契约审查**：前端 `invoke` 参数键名与后端命令签名、TS 类型镜像、事件契约的一致性；
- **构建与文档审查**：构建脚本与依赖、tauri.conf.json、docs 文档与 HEAD 实现的一致性。

**基线检查与核实方式**（均于基线 commit `7258309e7852` 上进行）：

- 全量前端单测实跑：`npm test` → Test Files 20 passed (20)、Tests 80 passed (80)。本文档撰写时于同一 commit 复跑一次，结果一致（20/20 文件、80/80 用例全绿）。该全绿结果与 PROB-01/02/03 并存，本身构成 PROB-27 所述「系统性假绿」的证据；
- 依赖版本核对：Cargo.lock 锁定 tauri 2.11.5 / tauri-macros 2.6.3 / russh 0.48.2 / ssh-key 0.6；node_modules 实测 dockview-vue 7.0.4；
- 关键第三方源码实读：本机 cargo registry 中 tauri-macros 2.6.3 的 `command/wrapper.rs`、tauri 2.11.5 的 `ipc/command.rs`、wry 0.55.1 的 WKWebView UIDelegate、russh 0.48.2 的 `client/mod.rs`；node_modules 中 `@tauri-apps/api/core.js`、`@tauri-apps/plugin-fs` dist 与 dockview-vue dist 产物；
- 权限对账：python 解析 `gen/schemas/acl-manifests.json`，与前端对 plugin-fs 的实际调用逐项对账；
- git 取证：`git show --stat 7258309` 证实该 commit 仅改动 `src/App.vue`、`src/components/TransferPanel.vue` 与两个 spec，未更新 docs/08、docs/09。

本文档撰写时对上述证据做了抽查复核（`git rev-parse HEAD`、`git show --stat 7258309`、`npm test` 复跑，以及对各问题引用的源码位置逐一 grep/sed 核对），抽查结果与体检结论一致。

## 二、问题清单

### PROB-01：前端 invoke 全部用 snake_case 键名，而 Tauri 2 命令默认要求 camelCase，几乎所有参数化 IPC 调用在参数提取阶段即失败

- **位置**：`src/ipc/client.ts:75`
- **严重度**：high
- **问题描述与证据**：门槛级（决定后续检查能否作为验收门槛）。证据链全部核实：`src/ipc/client.ts` 中所有参数化 invoke 均用 snake_case 键（`connectSession` 发 `{ session_id }` :75-76，`sendInput` :83-84，`pauseTransfer` :110，`trustHostKey` :189-198，`setAppTheme` :206 等），grep 统计 client.ts 中多词 snake_case 键恰为 54 处；`src-tauri/src/commands.rs` 全文件无 rename_all（grep 仅 `error.rs:16` 一处 serde 枚举），cmd! 宏模板 commands.rs:47 也是裸 `#[tauri::command]`，`connect_session(session_id: Uuid,...)` 在 :77。依赖源码实读（Cargo.lock 锁定 tauri 2.11.5 / tauri-macros 2.6.3）：`~/.cargo/registry/.../tauri-macros-2.6.3/src/command/wrapper.rs:51` 默认 `ArgumentCase::Camel`、:506-507 把参数键转 `to_lower_camel_case()`；`tauri-2.11.5/src/ipc/command.rs:96-103` 用 `v.get(self.key)` 精确匹配、不命中即报 "missing required key"；`node_modules/@tauri-apps/api/core.js:201` invoke 原样透传。`lib.rs:143` 为标准 generate_handler。唯一风格正确的调用点是 `TerminalPane.vue:157-160` 直接 `invoke("attach_terminal", { sessionId, onData })`。结论：connect_session/disconnect_session/send_input/pause_transfer 等几乎所有参数化命令必然以 missing key 被拒；单词参数与无参命令不受影响。
- **修复方案**：二选一并写进契约：给 commands.rs 全部 `#[tauri::command]`（含 :47 的 cmd! 宏模板）加 `rename_all = "snake_case"`，或前端统一改传 camelCase。
- **验收标准**：①新增对账测试遍历 client.ts 全部 helper 与 commands.rs 命令签名断言参数键一致，注入错误键名时测试失败；②`tauri dev` 真实完成一次 连接→终端输入→SFTP 传输→暂停/取消 全链路。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-02：DockviewVue 从未通过 @ready/addPanel 创建面板，TerminalPane 在真实应用中永不挂载，核心终端能力不可达

- **位置**：`src/App.vue:340`
- **严重度**：high
- **问题描述与证据**：门槛级。`src/App.vue:340-349` `<DockviewVue v-if="activeTerminal" :components="components">` 只渲染 #terminal 插槽（:343-345）；grep src/ 无 onReady/@ready/addPanel（仅 :22,:25 的 import）。实测 `node_modules/dockview-vue/dist/dockview-vue.es.js`（7.0.4）："slots" 出现 0 次，渲染只输出容器 div + 按 registry.entries 的 Teleport（:312,:534,:688,:711），registry 仅在 dockview core 经 API 创建面板时填充——真实库不渲染插槽，选中会话后终端区域是空白容器。`npm test` 本次实跑（Test Files 20 passed, Tests 80 passed）全绿，因 tests/unit/AppLayout.spec.ts:58 用 stub 把插槽直接渲染出来（见 PROB-27）。
- **修复方案**：为 DockviewVue 加 `@ready`，在回调中按 activeTerminal 调 `event.api.addPanel({ id, component: 'terminal', params: { sessionId } })`（或去掉 dockview 直接挂 TerminalPane）；同步修正 AppLayout.spec 的桩。
- **验收标准**：dev 下选中会话后终端可见可交互；桩测试改为不渲染插槽并断言 addPanel 调用。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-03：window.confirm/prompt 在 wry/WKWebView 中无实现：删除会话、删除密钥永远不执行，带口令导入的口令输入恒为 null

- **位置**：`src/components/SessionList.vue:123`
- **严重度**：high
- **问题描述与证据**：门槛级。`SessionList.vue:123` `confirm(\`确定删除会话…\`)` 恒 false → store.delete 永不执行且无提示；`KeyManagerPanel.vue:88` `window.confirm` 恒 return → deleteSshKey 不可达；`KeyManagerPanel.vue:58` `window.prompt` 恒 null → 加密私钥以空口令调用 importPrivateKey。依赖证据实读本机 cargo registry：wry-0.55.1 `src/wkwebview/class/wry_web_view_ui_delegate.rs`（全文 282 行）只实现 will_close/run_file_upload_panel/request_media_capture_permission/create_web_view_for_navigation_action，无 JavaScript confirm/alert/text-input 面板方法；WKWebView 在 WKUIDelegate 未实现这些方法时 confirm 静默返回 false、prompt 返回 null。单测掩盖问题：tests/unit/KeyManagerPanel.spec.ts:17,:29 桩化 window.prompt/confirm。仓库内正确范例：`TransferWorkspace.vue:149` 用 @tauri-apps/plugin-dialog 的异步 confirm；`QuickCommandPanel.vue:75` 用 ElMessageBox.confirm。
- **修复方案**：三处统一改用 @tauri-apps/plugin-dialog 的 confirm/ask（capabilities 已含 dialog:default/allow-message）或 ElMessageBox；口令输入做真正的密码对话框（参照 SessionCredentialDialog.vue 的 el-dialog）。
- **验收标准**：dev 真实执行 删除会话/删除密钥/带口令导入 各一次成功；单测改 mock plugin-dialog/ElMessageBox 而非 window.confirm/prompt。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-04：传输 pause/resume/cancel 都是静默 no-op（可见按钮是假状态，终态被无条件覆盖），docs/08、docs/09 的「无界面入口」说明同时与 HEAD 矛盾

- **位置**：`src-tauri/crates/rshell-core/src/transfer/service.rs:419`
- **严重度**：medium
- **问题描述与证据**：pause_transfer（transfer/service.rs:419-433）、resume_transfer（:436-450）、cancel_transfer（:453-467）只改任务表状态并广播 TransferQueueChanged；执行传输的 spawn 任务（:330-413）全程不读任务状态，结束时在 :389-412 无条件改写为 Completed/Failed 并广播 TransferCompleted/TransferFailed。协议层拷贝循环 copy_with_progress（`rshell-protocol/src/ssh/sftp.rs:276-318`）只有 read/write/progress 回调，grep sftp.rs 无任何 pause/cancel 检查。UI 已接线：`App.vue:391-392` @pause/@resume → runTransferAction（:95-118 调 pauseTransfer/resumeTransfer），`TransferPanel.vue:206-221` 渲染可见「暂停/继续」按钮。用户点暂停后 UI 显示可继续的 Paused 任务，但字节持续推进、结束时被改回 Completed——「已暂停/已取消」是假状态。文档漂移同根因：docs/08:23「本轮不提供界面入口，paused 状态在界面上不可达；不以占位按钮冒充实现」、docs/09:26「pause/resume 核心接口无界面入口已在功能与限制中记录」——`git show --stat 7258309` 实证该 commit 只改 src/App.vue、TransferPanel.vue 和两个 spec，未动 docs，两文档与 HEAD 直接矛盾。
- **修复方案**：给传输任务接按 task_id 的取消/暂停信号（CancellationToken 或 watch channel）传入传输循环，在 copy_with_progress 每分块前检查；cancel 与 pause 共用该通道，终态写回前先检查任务是否已被标记 Cancelled 并保持；同步改写 docs/08:23 与 docs/09:26。
- **验收标准**：集成测试中大文件上传后 pause——字节停止增长、状态保持 Paused、不广播 TransferCompleted；resume 后继续；cancel 后保持 Cancelled 且无 TransferCompleted；docs 与实现一致。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-05：VerifyMasterPassword 分支丢弃 verify() 的 bool 固定返回 None，已注册命令任何调用（密码对错皆然）都报 outcome_mismatch

- **位置**：`src-tauri/crates/rshell-core/src/command_dispatcher.rs:434`
- **严重度**：medium
- **问题描述与证据**：command_dispatcher.rs:434-437 `self.master_password.verify(&password).await?; Ok(CommandOutcome::None)`——verify 签名是 `Result<bool, CoreError>`（master_password.rs:90 实读确认），bool 被 `?` 丢弃。薄壳 commands.rs:721-724 期望 `CommandOutcome::Verified(b)` 否则 IpcError::outcome_mismatch。grep 全仓库 `CommandOutcome::Verified` 仅 outcome.rs 测试（:94,:188,:203）构造，生产代码从不构造。命令已注册（lib.rs:158）并在 client.ts:182-183 导出为返回 boolean。
- **修复方案**：dispatcher 改为 `let ok = self.master_password.verify(&password).await?; Ok(CommandOutcome::Verified(ok))`。
- **验收标准**：单测对已设主密码正确口令得 true、错误口令得 false，不再报 outcome_mismatch；client.ts verifyMasterPassword 返回正确布尔。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-06：TrustHostKey 命令用 .. 丢弃 decision 参数：传 Reject/TrustOnce 也会把主机密钥永久写入 known_hosts；且该命令绕过决策链、前端无调用方，是潜伏的无约束信任写入面

- **位置**：`src-tauri/crates/rshell-core/src/command_dispatcher.rs:449`
- **严重度**：medium
- **问题描述与证据**：command_dispatcher.rs:449-460 分支 `AppCommand::TrustHostKey { host, port, key_type, public_key_blob, .. }` 无条件调 host_key_manager.trust_host_key 落盘；TrustHostKeyDecision::TrustOnce/Reject（`rshell-api/src/types.rs:399-406`，注释明确「仅本次有效，不写入 known_hosts」「拒绝」）语义完全未消费。薄壳 commands.rs:424-444 接收 decision 并透传，lib.rs:177 注册，client.ts:189-198 导出。grep src/ 确认前端无 trustHostKey 调用方——当前是潜伏契约缺陷，但一旦调用方传 Reject，可能被怀疑为 MITM 的密钥即被永久持久信任。另外该命令不经 DecideHostKey 决策链（正规流程 command_dispatcher.rs:461-509 要求 resolve 决策后写入）。
- **修复方案**：分支内按 decision 分派：Reject 返回错误或 Ok 不落盘，TrustOnce 不写 known_hosts，仅 TrustPermanent 走 trust_host_key；或直接删除该命令、dispatcher 分支与前端导出（手动导入信任须关联 decision_id 二次确认）。
- **验收标准**：单测三种 decision 的落盘行为各断言一次（Reject/TrustOnce 不产生 known_hosts 条目）；grep 无无条件信任路径。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-07：SSH 客户端 inactivity_timeout=30s 且无 keepalive：双向静默 30 秒的会话（只读输出、无输出长命令）被强制断开且无提示

- **位置**：`src-tauri/crates/rshell-protocol/src/ssh/client.rs:467`
- **严重度**：medium
- **问题描述与证据**：`rshell-protocol/src/ssh/client.rs:467-469` `inactivity_timeout: Some(Duration::from_secs(30)), ..Default::default()`；grep src-tauri 无任何 keepalive 配置。russh 0.48.2（Cargo.lock 锁定；本机 registry 源码实读）：client/mod.rs:898-903 同时起 keepalive_timer 与 inactivity_timer，keepalive_interval 默认 None 时 keepalive_timer 永远 pending；:1028-1041 仅当 received_data || sent_keepalive 才重置 inactivity timer——空闲 30s 即返回 InactivityTimeout。session/service.rs 读循环在数据流结束后把会话置 Disconnected（:552-570 区域实读），用户表现为会话无提示掉线。
- **修复方案**：配置 keepalive_interval（如 15-30s，keepalive_max 默认 3）让空闲保活，inactivity_timeout 调大为分钟级或去掉；若 30s GC 是有意设计须与设计文档确认并写入 docs/08。
- **验收标准**：真实 SSH 会话静默 >5 分钟不断线；集成测试模拟空闲会话存活。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-08：会话断开后隧道不关闭也不改状态：listener 继续接受连接并逐一失败，TunnelState 恒 Active，UI 展示实际已死的隧道

- **位置**：`src-tauri/crates/rshell-core/src/security/tunnel_manager.rs:271`
- **严重度**：medium
- **问题描述与证据**：create_tunnel 把创建时的 SshClientHandle clone 进 listener 任务（tunnel_manager.rs:268-331）；SessionService::disconnect/detach_session（session/service.rs:763-818 实读）与 TunnelManager 无任何关联（grep session/service.rs 中 tunnel 仅出现在 lib.rs:1462 组装处）。close_tunnel（:357-390）仅由 CloseTunnel 命令触达（command_dispatcher.rs:321）。SSH 断开后 disconnect_ssh 已 handle.take()（`rshell-protocol/src/ssh/client.rs:688` 实读），旧 client Arc 上每条新 inbound 连接 open_direct_tcpip 失败仅 warn，隧道状态与 TunnelStateChanged 始终是 Active；重连后仍挂在旧 Arc 上不会自愈。
- **修复方案**：会话断开时由 SessionService 或 dispatcher 按 session_id 关闭（或标记 Error）对应隧道（或 listener 持 watch channel 感知连接失效），并发布 TunnelStateChanged。
- **验收标准**：集成测试建隧道→断开会话→TunnelState 变为非 Active 且 listener 停止接受新连接；重连后旧隧道不复活。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-09：KeyManager 启动时不加载 keys 目录：重启后所有已生成/导入密钥从列表消失，磁盘留下无法再管理的明文私钥孤儿文件

- **位置**：`src-tauri/crates/rshell-core/src/security/key_manager.rs:43`
- **严重度**：medium
- **问题描述与证据**：KeyManager::new（key_manager.rs:43-52）只 create_dir_all 不读目录；list_keys/get_private_key_data 只查内存 HashMap（:285-309）；generate/import 把私钥写入 `keys_dir/{id}.key`（:136-139、:227-230）却无人读回。delete_key（:258-272）只在内存命中时删文件，重启后找不到 key_id 只 warn。keys_dir 由 lib.rs:66-67 指向 data_local_dir/rshell/keys。附加核实：get_private_key_data 在 crates 内无任何调用方——生成的密钥当前也不参与 SSH 认证，整条密钥链路是半成品。
- **修复方案**：构造时扫描 `keys_dir/*.key` 解析公钥/指纹重建内存索引；或改为私钥不入盘、只存 Keychain，二选一保持一致；删除时同步清理磁盘。
- **验收标准**：生成密钥→用同一 keys_dir 新建 KeyManager→list_keys 非空且指纹一致；delete 后磁盘无 .key 残留。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-10：SSH 私钥明文落盘且无 0600 权限（同机其他用户/进程可读），generate/import 的 passphrase 参数被静默忽略、has_passphrase 恒 false

- **位置**：`src-tauri/crates/rshell-core/src/security/key_manager.rs:137`
- **严重度**：medium
- **问题描述与证据**：generate_key/import_private_key 用 tokio::fs::write 写 `keys_dir/{id}.key`（key_manager.rs:137-139、:227-230），默认权限 0666&~umask；grep src-tauri/crates 的 set_permissions/PermissionsExt 仅 repository.rs:453-467 测试代码，无任何 0600 收紧。passphrase 被忽略：generate_key(name, key_type, _passphrase)（:55-60）、import_private_key(path, _passphrase)（:169-173），PrivateKey::random 不加密，has_passphrase 恒 false（:131、:222），而 IPC 契约接受 passphrase（`rshell-api/src/commands.rs:174-183`；client.ts:168-171 导出 `passphrase: string | null`）——传口令即静默成功。明文私钥对同机其他用户/进程可读，与 docs/08:21「凭据走 Keychain」的路线相悖。更正原始发现中一处未证实子断言：「带口令的加密私钥在 from_openssh(:181) 导入直接失败」未能在代码中证实（ssh-key 0.6 可解析加密 PEM）；实际缺陷是后续无任何解密路径、该密钥不可用且 has_passphrase 错标 false。定级为 medium 而非 high 的原因：需要本机其他账户/进程访问用户数据目录这一前置条件，且当前 get_private_key_data 无调用方（密钥尚未被消费）。
- **修复方案**：写入后立即 set_permissions(0o600)（unix PermissionsExt）/等效 ACL；用主密码派生密钥或 Keychain 加密存储私钥、盘上只留指纹元数据；本轮若不实现加密，命令层对非空 passphrase 显式报错而非静默忽略，并如实设置 has_passphrase。
- **验收标准**：生成的 .key 权限为 0600；传 passphrase 的调用要么真实加密（has_passphrase=true 且口令可解密）要么返回明确错误；grep 无静默忽略路径。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-11：capabilities fs 授权与前端实际使用错配：漏授 fs:allow-stat（本地文件大小/时间被 ACL 拒绝后静默回退为 0/空），同时授予一批从未调用的写/删/改名权限与 dialog:allow-save

- **位置**：`src-tauri/capabilities/default.json:20`
- **严重度**：medium
- **问题描述与证据**：capabilities/default.json:20-29 授予 fs:allow-read-text-file/read-file/write-text-file/write-file/exists/mkdir/remove/rename/read-dir，无 allow-stat。前端对 plugin-fs 的唯一使用是 `FileBrowserPane.vue:14` `import { readDir, stat }`。实测 python 解析 gen/schemas/acl-manifests.json：fs.default_permission 仅 [create-app-specific-dirs, read-app-specific-dirs-recursive, deny-default]（不含 stat），allow-stat 存在于权限清单但未授予；`node_modules/@tauri-apps/plugin-fs/dist-js/index.js:583` 确认 stat() 调 plugin:fs|stat。因此 FileBrowserPane.vue:102 `stat(fullPath).catch(() => null)` 恒被 ACL 拒绝且被吞掉，本地文件大小恒 0、修改时间恒空（假数据回退）。反向：写/删/改名（含破坏性 fs:allow-remove）与 dialog:allow-save（:32）均无前端调用路径（grep src/ 证实；远程删除走自有 IPC delete_remote_entry），属未使用攻击面。
- **修复方案**：权限收敛为 fs:allow-read-dir + fs:allow-stat + dialog:allow-open/allow-message，删除 write-*/remove/rename/mkdir/exists/read-file/read-text-file 与 dialog:allow-save；FileBrowserPane 去掉对 ACL 拒绝的静默 catch，非 ENOENT 错误显示真实错误。
- **验收标准**：本地浏览的文件大小/修改时间正确显示；capabilities 中不存在无前端调用路径的 fs 写/删权限；tauri dev 无权限告警。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-12：密钥面板生成类型下拉提供 "RSA"/"ECDSA"，后端 SshKeyType 枚举无此变体，选这两项生成必然失败

- **位置**：`src/components/KeyManagerPanel.vue:118`
- **严重度**：medium
- **问题描述与证据**：KeyManagerPanel.vue:117-121 选项为 ED25519/RSA/ECDSA，genType（:37 `ref<SshKeyType>`）在 generate()（:67-75）直接作为 key_type 发送。后端 SshKeyType（`rshell-api/src/types.rs:354-361`）只有 RSA2048/RSA4096/ED25519/ECDSA256/ECDSA384/ECDSA521（普通外部标签枚举无 rename）；TS 镜像 types.ts:225-231 同。选 RSA/ECDSA 时 serde unknown variant，generate_ssh_key 失败（错误可见，非静默）。
- **修复方案**：下拉改为与 SshKeyType 完全一致的六个值（或映射到具体位数），用 TS 类型约束选项值。
- **验收标准**：六项逐一在真实后端生成成功。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-13：终端面板错误只写 console：attach_terminal 失败用户面对空白终端，sendInput 失败按键静默丢失，无任何界面反馈

- **位置**：`src/components/TerminalPane.vue:161`
- **严重度**：medium
- **问题描述与证据**：TerminalPane.vue:161-163 attach_terminal catch 仅 console.error，无 error ref/提示 UI；:169-171 sendInput 失败仅 console.error（按键被丢弃）；:187-189 resizeTerminal 失败仅 console.warn。对照仓库约定：`App.vue:159` 连接失败弹 ElMessage.error。
- **修复方案**：面板内渲染错误状态条（如「终端连接失败：…请重连」），sendInput 持续失败时给出一次性可见提示。
- **验收标准**：人为使 attach 失败时终端区域显示错误状态而非空白；不存在仅 console 的用户可见故障路径。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-14：主密码功能半接线：MasterPasswordRequired 事件后端从不发布、前端不订阅，设置主密码对话框不可达；对话框文案声称加密私钥与实现和 docs/08 矛盾（虚假安全声明）

- **位置**：`src/stores/sessions.ts:29`
- **严重度**：medium
- **问题描述与证据**：MasterPasswordDialog 可见性只绑定 masterPasswordRequired（MasterPasswordDialog.vue:13；sessions.ts:29 定义），但 sessions.ts:116-131 subscribeEvents 只处理 ConnectionStateChanged 与 SessionListChanged，从不置位；grep 后端 MasterPasswordRequired 仅 `rshell-api/src/events.rs:86` 枚举定义，无 emit 点——对话框永不弹出，setup_master_password 经 UI 不可达（死契约）。且 MasterPasswordDialog.vue:49 文案「主密码用于加密本地 SSH 私钥。…丢失后将无法恢复已存储的密钥」与实现矛盾：master_password.rs setup（:55-88 实读）只在内存派生密钥并加密固定验证令牌，不持久化任何用户数据；encrypt_data/decrypt_data（:186、:203）grep 全仓库无调用方；docs/08:21 明确「主密码仅用于应用锁定/验证，不是凭据保险库，也不加密钥匙串」。
- **修复方案**：二选一：后端在需要时真正 emit MasterPasswordRequired、store 订阅置位，且先让主密码真实加密私钥后再保留文案；或删除死事件与对话框接线，文案改为与 docs/08 一致（仅用于应用锁定/验证）。
- **验收标准**：要么对话框可经真实事件弹出且主密码确实加密私钥，要么死代码与虚假文案被移除、UI 文案与 docs/08 一致。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-15：WASM 沙箱 execute_async 把 &self 裸指针强转 &'static 移入 spawn_blocking（潜在 use-after-free 的 unsound unsafe）；String 参数经 From<WasmValue> 被静默折叠为 I32(0)

- **位置**：`src-tauri/crates/rshell-plugin-sdk/src/sandbox.rs:307`
- **严重度**：low
- **问题描述与证据**：sandbox.rs:307-310 实读：`let sandbox = self as *const _ as usize; let sandbox: &'static WasmSandbox = unsafe { &*(sandbox as *const WasmSandbox) };` 后 move 进 spawn_blocking，无任何机制保证 WasmSandbox 存活到任务运行，插件 unload 与执行竞态即 use-after-free；今日仅因 PluginLoader 持 Arc 全程存活且 grep 确认 execute_async 全仓库无调用方（死代码）而未爆。另 From<WasmValue> for Val 对 String 返回 Val::I32(0)（:85-93，注释称占位、真正路径是 marshal_string_input），经 execute_async 传字符串参数会被静默折叠为 0。
- **修复方案**：execute_async 改为接收 `self: Arc<WasmSandbox>` 并 clone Arc 进闭包，删除指针强转（或直接删除该死方法）；字符串序列化未实现前对 String 参数显式返回错误。
- **验收标准**：无 `&self`→`&'static` 转换残留（cargo clippy 通过）；若有调用方，字符串参数不再折叠为 0。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-16：SshClient 的 Connection::recv 把超出 buf 长度的数据静默丢弃（trait 契约级数据丢失点，当前不可达）

- **位置**：`src-tauri/crates/rshell-protocol/src/ssh/client.rs:828`
- **严重度**：low
- **问题描述与证据**：client.rs:828-838 实读：`let len = data.len().min(buf.len()); buf[..len].copy_from_slice(&data[..len]);` 余量无缓冲即丢。当前不可达：SessionService 的 SSH 路径走 take_data_receiver（session/service.rs:427-429），`Box<dyn Connection>` 只装 Telnet/Serial（:594-631，SSH 分支 unreachable!()）；Telnet/Serial 实现都做了不丢字节处理，唯独 SSH 没有。未来接线即触发终端输出丢失。
- **修复方案**：在 SshClient 内保留未消费余量（类似 serial 的 pending_bytes），或删除这个未使用的 Connection impl 防止误用。
- **验收标准**：单测以大于 buf 的消息验证多次 recv 字节总和无损；或 impl 已删除。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-17：ComposeService::send_text 对每个目标会话的发送失败只记 debug 日志并继续，全部失败也返回 Ok，前端撰写窗格静默成功

- **位置**：`src-tauri/crates/rshell-core/src/script/compose.rs:55`
- **严重度**：low
- **问题描述与证据**：compose.rs:53-60 实读：`if let Err(e) = session_service.send_data(...) { debug!(...); }` 循环后无条件 Ok(())。SendComposeText 是可见操作入口（commands.rs:607-618、command_dispatcher.rs:382-387、lib.rs:194 注册），目标会话全部未连接时用户得不到任何错误反馈。
- **修复方案**：统计失败数：全部失败返回聚合错误；部分失败至少发布一个失败事件供 UI 提示。
- **验收标准**：单测——全部未连接时 invoke 返回错误；部分成功时有可见失败提示。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-18：传输任务表只增不删：终态任务永久留在 HashMap，长期运行进程内存无界增长

- **位置**：`src-tauri/crates/rshell-core/src/transfer/service.rs:154`
- **严重度**：low
- **问题描述与证据**：`tasks: Arc<RwLock<HashMap<Uuid, TransferTask>>>`（transfer/service.rs:154）；grep 全文件无 remove/retain；enqueue 插入（:218、:268）后只有状态改写，终态分支（:389-412）也不移除。listTransfers 随任务数线性膨胀。
- **修复方案**：终态任务保留有限条数（如最近 N 条）或提供前端「清除已完成」命令走删除。
- **验收标准**：单测完成 N+1 个任务后任务表长度 ≤ N。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-19：aes::encrypt 先用随机 nonce 加密一次并整体丢弃结果，再用第二个 nonce 重加密，每次调用白做一遍 AES-GCM 且注释自认混乱

- **位置**：`src-tauri/crates/rshell-infra/src/crypto/aes.rs:41`
- **严重度**：low
- **问题描述与证据**：aes.rs:41-72 实读：第一个 SealingKey（SimpleNonceSequence）seal 出的 in_out 从未使用；注释「重新生成 nonce 用于存储…实际做法：加密前记录 nonce」后用 FixedNonceSequence 对原明文重加密输出。输出格式（nonce+密文+tag）由第二次加密决定，解密路径正确，但首个加密纯属浪费且易被误改。
- **修复方案**：删除第一次 seal，直接生成随机 nonce 后走 FixedNonceSequence 一次加密。
- **验收标准**：加解密 roundtrip 单测不变；函数体内只剩一次 seal_in_place_append_tag。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-20：Rhai 脚本只有 max_operations 限额而无时间上限：rshell_sleep(10000) 每次真实睡眠 10s，可组合出近乎无限的阻塞线程占用，IPC 调用无取消路径

- **位置**：`src-tauri/crates/rshell-core/src/script/engine.rs:72`
- **严重度**：low
- **问题描述与证据**：engine.rs:52 只 set_max_operations(100_000)；rshell_sleep 直接 std::thread::sleep、单次 clamp 10s（:72-74 实读）；dispatcher.execute_script 在 spawn_blocking 中 await（command_dispatcher.rs:622-624 实读），期间该 invoke 挂起且无可取消路径。循环内每次 sleep 消耗少量 operations 预算即可长时间占位。
- **修复方案**：用 rhai on_progress 回调按挂钟时间中止脚本，或把 rshell_sleep 的累计睡眠计入 operations 等价成本。
- **验收标准**：单测执行含长睡眠循环的脚本在设定时限内返回超时错误。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-21：suspend_tunnel/resume_tunnel 只改状态字段并广播事件，listener 全程继续接受并转发，「挂起」是假状态（已注册命令的静默假操作）

- **位置**：`src-tauri/crates/rshell-core/src/security/tunnel_manager.rs:393`
- **严重度**：low
- **问题描述与证据**：suspend_tunnel（tunnel_manager.rs:393-411）/resume_tunnel（:414-430）仅改 state 并 publish；listener spawn 任务（:271-331）从不读 state。命令已注册（lib.rs:190-191、commands.rs:584-601）；grep 前端无 suspendTunnel/resumeTunnel 调用方（仅 client.ts:131-132 导出）。比 pause_transfer 少一层 UI 暴露，同类假语义。
- **修复方案**：挂起应 abort listener 并保留规则、恢复时重新 bind（端口被占需报错）；或本轮移除这两个命令避免假语义。
- **验收标准**：suspend 后新建连接被拒绝，resume 后恢复转发且端口冲突时报错。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-22：known_hosts 搜索路径含当前工作目录的相对路径 known_hosts：受影响目录里预置的文件可绕过用户决策直接信任主机密钥

- **位置**：`src-tauri/crates/rshell-protocol/src/ssh/client.rs:484`
- **严重度**：low
- **问题描述与证据**：connect_ssh 构建 known_hosts_paths 时无条件 `known_hosts_paths.push(PathBuf::from("known_hosts"))`（client.rs:484，注释「开发环境」但无任何开关）；verify_known_hosts（:133-156 实读）对任一路径命中即返回 known=true 免弹窗。应用从被攻击者影响的 cwd 启动（如经 CLI 指定目录）时，目录内一个 known_hosts 即可预置信任，削弱「主机密钥决定来自用户」红线。
- **修复方案**：移除 cwd 相对路径，仅保留 ~/.ssh/known_hosts 与受控的 data_local/rshell/known_hosts；开发需要时用显式环境变量开启。
- **验收标准**：grep 无无条件 cwd 路径；单测验证 cwd 预置文件不再命中。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-23：隧道 bind_address 无任何校验或回环限制，SOCKS5 强制 no-auth：绑到 0.0.0.0 会把无认证代理暴露给局域网

- **位置**：`src-tauri/crates/rshell-core/src/security/tunnel_manager.rs:252`
- **严重度**：low
- **问题描述与证据**：create_tunnel 直接 `TcpListener::bind(format!("{}:{}", bind_address, bind_port))`（tunnel_manager.rs:252-255），无白名单/警告；DynamicForward 的 socks5_handshake 强制 no-auth（:542-546 实读，注释「强制 no-auth (即便客户端没列， RFC 允许 server 选)」）。前端 TunnelPanel.vue:20 draftBind 默认 127.0.0.1:8080，:49 parseEndpoint 原样解析传入无校验。用户填 0.0.0.0 后同网段任何主机可免费使用该 SSH 连接作出口代理。
- **修复方案**：对非回环 bind_address 默认拒绝或要求显式二次确认，界面标明「将暴露给局域网」。
- **验收标准**：bind 0.0.0.0 时 UI 出现确认且后端有对应校验；回环地址行为不变。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-24：状态条在传输工作区恒显示 "SFTP"、编码恒为硬编码 "UTF-8"，与会话实际协议/编码无关，与组件「只显示可验证状态」的注释矛盾

- **位置**：`src/components/StatusBar.vue:31`
- **严重度**：low
- **问题描述与证据**：StatusBar.vue:5 注释「只显示当前会话可验证的状态」，但 :31 `if (props.workspace === "transfer") return "SFTP"`（即便无会话或当前会话是 Telnet/Serial 也显示 SFTP），:35 `const encodingLabel = "UTF-8"` 为常量（实读）。SessionList.vue:208「打开 SFTP」菜单也未按协议过滤（同菜单「更新凭据」项有 protocol==='SSH' 过滤，SFTP 没有）。
- **修复方案**：protocolLabel 基于当前会话（无会话或非 SSH 显示 "—"），编码从会话/终端配置读取或去掉该项；SessionList 的 SFTP 菜单项按 protocol==='SSH' 显隐。
- **验收标准**：Telnet 会话的传输工作区不显示 SFTP 字样；菜单对非 SSH 会话隐藏。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-25：隧道状态列对字符串状态用 Object.keys 取 [0]，已建隧道的状态恒显示 "0" 而非 Active/Suspended

- **位置**：`src/components/TunnelPanel.vue:116`
- **严重度**：low
- **问题描述与证据**：TunnelPanel.vue:116 `String(Object.keys(row.state || {})[0] || "—")`：`Object.keys("Active")` 返回索引数组 ["0","1",…]，取 [0] 得 "0"（truthy，不落 "—"）。后端 TunnelState 无 serde rename（`rshell-api/src/types.rs:199-203` 实读），无负载变体序列化为字符串；types.ts:135 镜像 `"Active" | "Suspended" | { Error: string }`——状态列恒显示 "0"，仅 { Error } 对象态显示 "Error"。
- **修复方案**：改为 `typeof row.state === 'string' ? row.state : Object.keys(row.state)[0]`。
- **验收标准**：Active/Suspended 隧道的状态列显示正确文案。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-26：远程文件列表把 entry.file_type 塞进 mode 当「属性」列展示，列头写的权限属性实际显示 File/Directory，后端真正的 FilePermissions 被丢弃

- **位置**：`src/components/transfer/FileBrowserPane.vue:94`
- **严重度**：low
- **问题描述与证据**：FileBrowserPane.vue:94 实读映射 `mode: entry.file_type`；:285-286 「属性」列渲染 row.mode。后端 RemoteFileEntry（`rshell-api/src/types.rs:100-108`）提供的 permissions: FilePermissions 与 group 前端完全未消费（types.ts 有镜像）。展示语义错位。
- **修复方案**：由 entry.permissions 拼八进制/rwx 字符串展示（如 0644），或把列名改为「类型」以免错位；可选补 group。
- **验收标准**：属性列内容与真实权限一致或列名如实为「类型」。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-27：关键单测用桩替换了与真实库/webview 相反或不存在的行为，形成系统性假绿，掩盖 PROB-01/02/03

- **位置**：`tests/unit/AppLayout.spec.ts:58`
- **严重度**：low
- **问题描述与证据**：AppLayout.spec.ts:58 DockviewVue 桩 `<div data-testid='dockview'><slot name='terminal' /></div>` 渲染了真实 dockview-vue 7.0.4 从不渲染的插槽（dist 实测 "slots" 0 次）；AppLayout.spec.ts:35-39 整体 mock src/ipc/client，invoke 参数键名在测试中永不被校验；KeyManagerPanel.spec.ts:17/:29 桩化生产 webview 中不存在的 window.prompt/window.confirm。本次实跑 `npm test`（exact command: npm test）：Test Files 20 passed (20), Tests 80 passed (80)——全绿与上述高危问题并存，证明该套件当前不能作为验收门槛。
- **修复方案**：桩模拟真实行为：DockviewVue 桩不渲染插槽、改为验证 addPanel 调用；window.confirm/prompt 用法改为对 plugin-dialog/ElMessageBox 的 mock；补一条校验 invoke 参数键与命令签名一致的契约测试。
- **验收标准**：契约测试能检出键名不一致；无 addPanel 时 dockview 相关断言失败。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-28：生产构建 app.security.csp 为 null，WebView 完全没有内容安全策略兜底

- **位置**：`src-tauri/tauri.conf.json:26`
- **严重度**：low
- **问题描述与证据**：tauri.conf.json:25-28 实读 `"security": { "csp": null }`。该应用在 WebView 内渲染远端终端输出与 SFTP 文件名（xterm.js/FileBrowserPane），无 CSP 时任何注入回归可自由外联；应用仅加载本地资源（index.html 只引用 /src/main.ts），完全可配置最小 CSP。与 fs 过度授权（PROB-11 修复前）叠加会放大影响面。
- **修复方案**：设置最小 CSP（`default-src 'self'; connect-src ipc: http://ipc.localhost` 等），xterm/element-plus 需要时逐项加白。
- **验收标准**：打包配置带 CSP，dev 下终端与文件面板渲染正常且无 CSP 违规告警。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-29：声明但从未使用的依赖：@tauri-apps/plugin-clipboard-manager、@tauri-apps/plugin-shell（后者 Rust 侧还注册了插件却零权限零调用）

- **位置**：`package.json:26`
- **严重度**：low
- **问题描述与证据**：grep src/ 仅命中 plugin-dialog 与 plugin-fs 的 import；clipboard 只在 client.ts:90-91 注释出现（实际用 navigator.clipboard）。lib.rs:58 注册 `tauri_plugin_shell::init()`、Cargo.toml:61 声明 `tauri-plugin-shell = "2"`，但 capabilities/default.json 无任何 shell:* 权限且前端无调用——插件注册后完全不可达，属死面。
- **修复方案**：从 package.json 移除两个未用依赖；shell 插件若无近期用途，连同 Cargo.toml 依赖与 lib.rs:58 注册一并移除。
- **验收标准**：构建通过且 `npm ls` 无残留引用；capabilities/Cargo/代码三者一致。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

### PROB-30：scripts/build.ps1 / build.cmd 按旧布局在仓库根执行 cargo build（根目录已无 Cargo.toml），按现状必然失败；-Target 参数从未传给 cargo

- **位置**：`scripts/build.ps1:54`
- **严重度**：low
- **问题描述与证据**：实测 `ls /Users/letmlook/code/rshell/Cargo.toml` → No such file or directory；src-tauri/Cargo.toml:3 注明「仓库根不再有 Cargo.toml;Rust workspace 在 src-tauri/ 内」。build.ps1:24 `Set-Location $RepoRoot` 后 :54 `& cargo build --release --locked`，build.cmd:16 pushd 仓库根后 :48 同样——两脚本必然失败，随后 :59/:56 在根 target\release 找 rshell.exe 也落空。build.ps1:15 的 -Target 只在 :53 Write-Host 使用，从未作为 --target 传给 cargo。缓解：scripts/README.md:17 已标注「本轮未更新或验证，不作为当前构建入口」，但脚本头 Usage 注释仍给出失效用法。
- **修复方案**：删除两个 Windows 脚本，或改为 cd src-tauri 后构建并真正传递 --target、修正输出路径；至少修正脚本头 Usage 注释与 scripts/README 措辞一致。
- **验收标准**：脚本在干净环境可产出二进制，或已删除且无文档引用矛盾。
- **状态**：已修复（验证方式：fmt/clippy/typecheck 快速检查 + 最终全量验证）

## 三、暂缓项

以下各项**已证实成立，超出本轮修复容量**，暂不立条，留待后续轮次处理：

1. **rshell-protocol Cargo.toml 中 uuid 重复声明（[dependencies] 与 [dev-dependencies] 各一次）**（severity: low）——已证实（实读 `src-tauri/crates/rshell-protocol/Cargo.toml:26-31`）：`uuid = { workspace = true }` 同时出现在 [dependencies] 末尾与 [dev-dependencies]；dev 重复项无任何作用（uuid 在 `src/ssh/client.rs` 正常使用）。影响仅为声明冗余，不改变构建产物。
2. **建议：密钥体系整体半成品（生成的私钥无任何认证消费方）**（severity: low）——核实过程中发现 `get_private_key_data` 在 crates 内无任何调用方，即 KeyManager 生成的密钥当前不参与任何 SSH 认证（问题本体已并入 PROB-09/PROB-10 的证据）。若本轮容量允许，建议在修复 PROB-09/10 时一并明确密钥的消费路径或明确 docs/08 中密钥管理的能力边界。暂缓原因：超出本轮 30 条修复容量，且其修复方向依赖产品决策（接通消费方 vs 明示不支持）。

## 四、未证实发现

以下为体检过程中产生、经复核后**未作为独立问题立条**的原始发现及处置记录：多数与已立条问题同根因或同修复面，已并入对应条目（合并不改变对应条目的修复范围）；其中第 9 条携带一处**复核未能证实**的子断言，已更正表述并留待人工确认。

1. cancel_transfer 不发取消信号、传输跑完后被改写回 Completed/Failed（transfer/service.rs:453）——与 PROB-04 同根因合并：pause/cancel 缺失的是同一个取消信号通道，终态覆盖点是同一处 :389-412，修复与验收合并为一条。
2. KeyManagerPanel.vue:88/58 window.confirm/prompt 在 wry 中不可用——与 PROB-03 同根因合并（wry-0.55.1 WKWebView UIDelegate 未实现 JS 对话框面板，证据链完全相同），修复同一条。
3. docs/08:23 与 docs/09:26 声明 pause/resume「无界面入口」与 HEAD 矛盾——与 PROB-04 同根因合并：7258309 给 no-op 按钮接线且未更新文档，文档改写已并入 PROB-04 的修复与验收。
4. trust_host_key IPC 命令绕过决策链、前端无调用方的死暴露面（commands.rs:424）——与 PROB-06 同一命令、同一修复面（按 decision 分派或删除命令）合并为一条。
5. capabilities 权限过宽：fs 写/删/改名与 dialog:allow-save 无前端调用路径（构建与文档审查）——与 fs 权限错配（缺 allow-stat）同一权限集、同一修复（收敛 default.json），合并进 PROB-11。
6. Tauri capabilities 最小权限审查：fs 过度授予而 stat 未授权（default.json:23）——与构建审查的 fs 权限两条重复（同一文件同一结论），合并进 PROB-11。
7. tauri.conf.json csp 为 null（Tauri 配置最小权限审查）——与构建与文档审查的 CSP 条目完全重复，合并进 PROB-28。
8. KeyManager 启动不回读 keys_dir、重启后孤儿 .key 无法删除（密钥与凭据存储审查）——与正确性审查的同位置条目重复（同一 key_manager.rs:43 根因），合并进 PROB-09。
9. SSH 私钥明文落盘 + passphrase 被忽略 + 加密私钥导入失败（密钥与凭据存储审查）——与安全审查（0644）和契约审查（passphrase）同根因，合并进 PROB-10；其中「带口令私钥在 from_openssh 处导入直接失败」子断言**复核未能证实**（ssh-key 0.6 可解析加密 PEM，实际缺陷是无后续解密路径），已在 PROB-10 证据中更正表述，**留待人工确认**（修复 PROB-10 时一并验证加密私钥导入的实际行为）。
10. execute_async 悬垂引用 + String 折叠为 I32(0)（WASM 插件沙箱审查）——与 unsafe 审查的同方法条目重复（同一 sandbox.rs:307-310），合并进 PROB-15。
11. MasterPasswordDialog 文案虚假声明加密私钥（主密码安全审查）——与前端 MasterPasswordRequired 死链路同根因（主密码功能半接线：不可达对话框 + 与实现矛盾的文案），合并进 PROB-14。
12. generate_ssh_key/import_private_key 的 passphrase 被静默忽略（安全·命令参数语义）——与私钥明文落盘同模块同根因（加密存储缺失导致恒为未加密密钥），合并进 PROB-10，验收标准合并。

## 五、最终验证记录

> **验证日期**：2026-09-30
> **验证命令**：`bash scripts/verify.sh --skip-install`
> **结果**：**全部通过**，脚本退出码 0。

说明：本轮验证经 `npm run verify -- --skip-install` 触发（package.json:15 定义的 verify 脚本即 `bash scripts/verify.sh`，npm 执行日志确认实际运行的命令为 `bash scripts/verify.sh --skip-install`，与上述验证命令一致）。脚本以 `set -euo pipefail` 逐级 fail-fast，任一步骤失败即中止，本次全程无失败步骤。

各步骤结果：

| # | 检查项 | 实际命令 | 结果 |
| --- | --- | --- | --- |
| 1 | 前端类型检查 | `npm run typecheck`（vue-tsc --noEmit） | 通过，无错误 |
| 2 | 前端单测 | `npm test`（vitest run） | Test Files **21 passed (21)**、Tests **87 passed (87)** |
| 3 | 前端生产构建 | `npm run build`（vue-tsc --noEmit && vite build） | 构建成功（1061 modules transformed，built in 2.21s） |
| 4 | 文档契约检查 | `npm run check:docs` | 通过（Documentation contract passed, 14 current documents; dated historical records excluded） |
| 5 | 构建产物检查 | `npm run check:bundle` | 通过（Build output OK: 10 js chunks，均低于 500000 字节上限） |
| 6 | 维护脚本自检 | `npm run test:scripts`（node --test scripts/*.test.mjs） | **37 通过 / 0 失败** |
| 7 | Rust 格式检查 | `cargo fmt --all --check` | 通过，无 diff |
| 8 | Rust 静态检查 | `cargo clippy --workspace --all-targets -- -D warnings` | 通过，`-D warnings` 下无告警 |
| 9 | Rust 全量测试 | `cargo test --workspace` | **231 通过 / 0 失败**（rshell_lib 7、rshell_api 6、rshell_core 153、集成测试 private_key_security 4、rshell_infra 18、rshell_plugin_sdk 6、rshell_protocol 37；另有 1 个 doc-test 为 ignored，0 失败） |

与体检基线对比：前端单测由基线的 20 文件/80 用例增至 21 文件/87 用例（新增 IPC 契约对账等回归测试），全部通过；cargo test 覆盖 workspace 全部 crate 及 1 个集成测试目标，全部通过。上述结果对应第二节的 30 条问题修复：全部条目由 fmt/clippy/typecheck 快速检查覆盖后，在本节全量验证中统一收口。
