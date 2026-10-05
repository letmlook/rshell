# RShell 第三轮问题清单

> **体检日期**：2026-10-05
> **基线 commit**：`4358c85`（完整哈希 `4358c85`，fix: give the NSIS installer its own application icon）
> **上一轮基线**：`82304ca`（docs/11-known-issues-round2.md）。本轮基线之后另有 9 个 commit 未审，逐条见第一节。
> **基线验证状态**：通过。`npm run typecheck` 退出 0；`npm test` 34 文件 / 234 用例全绿（3 skipped）；`cargo clippy --workspace --all-targets -- -D warnings` 退出 0；`cargo test --workspace` 退出 0（rshell-core 177、rshell-protocol 54、rshell-infra 18、rshell-api 6）。
> **本轮特征**：**全套检查全绿**。下列 16 条无一被现有测试、typecheck 或 clippy 捕获——这本身延续了前两轮记录的「系统性假绿」特征，见第五节。

## 一、概述

**问题总数与分布**：本轮共立条 **16** 个问题——**high 3 条**（R3-01～R3-03）、**medium 7 条**（R3-04～R3-10）、**low 6 条**（R3-11～R3-16）。另记录**经复核未成立 9 条**（第三节），其中 6 条是对前两轮已修复项的复核确认，3 条是本轮提出后被证伪的假设。

**一句话总结**：三条 high 里有两条是**同一类根因**——「持锁跨远端 await」；R3-01 是本轮唯一会导致**用户必须重启应用**的缺陷（R3-03 是它的次生面）。R3-04 是唯一的后端路径穿越，且其远端孪生实现 `sibling_path` 已有正确检查，**只差几个条件判断**。R3-05/06 集中在刚落地的「每标签独立 pty」功能（commit `6fb5671`），是用户最容易撞到的两条。

**基线之后未审的 9 个 commit**：

| commit | 主题 | 与本轮的关系 |
|---|---|---|
| `23b65bd` | 管道化 SFTP 下载、传输队列移除、缩短终端标签 | R3-08 的引入背景 |
| `c6061f5` | 文件列表右键菜单、传输队列生命周期、剪贴板快捷键 | R3-14 的引入背景 |
| `5d00363` | 关闭 SSH Nagle、池化 SFTP 下载句柄 | 池化已复核无串位（第三节） |
| `18b9dcd` | 授予 WebView 全局读写文件系统 scope | **产品决策，非缺陷**（第三节） |
| `a59db8a` | dev profile 依赖代码生成优化 | 无关 |
| `5f6dff3` | 退格修复、路径栏导航、传输冲突处理 | R3-04 / R3-10 的引入背景 |
| `7891d8c` | 文件/传输/终端 UI  overhaul、应用内弹窗 | R3-05 / R3-06 的引入背景 |
| `6fb5671` | 每标签独立 pty、布局与设置打磨 | **R3-01 / R3-05 的引入背景** |
| `4358c85` | NSIS 安装包独立图标 | 无关 |

## 二、问题清单

### R3-01：SSH 发送窗口打满会永久卡死整条会话，只能重启应用

- **位置**：`src-tauri/crates/rshell-protocol/src/ssh/client.rs:818`（actor 内联 await）、`:853-860`（`close_terminal`）、`:909-933`（`disconnect_ssh`）；持锁点 `src-tauri/crates/rshell-core/src/session/service.rs:832-833`
- **严重度**：high
- **问题描述与证据**：pty actor 在 `select!` 的 `Send` 分支里**内联** `await channel.data(...)`（client.rs:818）。SSH 通道窗口（russh 0.48.2 `mod.rs:1506` `window_size` 默认 2097152，本应用未覆盖，见 client.rs:65-68）被填满时，`Channel::data` → `send_data` → `ChannelTx::poll_write` → `poll_writable` 返回 `Poll::Pending`，只在收到对端 `WINDOW_ADJUST` 时被 `Notify` 唤醒（russh-0.48.2 `src/channels/io/tx.rs:73-102`）。actor 因此再也不回到 `select!`，后续 `Resize`/`Close` 请求全部滞留在 mpsc（容量 32）中不被处理。此时 `close_terminal`（client.rs:856）的 `handle.sender.send(...)` **仍然成功**——通道有容量且接收端 actor 任务还活着，只是卡住——于是 client.rs:857 的 `let _ = received.await` 无限挂起而不是报错。`disconnect_ssh`（client.rs:912-915）逐个 `await close_terminal(id)`，而其调用方在 session/service.rs:832 `let mut client = conn.client.write().await;` 并**持有 `SshClient` 写锁跨越整个 await**，且无超时。写锁永不释放 → `open_terminal`（新标签）、`close_terminal`、会话删除、重连、以及每一次 `send_data` IPC 全部永久阻塞。
  **触发**：远端运行不读 stdin 的进程（`python3 -c 'import time;time.sleep(1e9)'`，或 Ctrl+Z 挂起前台作业），向终端粘贴/灌入 >2 MiB 数据，随后断开连接。
  **影响**：该会话在 UI 上无法关闭、无法重连、无法输入；只有重启应用能恢复。属本轮唯一「普通操作即可触发且不可自愈」的缺陷。
