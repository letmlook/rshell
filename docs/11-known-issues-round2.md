# RShell 第二轮问题清单

> **体检日期**：2026-10-02
> **基线 commit**：`82304ca`（完整哈希 `82304ca9d02f51d82c79efb579f899f3c73507bb`，fix: resolve 30 audit findings across IPC, UI, and backend）
> **体检方式**：五个方向并行排查 + 逐条独立复核，详见第一节。
> **基线验证状态**：通过（`bash scripts/verify.sh --skip-install` 退出码 0）

## 一、概述

**问题总数与分布**：本轮共立条 **20** 个问题——**high 0 条**、**medium 5 条**（R2-01～R2-05）、**low 15 条**（R2-06～R2-20）。按本轮严重度校准原则（high 仅留给数据丢失、崩溃或核心功能不可达/结果错误；medium 留给明确缺陷或假状态；low 留给冗余、文档漂移等），无达到 high 门槛的条目。修复过程中另发现并新立 1 条（R2-21，low，mark_failed 终态守卫缺失），合计 **21** 条。

**复核结果**：20 条全部经逐条独立复核成立——**confirmed 17 条、corrected 3 条**（R2-01、R2-10、R2-11，按复核意见修正表述后收录，修正点在各条证据中注明）；**复核推翻 0 条**，第三节「未证实发现」为空。复核员在各条上重新实读源码并复跑了关键命令（cargo 单点测试、/tmp 临时项目实验、node 差集脚本、git 取证），结论与修正意见一并列于各条「证据」。

**原始发现处理**：五个方向共返回 **29 条原始发现**，均有行级代码/命令输出佐证，**无一判定为不可信、丢弃 0 条**；其中 9 条为同根因重复发现，已并入对应条目（uuid 重复声明 4 条 → R2-09，get_private_key_data 零消费 4 条 → R2-08，RSA 位数问题 1 条 → R2-05），29 − 9 = 20 条。合并前对 R2-05/R2-08/R2-09/R2-04 做了整理员本机复核（`grep -n "uuid = " src-tauri/crates/rshell-protocol/Cargo.toml` 输出 25/30 两行；`grep -rn "get_private_key_data" src-tauri --include="*.rs"` 仅命中定义处；Read key_manager.rs:250-267 确认两个 RSA 分支同一无位数调用；grep src/ 隧道事件仅命中 types.ts:460/:495 类型镜像），结论一致。

**一句话总结**：第二轮共整理出 20 个问题，无 high：5 个 medium（传输取消/暂停竞态致终态失真、后端错误全部显示为 [object Object]、关闭终端标签后无法重建、隧道状态事件前端零订阅致假 Active 状态、RSA2048 恒生成 4096 位且标签翻转），15 个 low（IPv6 信任条目永不命中、终态可被改写、密钥体系半成品与 uuid 冗余等两条上轮暂缓项、契约镜像漂移及测试盲区、若干 UI/健壮性/文档项）；9 条重复发现已合并，无发现被丢弃。

**修复结果**：**R2-01～R2-21 共 21 条全部修复**（R2-21 为修复过程中新增，见二、问题清单末尾），无遗留、无回退；每条均按 fixPlan 修复并经 fmt/clippy/typecheck 快速检查，附针对性单测（多项经变异验证确认能捕获回归）或记录在案的验证。全量验证已于 2026-10-03 由 `bash scripts/verify.sh --skip-install` 统一收口通过（退出码 0），明细见第五节。

**体检方式**：五个方向并行排查 + 逐条独立复核。

- **并行排查**：五个方向各自返回带 where/evidence/fixHint 的原始发现（覆盖传输与后端运行时、密钥与安全组件、IPC 契约与事件、前端 UI 生命周期、依赖构建与文档记录等面向）；
- **逐条独立复核**：每条由独立复核员在 HEAD `82304ca` 上重新实读相关源码并复跑关键检查（单点 cargo test、/tmp 临时项目行为实验、node 脚本变体差集、git show/blame 取证），出具 confirmed / corrected / refuted 结论；corrected 条目按复核意见修正后收录，refuted 条目不立条并记录于第三节。

**基线检查与核实方式**（于基线 commit `82304ca` 上进行）：

- 基线全量验证通过：`bash scripts/verify.sh --skip-install` 退出码 0；
- 依赖版本核对：Cargo.lock 锁定 tauri 2.11.5、ssh-key 0.6.7（本机 cargo registry 源码实读）；node_modules 实测 dockview-core 7.0.4；
- 实验性验证：/tmp 临时项目分别复刻 `pattern_matches`/`parse_host_port`（IPv6 匹配）、SOCKS5 握手（分段读）、ssh-key RSA 生成调用（位数实测），均未修改仓库文件、已清理；
- git 取证：`git show 82304ca --stat`、`git blame`、与基线 `7258309` 对照，确认各条均非第一轮 30 条已修复问题（PROB-01～PROB-30）的重复记账；
- 复核环境说明：部分复核机上 rustup 自动更新损坏（rust-analyzer 组件冲突），以 `RUSTUP_TOOLCHAIN=stable-aarch64-apple-darwin` 或 `RUSTUP_AUTO_INSTALL=0` 绕行完成检查，不影响结论。

## 二、问题清单

### R2-01：传输 cancel/pause 存在竞态窗口：控制通道建立前的取消/暂停丢失，传输照跑完且终态失真

- **位置**：`src-tauri/crates/rshell-core/src/transfer/service.rs:355`
- **严重度**：medium
- **问题描述**：execute_transfer 在 :315-328 用 tasks 写锁置 Transferring（:318-321 只覆盖「启动前已取消」），经 :345 `ssh_client_provider(task.session_id).await` await 点后，:355-359 才创建初值 TransferControl::Run 的 watch 通道并 insert control_channels，:365 才 spawn 传输任务。此窗口内 cancel_transfer（:576 先读通道）与 pause_transfer（:529 先读通道）读 control_channels 得 None，send 被跳过（:583-585/:536-538），仅把任务表改为 Cancelled/Paused；spawn 后传输循环 wait_for_run（rshell-protocol/src/ssh/sftp.rs:365-377）首轮 borrow_and_update 读到 Run，copy_with_progress（sftp.rs:332 每分块前检查）全速执行至完成。cancel 场景：文件实际已完整落盘，finalize_transfer（:466-468）把任务定格「已取消」；pause 场景：finalize（:470-474）覆盖回 Completed 并广播 TransferCompleted，暂停意图完全失效。属明确假状态缺陷。窗口现实可达：enqueue_upload :243 / enqueue_download :294 在进入窗口前即广播 TransferQueueChanged，前端可渲染任务并发起 cancel/pause。
- **证据**：逐行实读确认（service.rs:315-365 时序、cancel_transfer :575-595 与 pause_transfer :527-547 均先读通道后锁任务表、sftp.rs:365-377、finalize :466-474）。复核实跑 `RUSTUP_AUTO_INSTALL=0 ~/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p rshell-core transfer::service::tests`（cwd=src-tauri）→ 24 passed; 0 failed; 129 filtered out；grep 证实 tests 模块（service.rs:706-1308）无任何 set_ssh_client_provider 注入（唯一注入点在生产 command_dispatcher.rs:176），窗口路径零测试覆盖。与 docs/10-known-issues.md PROB-04（:59-63）不同：PROB-04 记录基线 7258309 上「完全无控制通道、终态无条件覆盖」，HEAD 82304ca 已修复，本条为修复后残留竞态，非重复报告。（corrected：复核修正 fixPlan 方案二，见下。）
- **修复方案**：方案一（完备）：spawn 任务体在打开 SFTP 后、首轮 wait_for_run 前复查任务表，Cancelled 立即中止、Paused 挂起等待恢复。方案二：把控制通道创建移进 :315-328 同一 tasks 写锁临界区，且必须把 cancel_transfer/pause_transfer 的 control_channels 读取（现 :576/:529 位于锁任务表之前）一并移进 tasks 写锁临界区内，使传输启动决策与取消/暂停决策在同一把锁上互斥——仅移动 insert 不够，仍存在「cancel 先读到 None 后挂起 → execute_transfer 完成临界区+insert+spawn → cancel 再拿锁置状态」的交错丢信号。
- **验收标准**：新增单测注入可控 provider（经 set_ssh_client_provider）在 await 期间调用 cancel_transfer/pause_transfer，断言传输循环不启动或立即中止、远端/本地不产生完整文件、终态与实际一致；`cargo test -p rshell-core transfer::service::tests` 全绿。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-02：IpcError 整体序列化为对象、前端 catch 一律 String(e)，全部后端错误显示为 [object Object]

- **位置**：`src/stores/sessions.ts:48`
- **严重度**：medium
- **问题描述**：Tauri 2 对 Serialize 的 IpcError（error.rs:45-50，含 kind/message/session_id）走 InvokeError(serde_json::Value) 整体序列化（tauri 2.11.5 src/ipc/mod.rs:224/:240-245，ipc/protocol.rs:121-124，scripts/ipc-protocol.js:43-49 response.json()），前端 catch 到的是对象而非字符串；而消费面约 30 处 catch 全部用 String(e) 展示（sessions.ts:48/66/95/107、hostKey.ts:58、theme.ts:43/52/58、App.vue:212/317、KeyManagerPanel/TriggerEditor/QuickCommandPanel/TunnelPanel/PluginPanel/SessionCreateDialog/SessionCredentialDialog.vue、FileBrowserPane.vue:144、TerminalPane.vue:157），用户看到的所有后端错误文案都是 "[object Object]"。error.rs 头注释设计的 kind 分支/message 展示/session_id 挂会话行三个用途全部落空，前端无任何位置读取 .kind/.message。另 IpcErrorKind::Io/Permission 全仓库无构造点，声称的固定 kind 集合本身也未产满。
- **证据**：复核逐环实读验证全部成立：error.rs:45-50（#[derive(Serialize)] IpcError{kind,message,session_id}）、commands.rs:62 宏模板 `Result<$ret, IpcError>`（全文件 49 处签名）、tauri-2.11.5（Cargo.lock 锁定，registry 实读）ipc/mod.rs:224 `InvokeError(pub serde_json::Value)` 与 :239-245 blanket `From<T: Serialize>` 整体 serde_json::to_value、ipc/protocol.rs:118-124 Err→to_vec(application/json)、scripts/ipc-protocol.js:43-49 `response.json()`、scripts/core.js:88-91 `reject(e)`。前端 grep 实测 33 处 `String(e)`（报告所列位置全部命中；TerminalPane.vue:157 为 `e instanceof Error ? e.message : String(e)`，对象非 Error 实例仍落 String(e)）；node 实测 `String({kind:'not_found',...})` 输出「连接失败：[object Object]」。grep src/ 无任何位置读取 IPC 错误的 .kind/.message/.session_id；IpcErrorKind::Io/Permission 仅 error.rs:134-135 测试断言，无生产构造点。排除项成立：tauri-plugin-fs 2.5.1 error.rs:36-41 impl Serialize 用 serialize_str（原报告标 :52-57，行号偏差内容正确），FileBrowserPane.vue:91-94 isEntryGone 字符串匹配不受影响。实跑 `RUSTUP_TOOLCHAIN=stable-aarch64-apple-darwin cargo test -p rshell --lib error::tests::serializes` → 1 passed（仓库自带测试断言 IpcError 序列化为 {kind,message,session_id} JSON）。非 PROB-13 重复（其修复后的 attachError 状态条内容正是本条的 String(e) 文案）。
- **修复方案**：在 src/ipc/client.ts 的 call() 统一捕获 rejection，识别 {kind,message,session_id} 形状后转成可读 Error（message 作文案、kind 附加暴露、session_id 透传供挂会话行）再抛出；或约定薄壳 Err 路径直接 reject message 字符串。
- **验收标准**：人为触发任一后端错误（如连接不存在的主机）时界面显示 message 文案而非 [object Object]；client.ts 增加错误形状转换单测通过。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-03：关闭 dockview 终端标签后重新点击同一会话不重建面板：单会话时终端区域永久空白

- **位置**：`src/App.vue:180`
- **严重度**：medium
- **问题描述**：ensureTerminalPanel（:154-167）仅由 onDockviewReady(:171) 与 watch(activeTerminal)(:180-182) 触发；selectSession(:205-214) 是 `activeTerminal.value = id`，同一 id 重复赋值不触发 Vue watch（Object.is 相等），面板不会重建。关闭路径真实可达：App.vue:401-407 的 DockviewVue 未传 defaultTabComponent，dockviewPanelModel.js:74/:79 回落 DefaultTab，其自带关闭按钮（defaultTab.js:14-16 创建、:26-34 click→:33 params.api.close()），关掉面板后再次点击同一会话不会重建；只有一个会话时终端区域从此永久空白，直到重启应用。App.vue 无 onDidRemovePanel/closePanel/removePanel 任何面板关闭处理。
- **证据**：复核确认：grep 全 src/ 确认 ensureTerminalPanel 仅 App.vue:171/:181 两处调用（定义 :154-167）；selectSession :206 同 id 重复赋值不触发 watch；grep removePanel/closePanel/onDidRemove/onWillClose 等于 App.vue/TerminalPane.vue 零匹配，面板关闭不回写 activeTerminal，容器 v-if（App.vue:402）不重建、onDockviewReady 不再触发，单会话下确无恢复途径。实跑 `npx vitest run tests/unit/AppLayout.spec.ts` → Test Files 1 passed (1), Tests 10 passed (10)；其中 're-activates the existing panel'（:146-163）只走 a→b→a 值变化路径，无「关闭标签后重击同一会话」覆盖。未做：tauri dev GUI 实操验证（静态证据链已闭合）。非已修复问题重复（PROB-01～30 无涉及面板关闭后不重建的条目）。
- **修复方案**：selectSession 中直接调用 ensureTerminalPanel（不依赖 watch 的值变化）；或订阅 dockview 面板关闭事件，把 activeTerminal/面板存在性同步回内存。
- **验收标准**：新增用例：单会话下关闭终端标签后重新点击该会话，终端面板重建且可正常输入；AppLayout.spec.ts 覆盖该场景并通过。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-04：TunnelStateChanged/ActiveTunnelsChanged 前端零订阅：会话断开后 TunnelPanel 持续显示 Active（PROB-08 修复只覆盖后端半截）