- **修复方案**：拆卸路径不要 await `Close` 应答——actor 在 sender 被 drop 时已有 `None => { let _ = channel.close().await; break }` 分支（client.rs:830），故 `disconnect_ssh` 可直接 drop sender 后继续 `handle.disconnect()`；给 `close_terminal` / `send_data_to` 的应答等待加超时兜底；不要持写锁跨 `disconnect_ssh`。
- **验收标准**：构造一个不读 stdin 且不调整窗口的假 SSH 服务端，灌入 >2 MiB 后调用断开，断言 `disconnect_ssh` 在超时内返回、断开后 `open_terminal` 仍可用；actor 卡死场景下 `close_terminal` 不再无限等待。
- **状态**：已修复（详见第五节修复轮记录）

### R3-02：关闭或挂起隧道不会撤销已建立的转发连接

- **位置**：`src-tauri/crates/rshell-core/src/security/tunnel_manager.rs:616-645`（每连接 spawn，句柄于 `:618` 丢弃）；对照 `:421-429`（close）、`:478-480`（suspend）、`:678-680`（会话断开 deactivate）
- **严重度**：high
- **问题描述与证据**：accept 循环任务的 `JoinHandle` 被保存并在 close/suspend/deactivate 时 abort，但 `:618` 为**每条已接受的连接**另起的 `tokio::spawn` 其句柄被直接丢弃。该任务捕获 `inbound`（TcpStream）与 `ssh_client`，在 `forward_local` 内以 `tokio::io::copy` 双向中继直至一侧关闭。文件中不存在任何其他取消路径。因此关闭一个暴露到局域网的隧道后，**已连上的客户端继续享有完整访问权**，而 `close_tunnel` / `suspend_tunnel` 已返回 `Ok` 并广播 `TunnelStateChanged{Error("Closed")}`——UI 显示已关闭，实际未撤销。
  **触发**：撤销 LAN 暴露后立即关闭隧道，而此时有客户端保持着活跃的转发连接；或会话在转发中途掉线。
  **影响**：「关闭隧道」这一安全语义不成立，用户无法凭此收回已授予的访问。
- **修复方案**：把每连接的 `JoinHandle`（或 `CancellationToken`）存入 `ActiveTunnel`，在 close / suspend / deactivate 时一并 abort；`connections_count` 的递减需容忍任务被 abort（当前 `:641-644` 在任务体内，abort 后不再执行，需在 abort 前归零或改用 Drop 守卫）。
- **验收标准**：建立一条活跃转发连接后 `close_tunnel`，断言该连接的字节中继随即终止（写不再到达远端）；`suspend_tunnel` 同；单测断言 abort 后 `connections_count` 归零且无任务泄漏。
- **状态**：已修复（详见第五节修复轮记录）

### R3-03：隧道 accept 持读锁跨远端 await，卡死该会话所有写操作

- **位置**：`src-tauri/crates/rshell-core/src/security/tunnel_manager.rs:714-719`
- **严重度**：high
- **问题描述与证据**：
  ```rust
  let channel = {
      let client = ssh.read().await;              // 守卫
      client.open_direct_tcpip(remote_host, …).await   // 跨 await 存活
  };
  ```
  tokio `RwLock` 写优先，任何写者都排在只读守卫之后：`SessionService::open_terminal`（session/service.rs:963）、`close_terminal`（:1000）、`disconnect_ssh`（:574、:832）、触发器 `Disconnect` 动作（session/service.rs:159-162）全部挂起。russh 0.48.2 `client/mod.rs:511-534` 的 `wait_channel_confirmation` 在 `receiver.recv().await` 上循环且**无超时**，rshell 也未加。
  **触发**：Local / Dynamic 隧道上，被连端发起连接而 SSH 对端不回应 `direct-tcpip` channel-open（恶意或停滞的服务端、包黑洞的中间设备）。
  **影响**：该会话无法开新标签、无法干净断开；与 R3-01 同属「持锁跨远端 await」根因，且**无自愈路径**。
  **同时违反 CLAUDE.md**：「不要在等待远端输入时阻塞发送路径」。
- **修复方案**：await 前释放守卫（clone `russh::client::Handle`，或在窄作用域内借用），并给 `open_direct_tcpip` 套 `tokio::time::timeout`。
- **验收标准**：假服务端接受 TCP 但不回应 channel-open，断言 `open_direct_tcpip` 超时返回且 `open_terminal` / `disconnect_ssh` 在超时内不被阻塞。
- **状态**：已修复，但**未补自动化回归测试**。修复方式是在协议层给 `open_direct_tcpip` 加 `DIRECT_TCPIP_OPEN_TIMEOUT`（15s）上限，使隧道路径持有的 `SshClient` 读锁有界；「对端接受 TCP 但不回应 channel-open」这一前提需要真实的 SSH 服务端或 loopback russh server 配合可注入的停滞行为，本轮未搭建，故此处只做静态保证。如实记录：该条目前靠超时兜底，**未经运行时验证**。

### R3-04：下载「重命名」是任意本地路径写入——本地版漏了远端版已有的越界检查