- **位置**：`src/components/TunnelPanel.vue:111`
- **严重度**：medium
- **问题描述**：后端 deactivate_session_tunnels 已把该会话隧道置 TunnelState::Error、abort listener 并逐条 publish TunnelStateChanged + ActiveTunnelsChanged；但前端 6 处 subscribeAppEvents 调用点（App.vue:294、hostKey.ts:37、theme.ts:71、sessions.ts:118、QuickCommandPanel.vue:87、TriggerEditor.vue:84）处理的事件集合均不含这两个事件，TunnelPanel 的 refresh 只由 onMounted(:111) 与本面板增删操作(:95-108) 触发。面板打开期间会话断开，隧道行状态列持续显示旧的 Active 文案，直至用户手点刷新。
- **证据**：复核在 HEAD 82304ca 端到端核实：deactivate_session_tunnels 实际位于 tunnel_manager.rs:662-699（置 TunnelState::Error(SESSION_DISCONNECTED_REASON)、abort listener :684-686、逐条 publish TunnelStateChanged :694 + ActiveTunnelsChanged :699，原报告引 658-697 有 2-4 行偏移），经 new() 中 EventBus 订阅（:202）在 ConnectionStateChanged{Disconnected} 时触发；src-tauri/src/events.rs:21-32 subscribe_bridge 对所有事件无条件 emit("rshell://event")，事件可达前端。grep src/ 中两事件仅命中 src/ipc/types.ts:460/:495 类型镜像，无任何处理代码（整理员复核一致）。6 处订阅调用点行号逐一核实且事件集合均不含两事件。TunnelPanel.vue:111 onMounted(refresh)，refresh 另由 add(:96)/remove(:105)/手动按钮(:118) 触发；无轮询、无 tunnel store，listTunnels 全前端仅 TunnelPanel.vue:27 调用；后端 list_tunnels（tunnel_manager.rs:556-568）实时读 state，刷新即见 Error。非 PROB-08 重复：其验收标准为纯后端，82304ca 对 TunnelPanel 的改动仅 PROB-23/PROB-25。限定：TunnelPanel 经 SidePanel.vue:153-156 条件挂载，仅在侧栏 tunnels 子视图打开时存在，陈旧 Active 出现在「断开发生时面板正打开」场景（如网络被动掉线）；恢复路径除手点刷新外还有切换子视图重挂载一种。
- **修复方案**：TunnelPanel（或 App.vue 事件订阅处）补订阅 TunnelStateChanged/ActiveTunnelsChanged 触发 refresh；或最小改动：sessions store 的 ConnectionStateChanged=Disconnected 分支顺带刷新隧道列表。
- **验收标准**：打开 TunnelPanel 后断开所在会话，隧道行状态自动刷新为 Error 文案（组件测试或记录在案的手动验证）；订阅站点事件集合 grep 可查到两事件。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-05：generate_key 的 RSA2048/RSA4096 两分支走同一无位数调用：恒生成 4096 位，标签重启后自相矛盾

- **位置**：`src-tauri/crates/rshell-core/src/security/key_manager.rs:256`
- **严重度**：medium
- **问题描述**：SshKeyType::RSA2048 与 RSA4096 两个 match 分支均为 ssh_key::PrivateKey::random(&mut OsRng, Algorithm::Rsa { hash: Some(Sha256) })，无私钥位数入参。Cargo.lock 锁定 ssh-key 0.6.7，其 src/private.rs:171 DEFAULT_RSA_KEY_SIZE=4096、:498-499 Rsa 分支固定用该常量，故实际恒生成 4096 位密钥：UI 选「RSA 2048」得到 RSA-4096 级密钥且元数据标为 RSA2048（key_manager.rs:330/:349 落库与返回用请求的 key_type）；重启后 rebuild_stored_key（:193）→ ssh_key_type_of（:22-43，按模数位 >=4096 判 RSA4096）重新归类，同一密钥前后标签翻转。属假选择+假状态缺陷。
- **证据**：复核亲读 key_manager.rs:256-262/:22-43/:330/:349/:193；Cargo.lock ssh-key 0.6.7；registry ssh-key-0.6.7/src/private.rs:171 `const DEFAULT_RSA_KEY_SIZE: usize = 4096;`、:498-499（hash 参数被模式忽略）；并在 /tmp 临时项目（未修改仓库文件）实跑该调用，实测输出 `generated private modulus bits = 4104`（4096 级，绝非 2048）。KeyManagerPanel.vue:45-46 下拉确实提供 "RSA 2048"/"RSA 4096" 两项。实跑 `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo test -p rshell-core security::key_manager`（cwd=src-tauri）→ 9 passed; 0 failed，9 个测试全为 ED25519/passphrase/权限用例，无 RSA 生成位数覆盖。非 PROB-12 重复：PROB-12 恰是把下拉拆成 RSA2048/RSA4096 的修复，docs/10 的 30 条无一条涉及生成位数。
- **修复方案**：按 SshKeyType 分别用 RsaKeypair::random(rng, 2048/4096) 组装 KeypairData；或本轮先从 SshKeyType 与 UI 移除 RSA2048 选项，避免假选择。
- **验收标准**：新增单测：生成 RSA2048 后私钥模数为 2048 位、key_type 保持 RSA2048，重启 rebuild 后标签不变；`cargo test -p rshell-core security::key_manager` 全绿。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-06：known_hosts 主机模式匹配对裸 IPv6 地址永不命中：对 IPv6 主机的永久信任失效

- **位置**：`src-tauri/crates/rshell-protocol/src/ssh/client.rs:264`
- **严重度**：low
- **问题描述**：pattern_matches（:252-286）对 pattern 用 rfind(':') 切 host:port：对裸 IPv6（如 "::1"）切出 host_part=":"、port_part="1"，永不匹配；写侧 host_key_manager.rs:126-130 对 port==22 落盘裸 host（"::1"），parse_host_port（:216-229）同样用 rfind(':')。结果：应用自己保存的 IPv6:22 信任条目下次连接永远匹配不上，每次重弹主机密钥决策框；OpenSSH 对 port 22 的 IPv6 主机即写裸 "::1"，故 ~/.ssh/known_hosts 既有条目也不被信任继承。fail-closed、无信任绕过，但信任持久化对整类主机失效并训练用户盲点确认。
- **证据**：复核实读：client.rs:252-286（:258 已有 find("]:") 方括号分支，仅覆盖非 22 端口写法；:264 裸 host 回退 rfind(':') 仍在），为 scan_known_hosts 唯一 host 匹配路径（client.rs:220-222 调用）；host_key_manager.rs:126-130 与 :216-229（:224 rfind(':')）。因原函数为私有，复核将两函数逐字复刻到 /tmp/ipv6-check 临时 cargo 项目实跑（未修改仓库文件，已清理）：pattern_matches("::1", host="::1", port=22)=false、("fe80::1", "fe80::1", 22)=false，对照组 ("[::1]:2222", host="::1", port=2222)=true；parse_host_port("::1", 22)=(":", 1)（"1" 是合法 u16 走 Ok 分支，不回落 default_port=22）、("fe80::1", 22)=("fe80:", 1)。信任链闭环：command_dispatcher.rs:460-462 TrustPermanent → trust_host_key 写裸 "::1" → 重连 check_server_key（client.rs:302-318）→ verify_known_hosts 不命中 → client.rs:320 起 UI 决策弹框。OpenSSH man sshd(8) 格式规范：方括号仅用于非标准端口，端口 22 对 IPv6 即裸写，故 ~/.ssh/known_hosts 既有条目同样不被继承。grep docs/10 无 IPv6 条目；client.rs 现有 known_hosts 测试（:1402/:1431/:1448）仅方括号形式。
- **修复方案**：host 模式解析先用 find("]:") 识别方括号形式，再对剩余部分用 std::net::IpAddr::parse 判断是否 IPv6 字面量，而非盲目 rfind(':')；parse_host_port 同步修正。
- **验收标准**：新增单测：pattern "::1" 与 host "::1"（端口 22）匹配命中；保存过的 IPv6 主机密钥在重连时不再弹确认框。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-07：cancel_transfer 只排除 Completed：已失败终态任务可被改写为 Cancelled，终态互相改写

- **位置**：`src-tauri/crates/rshell-core/src/transfer/service.rs:580`
- **严重度**：low
- **问题描述**：:580-588 仅 `state != Completed` 即改 Cancelled 并广播 TransferQueueChanged：已广播 TransferFailed、error_message 非空的任务被再改为 Cancelled，错误信息与新状态并存；Cancelled 任务重复取消还会重置 finished_at（:582）。状态机允许终态→终态转换，与 is_terminal 的「不会再发生状态变化」注释（:53-59）矛盾。
- **证据**：复核实读：:53-59 is_terminal() 含 Completed/Failed/Cancelled；:580 守卫仅排除 Completed，Failed/Cancelled 均进入分支被改写（:581）并广播（:587），全程不清 error_message（失败路径 :486-496 设置 error_message=Some(e) 并广播 TransferFailed）。可达性：commands.rs:290 暴露 IPC、client.ts:112 导出 cancelTransfer，但 App.vue:449-450 仅接线 pause/resume、无 UI 取消按钮，需显式 IPC 调用触发——severity low 恰当。git blame 证实 :580 守卫来自旧提交 2b19b523（2026-07-31），82304ca 只加 finished_at 与 control.send，未动守卫，属上轮修复残留而非重复。实跑 `RUSTUP_AUTO_INSTALL=0 cargo test --lib transfer::service::tests::test_cancel`（cwd=src-tauri/crates/rshell-core）→ 2 passed; 0 failed; 151 filtered out，现有用例仅覆盖 Transferring→Cancelled 与 Completed noop，无 Failed 用例，验收所需测试缺失属实。
- **修复方案**：改为仅允许非终态（Pending/Transferring/Paused）取消：`if !task.state.is_terminal()`。
- **验收标准**：单测：对 Failed 任务调用 cancel_transfer 后状态仍为 Failed；对 Pending/Transferring/Paused 的取消行为不变。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-08：KeyManager 私钥无任何认证消费方，密钥体系仍是半成品（第一轮暂缓项 2）

- **位置**：`src-tauri/crates/rshell-core/src/security/key_manager.rs:543`
- **严重度**：low
- **问题描述**：get_private_key_data 全仓库仅定义处命中；会话公钥认证走 AuthMethod::PublicKey 的外部文件路径 key_path（rshell-api/src/types.rs:41-45，session/service.rs:70-77 组装 ResolvedAuthMethod，client.rs:593 russh_keys::load_secret_key(key_path)），与 keys_dir/{uuid}.key 完全无关；前端 KeyManagerPanel.vue 无「关联到会话」入口且注释明言私钥永不过 IPC；docs/08:11 未说明生成的密钥不参与认证。PROB-09/10 修复后密钥能落盘、能列出，但带口令加密的密钥在本应用内永远无法用于 SSH 连接。
- **证据**：复核（HEAD 82304ca）逐条成立：`grep -rn "get_private_key_data" src-tauri --include="*.rs"` 仅 key_manager.rs:543 定义一处（整理员复核一致），测试模块（:553 起 10 个 PROB-09/10 配套测试）也无调用，「按 key_id 取钥单测」不存在；rshell-api/src/types.rs:41-45 AuthMethod::PublicKey 仅 key_path: PathBuf 无 key_id；session/service.rs:70-76 组装 ResolvedAuthMethod::PublicKey{key_path, passphrase}；client.rs:593 精确命中 russh_keys::load_secret_key(key_path, passphrase.as_deref())；grep src/ keyPath|privateKey 零命中（exit 1）；KeyManagerPanel.vue:6 注释原文「只出 SshKeyInfo, 私钥永不过 IPC」，组件仅 listKeys/generateSshKey/importPrivateKey/deleteSshKey，无「关联到会话」入口；IPC 密钥命令仅 GenerateSshKey/ImportPrivateKey/DeleteSshKey/ExportPublicKey，无私钥读取命令；docs/08-incomplete-features.md:11 原文无边界说明。唯一保留：手改磁盘会话 TOML 填 keys_dir 绝对路径 + Keychain 口令存在理论旁路，但应用内无 UI 支撑、非 KeyManager 消费路径，不影响「本应用内无法使用」结论。PROB-09/10 已修复前提属实（:198-199/:209/:224-225/:305-310/:416-435）；docs/10:307 暂缓项 2 同题且 :11 明言状态不变。
- **修复方案**：接通消费路径（会话认证可选 KeyManager 密钥：按 key_id 经 get_private_key_data 取材并支持口令解密）；或在 docs/08 明示密钥管理当前仅为托管存储、不参与认证，避免误用。
- **验收标准**：二选一：存在从 KeyManager 密钥发起认证的路径（至少单测覆盖按 key_id 取钥）；或 docs/08 明确能力边界且 UI 提示与之一致。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-09：rshell-protocol 的 uuid 在 [dependencies] 与 [dev-dependencies] 重复声明（第一轮暂缓项 1）