- **位置**：`src-tauri/crates/rshell-core/src/transfer/service.rs:304-317`（对照远端孪生 `sibling_path` `:260-286`）
- **严重度**：medium
- **问题描述与证据**：`resolve_local_target` 的 `ConflictPolicy::Rename(name)` 分支只拒绝 `name.is_empty() || name == "." || name == ".."`，**不检查 `/`、`\`、`\0`、纯空白**。同一文件 :260-286 的 `sibling_path`（远端版）逐项全部拒绝，注释原文写明「否则『重命名』会成为任意路径写入的旁路，绕过 `validate_remote_mutation_path` 的越界检查」；而 `resolve_local_target` 自己的文档注释（:288-290）却声称「本地下载目标解析：语义与远端版一致」——**实现与注释不符**。
  `Path::with_file_name` 不做净化（已用 rustc 实测确认）：
  ```
  C:\Users\me\Downloads\report.txt  +  ..\..\Startup\evil.exe
    => C:\Users\me\Downloads\..\..\Startup\evil.exe     （解析后逃出目标目录）
  /home/user/downloads/report.txt   +  C:evil.txt
    => C:evil.txt                                          （整条路径被替换为驱动器相对路径）
  ```
  且 `enqueue_download` 的父目录存在性检查（service.rs:443）跑在改名**之前**，逃逸后的路径不再复验绝对性/非根/父目录。`ConflictPolicy` 从 IPC 原样传入（`src-tauri/src/commands.rs:304-314`、`src/ipc/client.ts:168-169`），后端不做任何校验。随后 `SftpClient::download` 还会 `create_dir_all` 逃逸出的父目录并写入**远端主机的内容**。
  **违反 CLAUDE.md**：「SFTP 写操作只接受绝对、非根、无歧义路径」。
  **可达性**：当前 UI 走不到——`client.ts:169` 默认 `Fail`，且前端 `askConflict` 的 validate 拒绝 `\` 与 `/`。属**后端契约缺口**：WebView 现已具备全局 fs scope（`18b9dcd`），注入脚本即可构造 `Rename` 负载。
- **修复方案**：把 `sibling_path` 的拒绝集（`/`、`\`、`\0`、`.`、`..`、空/纯空白）应用到本地改名名；并对**解析后**的路径重跑绝对 / 非根 / 父目录存在性校验，而非只校验原始 `local`。
- **验收标准**：单测断言 `resolve_local_target(existing, Rename("../../evil.txt"))`、`Rename("sub/x")`、`Rename("C:evil.txt")` 全部返回 `InvalidState`；断言解析后路径仍满足绝对且位于原父目录下。
- **状态**：已修复（详见第五节修复轮记录）

### R3-05：握手中途新建的标签页永久空白，必须用户手动重试

- **位置**：`src/App.vue:454-467`、`src/components/TerminalPane.vue:75-114`（无恢复 watcher）、按钮启用条件 `src/components/WorkspaceToolbar.vue:170`
- **严重度**：medium
- **问题描述与证据**：`openTabSession` 在 `connectionState === "connecting"` 时**跳过 await 直接**执行 `ensureTerminalPanel(id, { forceNew: true })`（App.vue:458 的守卫把 "connecting" 与 "connected" 同等对待，:466 无条件建面板）。`store.connect` 同步置 `connectionState = "connecting"`，故点击「新开标签」必然命中该分支。新面板拿到新 `terminalId`，`TerminalPane.ensurePty` 调 `open_terminal`，后端因会话尚未进入 `connections` 而返回 `NotFound`（session/service.rs:953-960），面板渲染永久错误条。握手随后完成时，**没有任何代码重跑 `ensurePty`/`attach`**：TerminalPane 仅有的两个相关 watcher 一个只置 `everConnected`（:75-81）、一个只切 `disableStdin`（:108-114）。工具栏按钮只按 `!terminalAvailable` 禁用，不含 connecting 态。
  **违反 CLAUDE.md**：「可见按钮必须调用真实操作或明确禁用，不能静默成功」。
  **触发**：点选一个从未连接过的会话（左列表 `selectSession` 先设 `activeTerminal` 再开始连接），握手进行中点「新开标签」。双击则产生两个死标签。
- **修复方案**：`openTabSession` 对 "connecting" 应 `await` 连接完成再建面板，而非跳过；工具栏按钮在 `connectionState === "connecting"` 时禁用；`TerminalPane` 增加 `disconnected→connected` 的 watcher 重跑 `ensurePty` + `attachTerminal`（作为兜底，覆盖其它进入死标签的路径）。
- **验收标准**：握手中点击「新开标签」，面板在连接完成后自动出现 shell 提示符，无需点「重试附加」；按钮在 connecting 态为 disabled。
- **状态**：已修复（详见第五节修复轮记录）

### R3-06：`TerminalPane` 的 async `onMounted` 在卸载后继续执行并抛错

- **位置**：`src/components/TerminalPane.vue:479-484` 对照 `:562-578`
- **严重度**：medium
- **问题描述与证据**：`onMounted` 内含两次 await（:479 `ensurePty`、:480 `attachTerminal`）。期间关闭标签 → `onBeforeUnmount` 执行 `term = null`（:576）、`channel = null`（:577）→ await 续体恢复后在 :484 执行 `term.onData(...)`，对 `null` 取属性抛 `TypeError: Cannot read properties of null (reading 'onData')`，成为 async `onMounted` 的 unhandled rejection。**且 :484 之后的全部注册被跳过**：`rshell:terminal-action` / `rshell:terminal-theme` window 监听、document 的 mousedown/keydown 捕获监听、以及 :526 的 `ResizeObserver` 都不再挂载；`attachError.value` 在卸载后才被写入。
  **违反 CLAUDE.md**：「组件和 store 的事件订阅必须释放，并处理异步订阅完成晚于 unmount 的情况」。
  **触发**：打开终端标签后立刻关闭（窗口约一个 Tauri 往返）。每个终端标签都有两处这样的 await。
- **修复方案**：加 unmounted 闩锁（`App.vue:96`、`stores/sessions.ts:34-35`、`QuickCommandPanel.vue:25` 已是此模式），在每次 await 之后、触碰 `term` 之前检查。
- **验收标准**：单测在 `ensurePty`/`attachTerminal` 挂起期间同步 unmount，断言无未捕获异常、`term.dispose()` 被调用一次、window/document 监听已注册或成对释放。
- **状态**：已修复（详见第五节修复轮记录）

### R3-07：Telnet 子协商不终止导致输出永久吞掉 + 内存无界增长

- **位置**：`src-tauri/crates/rshell-protocol/src/telnet/mod.rs:266-268`，入口 `:192-195`
- **严重度**：medium
- **问题描述与证据**：收到 `IAC SB <opt>` 而对端不发 `IAC SE` 时，`:267` 的 `self.pending_bytes.extend_from_slice(&data[command_start..])` 把整段缓冲（含此前已暂存的字节）重新入栈，本次 `output` 为空。`process_data` 入口 `:193-195` 每轮 `take(pending_bytes)` 并与新数据拼接后从头扫描，于是：① 该 SB 之后的所有终端输出被吞，终端表现为冻结，除断开连接外无法恢复；② `pending_bytes` 随远端发送量**线性无界增长**；③ 每轮全量重扫，累计 CPU 呈**二次增长**。代码中不存在任何上限或复位路径。
  **触发**：恶意或故障的 Telnet 设备 / 注入字节的中间设备发出不终止的子协商。
- **修复方案**：给 `pending_bytes` 设字节上限（超出按协议违规复位并向上层报可诊断的错误），并限制单次 SB 扫描长度。
- **验收标准**：单测喂入不终止的 SB 序列，断言 `pending_bytes` 长度有界、且超限时返回明确协议错误而非静默吞掉后续输出。
- **状态**：已修复（详见第五节修复轮记录）

### R3-08：管道化下载在取消与本地写失败时泄漏在途任务

- **位置**：`src-tauri/crates/rshell-protocol/src/ssh/sftp.rs:599`、`:634-637`
- **严重度**：medium
- **问题描述与证据**：`abort_all` 闭包在 `:589-594` 定义，`:614`、`:623`、`:630` 三处正确调用。但 `:599` 的 `wait_for_run(control).await?` 与 `:637` 的 `write_all(...).map_err(...)?` 两处 `?` 直接返回，**跳过 abort**。`VecDeque<JoinHandle>` 被 drop 不会 abort 任务，于是最多 `DOWNLOAD_CONCURRENCY`（8）个脱离任务继续发 64 KiB SFTP READ、继续持有 `RemoteHandlePool` 与远端文件句柄，与后续传输抢请求槽。这与代码自己在 `:587-588` 声明的不变量（「提前中止时必须停掉在途任务」）直接矛盾。现有测试 `pipelined_copy_propagates_fetch_error_and_aborts_inflight`（:951）只断言错误传播，且其失败走的是 `handle.await` 分支——**那条路径确实调了 `abort_all`**，故两处泄漏均无覆盖。
  **触发**：(a) 取消一个 ≥128 KiB 的下载，派发阶段停在 `:599`；(b) 本地写失败（磁盘满、目标被删、权限变更）于 `:634-637`。
- **修复方案**：把循环体包一层，使所有提前返回都经由 drain + abort 退出（guard 优于逐点补 `abort_all`，避免下次再漏）。
- **验收标准**：单测在 `pending` 非空时分别触发取消与写失败，断言无残留任务（可用 `JoinHandle::is_finished` 或任务计数断言）、池中句柄全部归还。
- **状态**：已修复（详见第五节修复轮记录）

### R3-09：串口 send 阻塞在读任务持有的端口互斥锁上，每次按键最多 100 ms

- **位置**：`src-tauri/crates/rshell-protocol/src/serial/mod.rs:255`（读持锁）对照 `:217-224`（send 等锁）；超时配置 `:169`
- **严重度**：medium
- **问题描述与证据**：读侧 `port.lock().unwrap().read(&mut bytes)` 把 `Arc<Mutex<Box<dyn SerialPort>>>` 守卫**持跨越整个阻塞 `read`**（最长为配置的 100 ms 端口超时），且 `:276` 在 `Err(TimedOut)` 时返回 `Ok(0)`，调用方（session/service.rs:756）立即重启读循环——锁因此近乎 100% 被占。`send` 的 `spawn_blocking` 阻塞在 `port_arc.lock()`（:218），于是每个按键额外延迟 0–100 ms，粘贴突发则完全串行排队。`session/service.rs:709-723` 的 `select!` 在收到 `Send` 时 drop 掉在飞的 `recv` future，但 `spawn_blocking` 任务存活（`&mut JoinHandle` 的 cancel-safety，:259-265），守卫并不释放。
  **违反 CLAUDE.md**：「不要在等待远端输入时阻塞发送路径」。
- **修复方案**：写侧另开一个独立句柄（多数平台支持同时打开），或 `try_lock` + 有界重试，使按键永不排在在途读之后。
- **验收标准**：单测/mock 下断言 `send` 不被读持锁阻塞（构造一个长持有的读，`send` 仍能在有界时间内返回）。
- **状态**：已修复（详见第五节修复轮记录）

### R3-10：路径栏的「授权根目录」检查是纯前缀比较，`..` 可绕过

- **位置**：`src/components/transfer/PathBar.vue:346-354`、`src/components/transfer/FileBrowserPane.vue:224-229`；最终写入 `src/components/transfer/TransferWorkspace.vue:309-310`
- **严重度**：medium
- **问题描述与证据**：两处守卫都只做前缀字符串比较，不折叠 `.` / `..`。选 `/home/user` 为本地根后输入 `/home/user/../etc` 回车：`PathBar.commitEdit` 归一化斜杠后 `normalized.startsWith("\\home\\user\\")` 为真 → 放行；`FileBrowserPane.navigateTo` 的 `path.startsWith("/home/user/")` 同样为真 → 放行。`internalLocalPath` 变为穿越路径，而 `localRoot` 不变（面包屑与 `rootPath` 仍显示 `/home/user`），`capabilities.download` 保持可用，下载落到选定目录之外。
  而**远端**侧由 `validate_remote_mutation_path`（session/service.rs:87-102）严格拒绝 `..`、空段、结尾 `/`、`\`、`\0`——两侧标准不一致。附带症状：Windows 根目录因 `navigateTo` 只判 `/` 而被误挡，两个入口的判断互相矛盾。
  **违反 CLAUDE.md**：「SFTP 写操作只接受绝对、非根、无歧义路径」。
- **修复方案**：解析 `.`/`..` 段后与**归一化**的 `rootPath` 比较，抽成一个 `isWithinRoot(candidate, root)` 共享 helper 供 `commitEdit` 与 `navigateTo` 共用。
- **验收标准**：单测断言 `/home/user/../etc`、`/home/user/./x`（放行）、`C:\root\..\other` 均按预期判定；`TransferWorkspace.spec.ts` 补一条越界导航不改变 `internalLocalPath` 的用例。
- **状态**：已修复（详见第五节修复轮记录）

### R3-11：每个已关闭的终端标签泄漏最多 256 KiB sink 条目

- **位置**：`src-tauri/src/terminal.rs:67-71`（`push` 建条目）、`:128-131`（`detach` 仅按会话）
- **严重度**：low
- **问题描述与证据**：`push` 用 `or_insert_with(TermSink::new)` 为未见过的 `(session_id, terminal_id)` 建条目并累积至多 256 KiB backlog。`TerminalChannels` 只提供按 `session_id` 的 `detach`，其唯一生产调用点是会话删除回调（`lib.rs:108`，R2-15 引入）；没有任何按单个 terminal 移除的路径。`SessionService::close_terminal`（session/service.rs:986-1005）关了远端 pty 并 `destroy_terminal`，但**不触碰 `TerminalChannels`**。因此关闭 N 个附加标签 → 泄漏至多 256 KiB × N，直到整个会话被删除。
  附带：`push` 的文档注释（terminal.rs:65）称未注册 pty「静默丢弃 + warn」，实际既不丢弃也无该 warn（只对回溯溢出 warn）。
- **修复方案**：新增 `detach_terminal(session_id, terminal_id)`，在 `close_terminal` 成功路径调用；顺带修正 `push` 的注释与实现不符。
- **验收标准**：单测断言关闭标签后 `debug_summary()` 不再含该 `(session, terminal)` 键，且其它标签的条目不受影响。
- **状态**：已修复（详见第五节修复轮记录）

### R3-12：`known_hosts` 整体重写会静默擦除/损坏不认识的行

- **位置**：`src-tauri/crates/rshell-core/src/security/host_key_manager.rs:107-143`（重写），`:69-72`（加载跳过）、`:116-125`（保存跳过）、`:140`（`std::fs::write`）
- **严重度**：low
- **问题描述与证据**：加载时跳过 OpenSSH 哈希条目（`|1|…`）、`@revoked`、注释与 legacy `SHA256:` 行；保存时从内存 map 整体重建并 `std::fs::write` 就地截断。于是任何该解析器不建模的行都在下一次 `trust_host_key` / `delete_host_key` 时被删除（仅 `warn!`）。`@revoked <host> <keytype> <base64>` 还会被解析错位——host pattern 位置被当作主机名，导致 `key_type` 变成主机名、`fingerprint` 变成 key type，重写后丢失被吊销的密钥material。此外就地截断写入在磁盘满或中途崩溃时会留下空/半截信任库，使已信任主机静默退回「未知」并重新弹窗。
  影响低（目标是 rshell 自己的 `data_root/known_hosts`，不是 `~/.ssh/known_hosts`，且 rshell 本就不认哈希条目）。
- **修复方案**：只重写本应用拥有的行（按条目改写，保留外部行），并改为临时文件 + rename 落盘。
- **验收标准**：单测：文件含注释/哈希/`@revoked` 行时执行 `trust_host_key`，断言这些行逐字保留、`@revoked` 的 base64 未丢失。
- **状态**：已修复（详见第五节修复轮记录）

### R3-13：永久信任先记入内存再落盘

- **位置**：`src-tauri/crates/rshell-core/src/security/host_key_manager.rs:172-179`（先 insert 后 `save_known_hosts()?`）；`:183-191` 同形（delete）
- **严重度**：low
- **问题描述与证据**：`trust_host_key` 先把条目以 `Trusted` 插入内存 map，再 `save_known_hosts()`。若写入失败（只读卷、权限、磁盘满）而 `DecideHostKey { accept: true, permanent: true }` 正在执行，**握手本身是安全的**——`command_dispatcher.rs:542-557` 在写失败时 resolve 为 `accept: false` 并返回错误，CLAUDE.md「保存成功后才接受握手」成立。残留问题是状态分歧：内存仍为 `Trusted`，`list_hosts` 报告已信任，而下一次无关的 `save_known_hosts` 会把它写盘（无需新的用户决策）；协议层因每次重读文件仍会继续弹窗。delete 失败是对称镜像：记录留在盘上。
- **修复方案**：先落盘再改内存（或在落盘失败时回滚内存条目）。
- **验收标准**：单测注入写失败，断言内存 map 未新增该条目、`list_hosts` 不报告为已信任。
- **状态**：已修复（详见第五节修复轮记录）

### R3-14：会话列表右键菜单在「连接/断开/打开SFTP/打开标签/新开标签」后不关闭

- **位置**：`src/components/SessionList.vue:106-126`（各 handler）、`:228-233`（菜单容器 `@click.stop`）、`:95-103`（唯一的关闭途径）
- **严重度**：low
- **问题描述与证据**：菜单唯一的关闭途径是 `openContextMenu` 注册的 `window` click/contextmenu 监听，但菜单容器带 `@click.stop`，点击菜单项不冒泡到 `window`。`ctxConnect` / `ctxDisconnect` / `ctxOpenSftp` / `ctxOpenTerminal` / `ctxNewTab` 五个 handler 触发动作后均未设 `contextMenu.value.visible = false`（只有 `ctxUpdateCredential` / `ctxDuplicate` / `ctxDelete` 设了）。结果：动作已执行，浮动菜单继续覆盖会话列表；对「打开SFTP / 打开标签」还会盖在刚跳转过去的界面上。`ctxDelete` / `ctxDuplicate` 注册的 window 监听也会滞留到下一次外部点击。
  **违反 CLAUDE.md** 的可见控件要求（表现为控件行为不完整）。
- **修复方案**：用一个统一 wrapper 收口「执行 + 关闭菜单 + 摘除 window 监听」，供全部菜单项调用，不再依赖被 `@click.stop` 阻断的事件冒泡。
- **验收标准**：组件测试：点击任一菜单项后菜单节点从文档中消失。
- **状态**：已修复（详见第五节修复轮记录）

### R3-15：WASM 沙箱无内存上限，三个声明的限流字段是死字段

- **位置**：`src-tauri/crates/rshell-plugin-sdk/src/sandbox.rs:41-50`（字段定义）、`:186-200`（`Config`）、`:259-264`（`Store::new`）
- **严重度**：low
- **问题描述与证据**：`Store::new(&self.engine, ())` 未挂 `StoreLimits` / `ResourceLimiter`，`Config` 只设 `consume_fuel`。全仓 grep `StoreLimits|limiter|max_wasm_stack_bytes|allow_network|allow_filesystem` **只命中定义处**——`max_wasm_stack_bytes`、`allow_network`、`allow_filesystem` 从未被读取，而模块头 `:5-9` 声称内存有界。恶意 guest 可把线性内存增长到其声明上限（wasm32 最高 4 GiB）导致 OOM。
  **当前不可达**：`execute` / `execute_async` 无生产调用方（`loader.rs:133-140` 只编译模块），本地 WASM 模块加载链路尚未真正执行 guest。一旦接通即成为实际风险。
- **修复方案**：通过 `store.limiter(...)` 挂 `StoreLimits`（memory_maximum），或删除三个未用字段与头部的不实声明，二者择一保持一致。
- **验收标准**：单测加载一个声明大内存上限的模块，断言执行被 `StoreLimits` 拒绝；或断言字段已删除且头部不再声称内存有界。
- **状态**：已修复（详见第五节修复轮记录）

### R3-16：监听 `:8080`（空 host）是无法完成操作的死路

- **位置**：`src/components/TunnelPanel.vue:65-68` 对照 `src-tauri/crates/rshell-core/src/security/tunnel_manager.rs:37-51`
- **严重度**：low
- **问题描述与证据**：`parseEndpoint(":8080")` 得到 `host: ""`；前端的 `host !== ""` 守卫使 `nonLoopbackBind` 为 false，既不弹暴露确认也不置 `allow_non_loopback`；而后端 `is_loopback_bind_address("")` 返回 **false**（其自身测试 `tunnel_manager.rs:955` 明确断言），`validate_bind_address` 直接拒绝并提示「Confirm the exposure in the UI and retry (allow_non_loopback)」——而该输入在 UI 上没有这个选项，隧道永远建不出来。属响亮失败而非静默失败，故定级 low。
- **修复方案**：提交前要求 host 非空并给行内错误提示；保留当前保守判定，确保 `""` 永不进入 `TcpListener::bind(":port")`（那会绑定所有网卡）。
- **验收标准**：组件测试：`:8080` 输入得到行内错误、不发起 `create_tunnel`。
- **状态**：已修复（详见第五节修复轮记录）

## 三、经复核未成立

以下 9 条经实读证伪或确认为已修复，**不立条**，记录以免重复排查：

1. **`Handle::block_on` 在 Rhai 宿主回调里 panic** —— `CoreScriptHost::send_text` / `list_sessions` / `execute_quick_command`（command_dispatcher.rs:57-76）调用 `Handle::current().block_on(...)`，而脚本跑在 `spawn_blocking` 线程（:684）。实读 tokio 1.53.1：blocking 池线程经 `rt.enter()` → `try_set_current`，只设置当前 handle 而非 `EnterRuntime::Entered`，故 `enter_runtime` 的 panic 分支不会被触发。**不成立。**
2. **池化 SFTP 句柄跨区间串位导致下载错乱** —— `read_range` 每次读前显式 `seek(Start(offset))`（sftp.rs:117-127），russh-sftp 2.3.0 对 `SeekFrom::Start` 的 `start_seek` 是 ready future，只置 `pos`，位置不跨借用残留；池本身容量有界且有 `acquire → None → 新开` 兜底（sftp.rs:53-63、:107-114）。**不成立。**
3. **`SftpClient::close()` 零调用导致 SFTP 通道泄漏** —— `Channel::into_stream` 把 `Channel` 包进 `ChannelRx`，其 `Drop` 会 `try_send(CHANNEL_CLOSE)`（russh-0.48.2 `src/channels/io/rx.rs:93-102`），drop client 即关闭服务端通道。**不成立。**
4. **错误仍显示 `[object Object]`** —— `client.ts:80-101` 的 `call()` 已把 `IpcErrorShape` 转为 `IpcCallError extends Error`，`String(e)` 得到 `"Error: <message>"`；唯一的裸 `invoke` 路径（`attach_terminal`）用 `ipcErrorMessage`。R2-02 **确已修复**。
5. **PROB-06 主机密钥无条件信任** —— `command_dispatcher.rs:503-520` 已按 `decision` 分派：`TrustPermanent` 才落盘，`TrustOnce` / `Reject` 显式返回错误且不写任何条目。**确已修复。**
6. **PROB-01 风格：snake_case 键名** —— 全部 `#[tauri::command]` 声明 `rename_all = "snake_case"`，且 `tests/unit/ipcContract.spec.ts` 对 45 个 client.ts helper ↔ 62 个已注册命令做双向对账（参数键、事件变体、`CommandOutcome` 返回标注、helper 调用方）。前端 agent 独立复核 45 个 helper 全部命中已注册命令且键名一致，**无契约漂移**。
7. **`remove_transfer` 可移除活跃任务** —— 只允许终态（service.rs:766-780），非终态返回 `InvalidState` 并提示先取消；未知 id 幂等。注释亦解释了控制通道泄漏的顾虑。**已正确处理。**
8. **`18b9dcd` 授予 WebView 全局读写文件系统 scope** —— 有 commit 正文与 docs/08 记录风险边界的**产品决策**（与 Xftp/WinSCP 同类工具一致），非缺陷。该 commit 对 `allow_local_directory` / `authorizeLocalDir` 的回退干净：全仓 grep 零悬挂引用。**不立条。**
9. **Windows PTY 的 stderr 管道不排空（child 约 64 KiB 后阻塞）** —— 机制成立（`pty/windows.rs:28`），但 `UnixPty` / `WindowsPty` / `PtyError` 在自身模块外**无任何调用点**，`Pty` trait 零消费者，路径不可达。**当前不立条**；若接通本地 shell PTY 支持则升级为缺陷。

## 四、暂缓项

本轮无暂缓项。

## 五、修复轮与最终验证

### 基线验证（`4358c85`，修复前）

`npm run typecheck` 退出 0；`npm test` → Test Files 34 passed (34)、Tests 234 passed | 3 skipped (237)；`cargo clippy --workspace --all-targets -- -D warnings` 退出 0；`cargo test --workspace` 退出 0（rshell-core 177、rshell-protocol 54、rshell-infra 18、rshell-api 6、rshell-plugin-sdk 4、rshell 9）。

**这组全绿结果与本清单 16 条并存**，其含义与 docs/10 记录的 PROB-27「系统性假绿」一致：现有测试覆盖的是单函数行为与契约表，而 R3-01/02/03 需要「持锁跨网络 await」「任务生命周期」「对端不响应」这三类时序条件，R3-04/10 需要构造非 UI 来源的路径负载，均落在当前测试形态之外。因此本轮修复**每条都补了对应的回归测试**（R3-03 除外，见其状态行），并在第五节之二记录了实测计数。

### 修复后的最终验证（全量）

| 命令 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 退出 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 退出 0 |
| `cargo test --workspace` | 退出 0 |
| `npm run typecheck` | 退出 0 |
| `npm test` | Test Files 37 passed (37)、Tests 276 passed \| 3 skipped (279) |
| `npm run check:docs` | 通过（14 份现行文档） |