- **位置**：`src-tauri/crates/rshell-protocol/Cargo.toml:25`
- **严重度**：low
- **问题描述**：:25（[dependencies] 末尾）与 :30（[dev-dependencies]）均为 `uuid = { workspace = true }`，dev 段重复项无任何作用（uuid 在 src/ssh/client.rs 正常使用，[dependencies] 声明必需）。属纯声明冗余，不影响构建产物。
- **证据**：复核实读 Cargo.toml 全文：:25 与 :30 均为 `uuid = { workspace = true }`（复核命令 `grep -n "uuid = " src-tauri/crates/rshell-protocol/Cargo.toml` 输出 25、30 两行，整理员复核一致）；uuid 在 rshell-protocol/src/ssh/client.rs:36 `use uuid::Uuid;`、:1496 `uuid::Uuid::new_v4()` 正常使用。对应 docs/10-known-issues.md:306 暂缓项 1（:11 明示暂缓项不随本轮关闭）；`git show 7258309:src-tauri/crates/rshell-protocol/Cargo.toml` 确认基线以来该文件未改动。复核顺带用 awk 扫描 src-tauri/crates/*/Cargo.toml 与 src-tauri/Cargo.toml，全 workspace 仅此一处双段重复声明，其它 crate 无同类冗余。
- **修复方案**：删除 [dev-dependencies] 第 30 行的重复 uuid 声明。
- **验收标准**：该文件 [dev-dependencies] 不再含 uuid；`cargo check`（或 scripts/verify.sh）通过。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-10：AppEvent 枚举与 TS 镜像漂移（PendingTunnelsSnapshot 缺失）且两侧各有一批零消费死变体

- **位置**：`src/ipc/types.ts:483`
- **严重度**：low
- **问题描述**：events.rs:101-104 定义的 PendingTunnelsSnapshot 在 TS 镜像 src/ipc/types.ts 中不存在（rust 34 变体 vs ts 33，逐变体差集唯一差即此），且 types.ts:506 注释把它列为切片 2.2 已删除、与后端 events.rs:152 注释的删除清单（不含它）互相矛盾。死变体双向清单实测成立——后端 0 发布点：TerminalTitleChanged、TransferTaskAdded/Completed/Failed、PendingTunnelsSnapshot、TunnelUpdated（全仓库 grep 仅定义处，其中前两者在 events.ts:71-72/91-92 的 makeDispatcher 有 case 但该 dispatcher 0 调用）；后端已发布但前端 0 监听：SessionUpdated、ScriptFinished、SyncInputSessionsChanged、SshKeyListChanged、SshKeyGenerated、PublicKeyExported、MasterPasswordChanged、MasterPasswordVerified、ActiveTunnelsChanged、ColorSchemeListChanged、PluginListUpdated、PluginStateChanged、PluginLoadFailed（后端 publish 点实存，前端引用仅 types.ts union 类型行）。同性质死面：CommandOutcome::PublicKey（outcome.rs:52）生产 0 构造，client.ts:173 exportPublicKey 前端 0 调用。镜像漂移当前无运行时后果（前端对未知事件默认忽略），但契约面失真、失去对账基准价值。
- **证据**：node 差集脚本对 src-tauri/crates/rshell-api/src/events.rs 与 src/ipc/types.ts（:422-504 union）提取变体名：rust count 34 / ts count 33 / rust-only [PendingTunnelsSnapshot] / ts-only 无。死变体判定基于前端全仓库 grep（不依赖订阅站点计数）：13 个 0 监听事件在前端的引用各仅 1 处且全部是 types.ts 类型定义行（如 types.ts:437/484/495/504）；后端 publish 点实存（session/service.rs:1010、command_dispatcher.rs:425/643/665-690、sync_input.rs:43、key_manager.rs:358-511、master_password.rs:83/:116-128、tunnel_manager.rs:406/436/488/549/699、theme/mod.rs:110）。前端订阅机制实测（整理员复核）：subscribeAppEvents 共 6 处调用点（App.vue:294、hostKey.ts:37、theme.ts:71、sessions.ts:118、QuickCommandPanel.vue:87、TriggerEditor.vue:84，其中 3 个 store 由 App.vue:283-289 的 subscribeEvents() 激活），底层 listen 仅 src/ipc/events.ts:29 一处；makeDispatcher 及其 EventDispatcher 接口本身也是 0 调用死代码。前端对未知事件默认忽略（events.ts:63/:101-104），漂移无运行时后果。（corrected：原 evidence 以「对照 6 个订阅站点」为据，复核修正为「判定基于全仓库 grep、不依赖站点计数」，订阅机制按上述实测表述。）
- **修复方案**：删除两侧死事件（或在前端真正接线，隧道两事件与 R2-04 一并处理）；若保留 PendingTunnelsSnapshot 则补进 types.ts 并修正 :506 注释；把 events.rs ↔ types.ts 变体集纳入对账测试（当前仓库无此测试）。
- **验收标准**：对账测试断言两侧变体集一致（或有明确白名单）；grep 无 0 发布点死变体、无 0 监听未白名单事件。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-11：IPC 契约对账测试只覆盖参数键：返回结构与事件契约完全不在对账范围

- **位置**：`tests/unit/ipcContract.spec.ts:229`
- **严重度**：low
- **问题描述**：实跑 `npx vitest run tests/unit/ipcContract.spec.ts` 4 passed：覆盖 rename_all=snake_case、参数键一致、带参命令有调用点、lib.rs 注册表与 commands.rs 一一对应；但全文（273 行）无任何解析 events.rs/types.ts/返回类型的断言——R2-10 的镜像漂移（rust 34 变体 vs ts 33，唯一差 PendingTunnelsSnapshot）与事件无人监听正是其盲区实证。另两个覆盖缺口：③把 client.ts 全部 helper 都算作调用点（:216-218），exportPublicKey 这类 0 UI 调用方的死导出也算已覆盖（其后端命令 export_public_key 真实存在 commands.rs:646，测试对该命令恒真）；键名对账无法发现薄壳 dispatch 错接同形变体——pauseTransfer/resumeTransfer 键集同为 {task_id}（client.ts:110-111；commands.rs:273/283），而 spec:247-249 仅比对排序后键集。
- **证据**：实跑 `npx vitest run tests/unit/ipcContract.spec.ts` → Tests 4 passed (4)；spec 全文实读（273 行），grep -i 'events\.rs|types\.ts|AppEvent|CommandOutcome|emit|listen' 零匹配（exit 1）；node 差集脚本 rust 34 vs ts 33、rust-only=[PendingTunnelsSnapshot]；grep exportPublicKey src/ tests/ 仅命中定义 client.ts:173。commands.rs 实测共 48 个手写命令 + 10 个 cmd! 宏命令（commands.rs:191/:192/:350/:460/:461/:616/:713/:714/:717/:720），spec 解析 58 个命令与 lib.rs generate_handler 注册 58 个一一对应（corrected：原表述「51 个薄壳 + 8 个 cmd!」计数有误）。本 spec 即第一轮 PROB-01 的修复物（文件头自称 PROB-01），本条属其覆盖缺口评估，非已修复问题重复。
- **修复方案**：把对账扩展到三张表：AppEvent 变体集（定义在 src-tauri/crates/rshell-api/src/events.rs:16，src-tauri/src/events.rs 仅为 EventBus→app.emit 桥）↔ types.ts AppEvent、CommandOutcome 变体 ↔ client.ts call<T> 返回标注、client.ts helper ↔ src/ 实际调用方（零调用白名单化）。
- **验收标准**：人为在 events.rs 增删一个变体或删除某 helper 的实际调用点时对账测试失败。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-12：删除已打开终端的会话后 dockview 残留指向已删除会话的僵尸面板

- **位置**：`src/App.vue:323`
- **严重度**：low
- **问题描述**：sessions.ts:103-112 deleteSessionById 只调后端 delete_session 并 refresh 列表；App.vue 无任何把会话删除映射到 dockview 面板关闭的代码（唯一 getPanel 用途是 ensureTerminalPanel(:157) 激活既有面板），terminal-{id} 面板残留，键入只会触发 IO 失败提示。App.vue 订阅的事件(:294-310)只处理 Transfer/Trigger/Compose 类，ConnectionStateChanged 不清理面板。
- **证据**：复核：sessions.ts:103-112 实读确认只调 IPC deleteSession + refresh()；getPanel 全 src 仅 App.vue:157 一处（激活既有面板），全前端无任何 panel.api.close() 调用（唯一 close() 为 CustomTitleBar.vue:64 窗口关闭）；App.vue:294-310 事件分支实读无 ConnectionStateChanged/SessionListChanged 清理，sessions.ts:119-126 同样只更新状态/refresh，也无 watch(store.items) 补偿清理；触发路径可达（SessionList ctxDelete 经 plugin-dialog confirm → commands.rs:130 → rshell-core session/service.rs:1018-1044 delete_session 真正 detach 并发布 SessionListChanged）；删除后 TerminalPane 键入 send_input 得 NotFound，TerminalPane.vue:120-137/:218 弹一次性 ElMessage，terminal-{id} 面板残留。轻微瑕疵：where 锚点 App.vue:323 实为 onBeforeUnmount(() => { 行，实际相关区域为 App.vue:151-182（dockviewApi/ensureTerminalPanel）与 :294-310（事件订阅），不影响结论。非 PROB-02/03 重复：二者恰是本问题成立的前置条件且已修复。
- **修复方案**：会话删除（或 SessionListChanged diff 出消失的 id）时调 dockviewApi.getPanel(`terminal-${id}`)?.api.close()。
- **验收标准**：删除已打开终端的会话后对应面板同步关闭，无残留僵尸面板。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-13：Ctrl+F/Escape 的 window keydown 无 sessionId 过滤：多终端面板并存时快捷键同时作用于所有面板

- **位置**：`src/components/TerminalPane.vue:104`
- **严重度**：low
- **问题描述**：onWindowKeydown(:104-111) 直接改自身 searchBarVisible，不校验事件属于哪个面板；:225 每个实例都 window.addEventListener("keydown")。多面板并存是 App.vue ensureTerminalPanel 每会话建一个面板且不关闭旧面板的必然结果；对照同文件 onTerminalAction(:113-118) 有 detail.sessionId 过滤，说明快捷键路径漏了同样的过滤。
- **证据**：复核：:104-111 Ctrl/Cmd+F 分支无任何 sessionId/激活判断直接 toggle，Escape 分支有 searchBarVisible 前置条件（同时作用于所有已打开搜索栏的面板）；:225 每实例注册、:251 卸载；对照 onTerminalAction :114-115 `detail.sessionId !== props.sessionId` 即 return，且 App.vue:269 工具栏查找按钮正是走该 CustomEvent 路径——证明快捷键路径漏了同样的过滤；多面板并存必然（App.vue:154-167 以 terminal-{sessionId} 为 panelId，已存在仅 setActive 不关闭旧面板）；非激活面板保持挂载已实证：dockview-core 7.0.4 content.js:118/:121 用 element.style.display 显隐面板，组件不卸载、监听器持续活跃，N 个面板并存时一次 Ctrl+F 同时 toggle 所有面板的搜索栏。grep docs/10 无相关条目；git show 82304ca 对该文件只改 on_data 键名、PROB-13 错误条与 IO 提示，未动 onWindowKeydown。
- **修复方案**：仿照 rshell:terminal-action 的 sessionId 过滤路由快捷键，或仅让当前激活面板响应 window 级快捷键。
- **验收标准**：两终端面板并存时按 Ctrl+F 仅当前激活面板弹出搜索栏。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-14：触发器删除无任何确认，是当前唯一没有确认流的删除入口

- **位置**：`src/components/TriggerEditor.vue:61`
- **严重度**：low
- **问题描述**：remove(:61-64) 直接 await deleteTrigger(t.id) 后 refresh，模板 :120 删除按钮直接触发；对照同类删除均有确认：SessionList.vue:126 与 KeyManagerPanel.vue:120 用 plugin-dialog confirm，QuickCommandPanel.vue:75 用 ElMessageBox.confirm，transfer/TransferWorkspace.vue:149（远程文件删除）也先 confirm。
- **证据**：复核：TriggerEditor.vue:61-64 无任何确认；四个对照点逐行属实。grep 全 src/ 盘点删除入口共 5 处（会话/SSH 密钥/快速命令/远程文件/触发器），仅触发器无确认流（TunnelPanel.vue:102 的 remove 是「关闭」非删除持久数据；deleteHostKey client.ts:202 无组件调用方）。fixPlan 用 ElMessageBox.confirm 在 wry/WKWebView 可用（QuickCommandPanel 先例），避免第一轮 PROB-03 揭示的 window.confirm 无实现问题；grep docs/10 无触发器删除确认条目，PROB-03 修复的正是 SessionList/KeyManagerPanel 的确认（即本条对照点来源），TriggerEditor 从未被覆盖。
- **修复方案**：与快速命令面板一致，删除前加 ElMessageBox.confirm。
- **验收标准**：删除触发器先弹确认，取消则不删除。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-15：TerminalChannels::detach 生产代码零调用：每会话 sink 条目长期留存于 HashMap

- **位置**：`src-tauri/src/terminal.rs:107`
- **严重度**：low
- **问题描述**：grep "\.detach(" 仅命中 terminal.rs:159 的单元测试，生产代码零调用；SessionService::detach_session/delete_session 及 reader 任务清理均不调用 terminal_channels，条目（Buffering 固定预分配 256KiB 缓冲或 Attached 的 Channel 句柄）随历史连接过的会话数无上界累积（重连同 ID 会复用条目故非按次增长，但长期运行仍缓慢增长）。
- **证据**：复核：terminal.rs:107-109 为 detach 定义；`grep -rn "\.detach(" --include="*.rs" src-tauri/`（排除 target）仅命中 terminal.rs:159 单测，生产零调用（其余命中为 rshell-core 的 SessionService::detach_session 同名不同物与注释）；SessionService::detach_session 实际位于 rshell-core session/service.rs:768-795（由 disconnect :763-765 与 delete_session :1018→:1037 调用），仅清理 sessions/connections/protocol_connections 并发布 ConnectionStateChanged；reader 清理段 :554-571/:740-757 同样不做 TerminalChannels 清理；rshell-core crate grep 无 TerminalChannels 引用（壳层 crate 反向不依赖）。增长机制：lib.rs:74-78 转发任务对每个有输出的 session_id 调 push → terminal.rs:54-57 entry().or_insert_with 建 Buffering(VecDeque::with_capacity(256KiB))（terminal.rs:24），条目固定预分配约 256KiB 而非「最多」，问题实际略重于原表述；attach_terminal（commands.rs:772）直接 insert Attached 条目；HashMap 键为 session_id，同 ID 重连复用，非按连接次数增长。docs/10 中 detach_session 相关条目（:99）是隧道句柄陈旧问题，与本条不同。
- **修复方案**：在 SessionService 断开/删除路径或 reader 任务清理处按 session_id 调用 TerminalChannels::detach。
- **验收标准**：会话删除后该 session_id 的条目从 terminal_channels 移除（新增单测覆盖）。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-16：SOCKS5 握手对 greeting 与 request 各用单次 read 即判长：TCP 分段到达的合法握手被误判协议错误

- **位置**：`src-tauri/crates/rshell-core/src/security/tunnel_manager.rs:766`
- **严重度**：low
- **问题描述**：socks5_handshake 中 :766-769 单次 read 后 :774 `n < 2 + nmethods` 直接报 "greeting truncated"；:787-792 request 同样单次 read 后 `n < 7` 报 "request too short"（后续 :808-809/:816-819/:827-828 对 ipv4/domain/ipv6 同样单次判长），均无补读循环。回环客户端通常单包无碍，但 read 返回部分数据是 TCP 合法行为。
- **证据**：复核亲读 :761-851 全函数：全函数无任何循环补读或 read_exact。单点实验（/tmp 临时项目复刻该函数逻辑，客户端分段写 greeting：先 write 1 字节 [0x05] 并 flush，sleep 80ms 再写 [0x01,0x00]）：输出 `SERVER handshake result: Err("socks5 bad greeting: n=1 ver=5")`，合法分段握手被误判协议错误并断开，实测复现；request 阶段结构相同同理成立。现有 4 个 socks5 握手测试（:1422/:1467/:1510/:1547）全部单次 write_all 完整报文，无分段场景覆盖，验收所需测试缺失属实；docs/10 仅 PROB-23（bind_address/no-auth）相关，非重复。
- **修复方案**：改为按需循环读取（先读 VER+NMETHODS 再补齐 methods，request 同理），或用 tokio_util 的 codec/读满 helper。
- **验收标准**：单测模拟握手字节分段两次 write（中间 flush），握手成功建立连接。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-17：传输执行任务 spawn 后句柄丢弃且无 panic 保护：panic 会跳过控制通道清理与终态写回

- **位置**：`src-tauri/crates/rshell-core/src/transfer/service.rs:365`
- **严重度**：low
- **问题描述**：tokio::spawn 闭包（:365-446）中 controls.remove（:439-442）与 finalize_transfer（:445）按顺序写在闭包体内，无 RAII/defer 兜底；unwind 时两者均被跳过，任务表永久停在非终态、control_channels 泄漏条目，后续 pause/resume/cancel 对幽灵任务操作（状态改写但无循环消费信号）。当前闭包内无已知 panic 点（apply_progress/sftp 路径无 unwrap），属结构性防御缺失。
- **证据**：复核：行号逐一吻合（spawn 闭包 :365-446、controls.remove :439-442、finalize_transfer :445），JoinHandle 未绑定即丢弃（:446 `});` → :448 `Ok(())`），panic 后无人感知；`grep -rn "catch_unwind|AssertUnwindSafe" src-tauri/crates/` 零匹配。后果链成立：control_channels 条目残留；任务表停 Transferring/Paused 非终态，而 prune_finished_tasks（:506-521）只清理 is_terminal() 任务，非终态条目永不清除；pause/resume/cancel（:527-595）仍取到幽灵条目的 Sender，watch channel 无接收端 send 也不报错。自我限定核实为真：闭包体内直接代码无 panic 点，apply_progress（:619-656）无 unwrap，grep sftp.rs 的 unwrap 匹配全为 unwrap_or_default 或 #[cfg(test)] 代码（:390 起）；SshClientHandle = Arc<tokio::sync::RwLock<SshClient>>，read() 不 panic。docs/10 的 PROB-04（控制通道缺失）与 PROB-18（任务表无界增长）为不同问题且已修复（当前代码已有 watch 通道与 prune_finished_tasks），非重复。
- **修复方案**：用 guard 结构（Drop 时 remove 控制通道并兜底 finalize）或 catch_unwind 包裹任务体，保证清理必达。
- **验收标准**：注入 panic 的测试（或代码评审确认 guard 落地）下 control_channels 条目被移除、任务终态被写回。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-18：触发器正则在每条终端输出块上重新编译（recv 循环热路径），开销随触发器数×输出块数线性增长

- **位置**：`src-tauri/crates/rshell-core/src/script/trigger_engine.rs:217`
- **严重度**：low
- **问题描述**：check_output 的 RegexAppear 分支每次调用 Regex::new（:216-223）；check_output 被 SSH reader（session/service.rs:519）与 Telnet/Serial 循环（:710）在每个输出块调用（SSH 每条消息、协议层每 8KiB），而 create_trigger 在 :94-96 已编译验证过却不缓存。
- **证据**：复核实读 trigger_engine.rs 全文：RegexAppear 分支 :217-218 每次调用 Regex::new(pattern) 后才 is_match（:219-222 为失败兜底）；create_trigger :94-95 仅做校验、编译结果即弃；TriggerEngine 结构体（:18-25）无任何 Regex 缓存字段，全文件仅 :94/:217 两处 Regex::new（grep 验证）。session/service.rs:519 位于 SSH reader 循环、每条 output_rx.recv() 消息调用一次；:710 位于 Telnet/Serial 循环（buf=[0u8;8192]，:687），每 8KiB 块（:704）调用一次。复杂度表述成立：每输出块对每个启用的 RegexAppear 触发器各编译一次（ExactMatch 走 contains 不编译）。补充：with_path（:44-61）从文件加载触发器时不验证正则，正是 check_output 保留 Err 兜底的原因，fixPlan「创建/加载时预编译」已涵盖该路径。git log --follow 该文件仅 3 个早期 commit 触及，82304ca 未动；docs/10 无 trigger 条目。
- **修复方案**：触发器创建/加载时预编译 Regex 缓存（旁路 HashMap<trigger_id, Regex>），check_output 只做 is_match。
- **验收标准**：check_output 路径无 Regex::new（grep/评审可查）；触发器行为既有单测不变。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-19：xtask 的 dev 子命令引用已删除的 rshell-ui 包，必然失败；文档注释与 help 仍在宣传

- **位置**：`src-tauri/crates/xtask/src/tasks/dev.rs:6`
- **严重度**：low
- **问题描述**：dev.rs:6 为 `cargo run -p rshell-ui`，而 workspace 实际包为 rshell/rshell-api/rshell-core/rshell-infra/rshell-plugin-sdk/rshell-protocol/xtask（RDP/UI 重构已删 rshell-ui），该子命令按现状必然失败。main.rs:9/:33 与 help.rs:8 的文档注释/帮助输出仍在宣传这条失效命令，且「rshell-ui」正是 check-docs.mjs:11 列为过时架构的禁用词。影响限于该死子命令（docs 与 scripts 无 xtask 引用）。
- **证据**：复核：dev.rs:6 为 `cargo(["run", "-p", "rshell-ui"])`（Read 确认）；实跑 `cargo metadata --no-deps` 包列表 ['rshell','rshell-api','rshell-core','rshell-infra','rshell-plugin-sdk','rshell-protocol','xtask'] 与报告一致；实跑 `cargo run -p rshell-ui` 输出 `error: package(s) \`rshell-ui\` not found in workspace /Users/letmlook/code/rshell/src-tauri`，exit 101，逐字一致；main.rs:9（`//! cargo xtask dev      # cargo run -p rshell-ui`）、main.rs:33（`/// 启动 rshell-ui 开发版本`）、help.rs:8、check-docs.mjs:11 禁用词正则均确认；grep docs/、scripts/、README.md、CLAUDE.md、CONTRIBUTING.md、scripts/verify.sh 均无 xtask 引用；docs/10 无记录，非已修复项重复。
- **修复方案**：dev 子命令改为启动现有 rshell 包（如 `cargo run -p rshell` 或转发 npm run tauri:dev），同步修正 main.rs 文档注释与 help.rs 输出；或删除该子命令。
- **验收标准**：`cargo xtask dev` 能启动开发环境（或子命令已移除）；help.rs 输出与 main.rs 注释不再出现 rshell-ui。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-20：修复轮改写的四份文档头部日期戳未更新，两轮修复混在同一日期下

- **位置**：`docs/09-macos-validation.md:3`
- **严重度**：low
- **问题描述**：commit 82304ca（提交日期 Fri Oct 2 16:34:23 2026 +0800）改写了 docs/05-development-standards.md、docs/08-incomplete-features.md、docs/09-macos-validation.md、scripts/README.md 正文，但 HEAD 上四份文档头部仍标「更新：2026-09-26 / 日期：2026-09-27」（docs/05:3、docs/08:3、docs/09:3、scripts/README.md:3）。docs/09:26「本轮另修复：…传输 pause/resume 现已接入传输队列界面」描述的是体检修复轮行为，却落在 2026-09-27 日期之下，两轮修复混在同一日期，削弱验证记录作为证据文档的可追溯性。
- **证据**：复核实跑（HEAD=82304ca）：`git show 82304ca --stat` 显示该 commit 改写 docs/05(+2)、docs/08(2±)、docs/09(2±)、scripts/README(3±)，Date 为 Fri Oct 2 16:34:23 2026 +0800；四文件头部实读与原报告一致；`git show 82304ca` diff 显示 docs/09:26（pause/resume 接入传输队列界面表述，对应本轮 transfer/service.rs +496、sftp.rs +285 加固）、docs/08:23、scripts/README.md:17-18（Windows 脚本已删除 2026-09-30）均为审计轮写入；惯例佐证 `git show 6b11802:docs/09-macos-validation.md` 第 3 行已刷为「日期：2026-09-27」，证明惯例确为每轮修订刷新验证记录日期；`git show 7258309 --stat` 确认 9-27 体检接线 commit 未改 docs。docs/10 无此问题记录；:26 技术描述本身与代码事实相符，严重度 low 恰当。
- **修复方案**：把四份文档头部更新日期改为实际修订日期；或在 docs/09 中为体检修复轮单独立一节并标注日期，保持「验证记录按轮次记日期」的仓库惯例。
- **验收标准**：四份文档头部日期与最近实际修订 commit 日期一致；docs/09 按轮次分节记日期。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

### R2-21：execute_transfer 的 mark_failed 路径无终态守卫：启动窗口内已取消的任务可被改写为 Failed（R2-01 修复中发现）

- **位置**：`src-tauri/crates/rshell-core/src/transfer/service.rs:358`
- **严重度**：low
- **问题描述**：execute_transfer 在 provider 不可用（:356-361）与 provider 出错（:363-370）两个分支调用 mark_failed，而 mark_failed（:726-741）无条件把任务置 Failed、写 error_message 并广播 TransferFailed。R2-01 修复后，provider await 窗口内到达的 cancel_transfer 会先把任务置为 Cancelled 终态；若 provider 随后才返回错误（如会话在取数期间断开），任务即被改写为 Failed，取消意图被终态覆盖——与 R2-07 同属「终态互相改写」，但站点不同（mark_failed 的两个生产调用点，非 cancel_transfer），R2-07 的修复方案不覆盖此处。mark_completed（:709-723）同样无守卫，但全仓库无生产调用点（仅测试使用），暂无实际路径。
- **证据**：R2-01 修复过程中实读确认：`grep -rn "mark_failed" src-tauri/crates/rshell-core/src --include="*.rs"` 生产调用仅 service.rs:358/:367 两处（其余为 tests）；mark_failed 实现（:726-741）对任何找到的任务无条件 `task.state = TransferTaskState::Failed`（:730），无 is_terminal 检查；对比 finalize_transfer（:489-547）的 Ok/Err 分支均显式保留 Cancelled，守卫缺失仅在此路径。
- **修复方案**：mark_failed（及 mark_completed）仅在任务非终态时写回：`if !task.state.is_terminal()`（与 R2-07 对 cancel_transfer 的修法一致）。
- **验收标准**：单测：对已 Cancelled 任务调用 mark_failed 后状态仍为 Cancelled 且不广播 TransferFailed；非终态任务的失败路径行为不变；`cargo test -p rshell-core transfer::service::tests` 全绿。
- **状态**：已修复（验证方式：fmt/clippy/typecheck/npm test/cargo test 快速检查）

## 三、未证实发现

本轮无。20 条问题全部经独立复核成立（confirmed 17 条、corrected 3 条），无 refuted 条目；复核过程中对个别原始表述的修正（R2-01 fixPlan 方案二、R2-10 订阅站点表述、R2-11 命令计数与 events.rs 位置、R2-12 where 锚点、R2-15 缓冲大小表述）均已并入对应条目，不构成被推翻的发现。

## 四、暂缓项

本轮无新增暂缓项。第一轮（docs/10-known-issues.md 第三节）暂缓项 1（uuid 重复声明）与暂缓项 2（KeyManager 密钥无认证消费方）本轮已转正为正式条目 R2-09、R2-08，不再是暂缓状态。注：R2-08 的修复方向含产品决策（接通消费路径 vs docs/08 明示能力边界），若本轮修复容量不足以接通消费路径，至少应完成文档边界明示这一最低成本路径。

## 五、最终验证记录

> **验证日期**：2026-10-03
> **验证命令**：`bash scripts/verify.sh --skip-install`
> **结果**：**全部通过**，脚本退出码 0。

说明：本轮全量验证由仓库统一入口 `scripts/verify.sh` 收口执行（`--skip-install` 跳过依赖安装），脚本逐级 fail-fast，本次全程无失败步骤。收口日志末尾可见 rshell-protocol 测试 `test result: ok. 38 passed; 0 failed` 与 doc-test `1 ignored`。下表中各分项结果凡标注「本会话实测」者为此前各条目修复时实际执行的命令与输出，与收口结果一致。

各步骤结果：

| # | 检查项 | 实际命令 | 结果 |
| --- | --- | --- | --- |
| 1 | 前端类型检查 | `npm run typecheck`（vue-tsc --noEmit） | 通过（本会话实测 exit 0，无错误） |
| 2 | 前端单测 | `npm test`（vitest run） | Test Files **23 passed (23)**、Tests **110 passed (110)**（本会话实测） |
| 3 | 前端生产构建 | `npm run build`（vue-tsc --noEmit && vite build） | 通过（verify.sh 收口执行，退出码 0） |
| 4 | 文档契约检查 | `npm run check:docs` | 通过（Documentation contract passed, 14 current documents; dated historical records excluded；本会话实测） |
| 5 | 构建产物检查 | `npm run check:bundle` | 通过（verify.sh 收口执行，退出码 0） |
| 6 | 维护脚本自检 | `npm run test:scripts`（node --test scripts/*.test.mjs） | 通过（verify.sh 收口执行，退出码 0） |
| 7 | Rust 格式检查 | `cargo fmt --all --check` | 通过，无 diff（本会话实测） |
| 8 | Rust 静态检查 | `cargo clippy --workspace --all-targets -- -D warnings` | 通过，`-D warnings` 下无告警（本会话实测） |
| 9 | Rust 全量测试 | `cargo test --workspace` | 通过。收口日志可见 rshell-protocol **38 通过 / 0 失败**（与收口前分 crate 实测一致）；本会话另实测 rshell-core **171 通过 / 0 失败**、rshell-api **6 通过 / 0 失败**；其余 crate 与集成测试目标由收口脚本统一覆盖，退出码 0 |

与体检基线对比（基线计数见 [docs/10](10-known-issues.md) 第五节）：前端单测由 21 文件/87 用例增至 **23 文件/110 用例**（新增 R2-10/R2-11 契约对账、R2-13/R2-14/R2-04/R2-12 组件回归、R2-02 错误形状转换等）；rshell-core 由 153 增至 **171**、rshell-protocol 由 37 增至 **38**（新增传输竞态/守卫、known_hosts IPv6、SOCKS5 分段握手、触发器缓存、会话删除回调等回归），rshell-api 保持 6。上述结果对应第二节 **21 条**问题修复（R2-01～R2-20 原始 20 条 + 修复中新增并已修复的 R2-21）：全部条目先经 fmt/clippy/typecheck 快速检查覆盖，由本节全量验证统一收口。