**Rust 用例增量**：rshell-core 177 → **187**（+10）、rshell-protocol 54 → **64**（+10）、rshell 9 → **14**（+5）、rshell-plugin-sdk 4 → **10**（+6）；rshell-infra 18、rshell-api 6 不变。
**前端用例增量**：34 文件 / 234 → **37 文件 / 276**（+42）。新增文件 `tests/unit/TerminalPaneUnmount.spec.ts`、`tests/unit/rootBoundary.spec.ts`、`tests/unit/SessionListContextMenu.spec.ts`。

### 已知未通过项（如实记录，非本轮引入）

`npm run test:scripts`（`node --test scripts/*.test.mjs`）在 Windows 上失败，原因是这些自测以 POSIX 形式调用 `/bin/bash scripts/build.sh`，在 Windows 上被解析成 `C:Usersletmlookcodershellscriptsbuild.sh` 而找不到（`exit 127`）。本轮未改动 `scripts/` 下任何文件（`git status -- scripts/` 为空），该失败为**既有环境问题**，与本清单 16 条无关。

### 修复方式要点

- **R3-01**：pty actor 不再逐个等待 `Close` 应答——`disconnect_ssh` 直接丢弃 sender（actor 的 `None` 分支自行收尾），russh `disconnect` 加 5s 上限；新增 `await_actor_reply` 给 `send_data_to` / `resize_terminal_of` 的应答等待统一加 10s 上限；`SessionService::disconnect` 持写锁跨 `disconnect_ssh` 的外面再包 15s 兜底超时。**根因未变**（actor 仍会卡在 `channel.data()`），但「永久卡死 → 一次可见失败」，用户不再被迫重启应用。
- **R3-02**：`ActiveTunnel` 新增 `cancel: watch::Sender<bool>`，每条已接受的连接订阅一份并在 `select!` 中与中继并行；`close_tunnel` / `suspend_tunnel` / `deactivate_session_tunnels` / `Drop` 四条拆除路径都广播取消。`resume_tunnel` 必须换新通道（否则恢复出来的隧道一出生就处于已取消状态，已有专门用例锁定）。连接计数递减改由 `ConnectionCountGuard` 的 `Drop` 承担——取消分支会跳过任务体末尾的显式递减。
- **R3-03**：见该条状态行，**仅有超时兜底，无自动化测试**。
- **R3-04**：抽出 `is_unsafe_component` 供远端 `sibling_path` 与本地 `resolve_local_target` 共用；本地版另加「解析后父目录必须与原父目录相同」的纵深防御——驱动器相对名（`C:x`）能通过名字检查但父目录会变，正是被这一层拦下。
- **R3-05**：`store.connect` 对同一会话改为**幂等**（复用 in-flight Promise），`openTabSession` 因此可以 `await` 真实连接完成再建面板，而不再靠 `connectionState === "connecting"` 猜测；工具栏「新开标签」在 connecting 态禁用；`TerminalPane` 增加 `disconnected→connected` watcher 补挂一次。
- **R3-06**：`TerminalPane` 引入 `unmounted` 闩锁，在两次 await 之后各检查一次；`ensurePty` / `attachTerminal` 的状态写入也一并挡掉。
- **R3-10**：新增 `src/utils/rootBoundary.ts`，`isWithinRoot` **逐段**比较（不再字符串前缀），`PathBar.commitEdit` 与 `FileBrowserPane.navigateTo` 共用，顺带消除两入口对 Windows 根判断相反的分歧。
- **R3-12**：`known_hosts` 由「整表重写」改为**逐行渲染 + 临时文件 rename 原子替换**：注释、`|1|…` 哈希条目、`@revoked` 等非本应用行逐字节保留；加载阶段也不再把 `@marker` 行错位解析成主机名条目。
- **R3-15**：选用方案 (a)——`wasmtime` 已是直接依赖且 `StoreLimits` 可用，故真正接上 `StoreLimitsBuilder::memory_size`（默认 64 MiB）；`allow_network` / `allow_filesystem` 置 true 时**显式拒绝**而非静默忽略；`max_wasm_stack_bytes` 接入引擎 `Config`。模块头「内存有界」的声明此时才成立。

### 仍然未验证的边界

- R3-03 的超时行为未在运行时验证（需不对 channel-open 响应的 SSH 对端）。
- R3-09 的 `SerialPort::try_clone` 在**真实 macOS / Windows 串口硬件**上的行为未验证（本机无物理串口）；已测的是 fake port 上的加锁契约，`try_lock` 有界回退覆盖克隆失败的情形。
- 外部 SSH 服务、签名与公证仍按 CLAUDE.md 标记为未验证。
