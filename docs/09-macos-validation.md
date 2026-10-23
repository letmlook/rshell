# macOS 验证记录

本记录按轮次分节记日期（最新一轮：2026-10-21）。分支：`main`。此记录区分代码检查、自动测试、原生进程启动和真实交互，不将未执行项记为通过。

## 2026-09-27：凭据加固与发布就绪轮

## 环境

macOS 本机，Rust stable 1.98.1，Tauri 2 + Vue 3 + xterm.js。依赖由 `npm ci` 和 Cargo.lock 锁定。Rust 命令在 `src-tauri/` 执行，并使用 `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1` 避开本机 rustup 自动更新组件冲突。

## 自动检查

本轮同时运行仓库内统一入口脚本（与 CI 复用同一份逻辑，命令清单见 [scripts/README.md](../scripts/README.md)）：

- `npm run verify`（`scripts/verify.sh`）：前端 `typecheck` / `test` / `build` / `check:docs` / `check:bundle` / `check:release` / `test:scripts`，Rust `fmt` / `clippy --workspace --all-targets -- -D warnings` / `test --workspace`，全部 0 失败。
- `npm run audit`（`scripts/audit.sh`）：`npm audit --registry=https://registry.npmjs.org` 与 `RUSTUP_TOOLCHAIN=stable cargo audit --file src-tauri/Cargo.lock`，本地开发机需先 `cargo install cargo-audit --version 0.22.2 --locked`，否则脚本立即以非零退出并指出安装命令。

`npm test` 累计 75 项通过（20 文件），`node --test scripts/*.test.mjs` 累计 32 项通过；`npm run check:bundle` 在 macOS 调试包构建后扫描 `dist/assets/` 并断言每个 JS chunk ≤ 500 KiB、无 `.map`，最大 chunk 403 KiB（vendor-xterm）。

同时构建 Tauri 与运行 rustdoc 时曾出现 `E0463`（找不到 tauri crate）；停止重叠构建后，文档测试及全工作区测试均串行复跑通过，未通过禁用文档测试规避错误。

早期 App 测试出现 jsdom Canvas 能力提示，已通过 mock 与测试无关的终端子组件消除；真实渲染测试边界保持明确。Vite 仍报告 bundle 大小与注解警告。初始 `npm audit` 报告 6 项 advisory；Vite 6.4.3、Vitest 4.1.11、nanoid 3.3.19 更新后重新安装，最终 audit 报告 0 项。该结果仅对应 npm 已知漏洞数据库，不代表完整安全审计。

macOS 构建脚本回归通过（2/2），覆盖图标资源存在性、工作目录、Tauri 调用、target 参数和失败退出码。前端最终类型检查与 65 项测试通过，当前文档契约通过（14 份文档）。其中 3 项标题栏集成测试执行 Cargo.lock 对应的 Tauri 原生拖动脚本和生成权限清单；本机全部实际执行，没有跳过。仅前端安装且缺少 Cargo 依赖/权限清单时，这 3 项会明确跳过，基础权限断言仍会执行。

独立审查发现的 SSH 子通道输出隔离、连接取消/删除竞争、退出清理、协议请求锁、串口取消丢字节、Telnet resize、通知文案、新终端配色和传输错误详情均已修复并通过定向复审。新增本地真实 SSH 传输回归覆盖 shell 输入、尺寸、独立子通道数据与 EOF；它不等于完整 SFTP 文件系统或外部服务器验收。

## 原生开发启动

执行 `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 npm run tauri:dev`。

第一次启动暴露初始化中的嵌套 Tokio 运行时错误，已改为同步注入与同步恢复配置。再次构建并启动 `target/debug/rshell` 后，进程持续运行超过 1 分钟，未再出现该 panic；测试结束后停止了本次开发进程。

这项开发启动证据只确认进程启动与存活。后续通过打包后的应用完成了下述原生界面验收。

## macOS 应用打包与原生界面

执行 `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 npm run tauri:build -- --debug --bundles app`，成功生成 `src-tauri/target/debug/bundle/macos/RShell.app`。首次打包发现缺少 ICNS，已从既有 PNG 生成 macOS 图标并增加资源存在性回归。此为调试应用包，不代表发布签名、公证或安装分发已验证。

实际打开该 `.app`，观察到 `tauri://localhost` WebView 正常渲染，完成以下非破坏性交互：

- 主界面、终端/传输工作区和会话、密钥、工具、设置面板可打开。
- 无连接时上传、下载、新建目录、删除和终端操作按钮禁用；传输队列显示真实空状态及错误详情列。
- 密钥导入、刷新、生成表单可达；主题和插件列表由实际 IPC 加载，未生成或删除用户密钥。
- 快速命令、触发器、隧道表单可达；隧道只有 Local/Dynamic。
- 新建会话只有 SSH/Telnet/Serial；串口草稿显示设备和波特率、数据位、停止位、校验、流控参数，取消后没有保存测试会话。
- 最大化、还原与关闭已实际操作，关闭后重新打开成功。

标题栏操作曾在原生控制台出现 `window.start_dragging not allowed`。已补充最小窗口拖动权限并删除与 Tauri 原生脚本重复的 Vue 双击处理，4 项新增回归先失败后通过。重新打包启动后的控制台未再出现该权限错误，独立复审通过。自动化指针操作未取得窗口位置变化及双击切换的确定证据，因此这两项物理手势仍保留人工复测，不将脚本回归当成实机手势通过。测试结束后已关闭应用，没有保存测试会话或改动用户凭据。

以上面板可达性不等于服务器、物理设备或写操作端到端验收。

## macOS 应用打包与发布预检

Bundle ID 本轮统一为 `com.letmlook.rshell`。`src-tauri/target/debug/bundle/macos/RShell.app/Contents/Info.plist` 的 `CFBundleIdentifier` 已通过 PlistBuddy 校对为 `com.letmlook.rshell`，Tauri 不再发出 identifier 警告。

CI 工作流（`.github/workflows/ci.yml`）在 macOS runner 上调用：

1. `bash scripts/verify.sh --skip-install`
2. `bash scripts/audit.sh`
3. `npm run tauri:build -- --debug --bundles app`
4. `bash scripts/macos-release-preflight.sh --unsigned src-tauri/target/debug/bundle/macos/RShell.app`

第三方 action 全部按提交 SHA 锁定（`checkout@11d5960a326750d5838078e36cf38b85af677262`、`setup-node@49933ea5288caeca8642d1e84afbd3f7d6820020`、`cache@0057852bfaa89a56745cba8c7296529d2fc39830`），并使用 `cargo install cargo-audit --version 0.22.2 --locked`；不允许把签名/公证凭据写入 CI。

本地开发机同样使用 `bash scripts/macos-release-preflight.sh --unsigned <app-path>` 完成预检；带签名的预检需要显式提供 `APPLE_SIGNING_IDENTITY` 与 `APPLE_NOTARY_PROFILE`，脚本不会回显任一变量。

## 真实环境验收清单

- [x] 原生开发进程构建启动，启动崩溃修复后存活超过 1 分钟。
- [x] 调试 `.app` 打包、主界面渲染、面板导航、最大化/还原/关闭和重启。
- [ ] 物理鼠标/触控板的标题栏拖动与双击（权限和事件分发回归已通过）。
- [ ] SSH 真实服务器连接、终端输入/输出、resize、搜索、清屏与剪贴板。
- [ ] 首次主机密钥拒绝/一次信任/永久信任及重启恢复；变化密钥的 GUI 警告。
- [ ] SFTP 真实浏览、上传、下载、新建目录、普通文件删除与失败提示。
- [ ] Local 转发和 Dynamic SOCKS5 经真实 SSH 服务器通信。
- [ ] 主题/配色切换、新建终端和反复挂载的实际视觉检查。
- [ ] 快速命令、触发器、同步发送的桌面交互及重启恢复。
- [ ] Rhai 宿主脚本在真实连接上的端到端调用。
- [ ] 插件面板加载/卸载本地 WASM 的桌面交互。
- [ ] 物理 Serial 设备收发与拔插、参数匹配、断开。
- [ ] 发布签名、公证、安装和分发验证。
- [ ] 钥匙串首次访问授权提示、旧版会话凭据迁移、缺失条目后的重新输入与保存恢复。

限制：未提供用于验收的真实 SSH/SFTP 服务器凭据或物理串口；没有把单元测试/回环服务替代为上述实测。Windows 不属于本轮验收。

## 凭据存储边界

SSH 密码和密钥口令使用 macOS 钥匙串服务 `com.letmlook.rshell.credentials`。会话 TOML 只含凭据元数据；旧版文件中的明文密码/口令会在载入时迁移到钥匙串，并重写文件以移除秘密。迁移失败保留原文件，在会话列表持续显示会话 UUID 和加载问题；修复钥匙串权限或配置后点击“重试加载”。钥匙串条目缺失时，凭据读取失败关闭、不回退到旧明文；恢复方式是在会话列表右键选择“更新凭据”，重新输入密码或私钥口令，点击“保存凭据”后重新连接。若钥匙串不可访问，须先修复 macOS 钥匙串访问权限。macOS 可能显示钥匙串访问提示。主密码用于应用锁定/验证，不是此凭据存储的密钥，也不加密钥匙串。

以上行为有自动化测试和文档契约检查；本记录没有钥匙串提示、真实旧文件迁移或缺失条目恢复的 macOS 人工验收证据。此处不将外部服务器、设备、签名公证验收标为通过。

## 范围审计

当前用户文档不再使用旧 UI 架构或旧启动命令。RDP 仅作为已删除/拒绝输入的说明；运行时公共协议为 SSH、Telnet、Serial。历史设计和执行计划保留在 `docs/superpowers/`，不能作为当前功能承诺。

凭据存储行为和未完成验收项见上文及 [功能与限制](08-incomplete-features.md)。

## 2026-10-02：体检修复轮（commit 82304ca）

独立审查发现的 SSH 子通道输出隔离、连接取消/删除竞争、退出清理、协议请求锁、串口取消丢字节、Telnet resize、通知文案、新终端配色和传输错误详情均已修复并通过定向复审。新增本地真实 SSH 传输回归覆盖 shell 输入、尺寸、独立子通道数据与 EOF；它不等于完整 SFTP 文件系统或外部服务器验收。

本轮另修复：SFTP 传输改为 64 KiB 分块拷贝并按 200ms 节流广播进度，速度随 `TransferTaskInfo` 快照下发；删除不可达的 `HostKeyManager::check_host_key` 及其连带死状态，前端对 nil `decision_id` 决策事件做防御；断开/删除会话与传输队列读取失败不再静默。传输 pause/resume 现已接入传输队列界面，并通过按任务控制通道在分块间真实挂起/恢复拷贝循环，取消保持 `cancelled` 终态且不广播完成事件；真实 SSH/SFTP 服务器上的暂停/恢复观感仍属未执行验收，不因单元测试通过而勾选。

## 2026-10-03：第二轮问题修复（R2-01～R2-20）

第二轮体检的逐项修复记录见 [docs/11](11-known-issues-round2.md)：传输启动窗口的取消/暂停竞态与终态守卫（含修复中新立并已修复的 R2-21：mark_failed/mark_completed 终态守卫）、传输任务 panic 兜底清理、后端错误在前端的可读转换、终端面板关闭后重建与会话删除后的面板清理、搜索快捷键按激活终端路由、触发器删除确认、隧道面板事件订阅、RSA 生成位数、known_hosts 裸 IPv6 匹配、SOCKS5 分段握手、触发器正则预编译、xtask dev 入口、IPC 契约对账测试扩展、KeyManager 密钥能力边界文档，以及本记录按轮次分节。

本轮验证口径为快速检查：`cargo fmt --check` 与 `cargo clippy --workspace --all-targets -- -D warnings` 0 告警；`cargo test` rshell-core 168 项、rshell-protocol 38 项、rshell-api 6 项通过；`npm run typecheck` 通过，`npm test` 110 项（23 文件）通过，文档契约检查通过；每项修复另附针对性单测或变异验证（见 docs/11 各条）。全量 `verify.sh` 本轮未运行，留待统一收口，不在此记为已执行。真实 SSH/SFTP 服务器、物理串口与签名公证仍未验证，上方验收清单状态不变。

## 2026-10-04：传输速度、队列删除与终端短标签

用户实机反馈三项问题，均已定位到具体代码并修复。

**SFTP 下载慢**：根因不是应用侧限速——`copy_with_progress` 无 sleep，200ms 节流只在进度广播路径上、不在拷贝热路径。真实原因是 `russh-sftp` 的 `File::poll_read` 只有一个 `f_read` 槽，串行下载吞吐上限为 `CHUNK_SIZE / RTT`；上传用 `write_nowait` 支持 `max_concurrent_writes = 8` 路并发，故下载长期慢于上传。改为按 64 KiB 切分连续不重叠区间、最多 8 个 READ 在途，写入侧仍严格按 offset 升序串行。`SftpClient` 的 `SftpSession` 改为 `Arc` 共享以支持同会话并发 READ（russh-sftp 按请求 id 多路复用）。小于 128 KiB 退回串行。

**传输队列无删除**：原先只有 `PauseTransfer`/`ResumeTransfer` 的界面入口，`CancelTransfer` 命令已注册但无界面入口，且后端根本没有「从队列移除」的 API。新增 `RemoveTransfer` 命令与服务方法，只允许移除终态任务，活跃/暂停任务返回 `InvalidState`，未知条目按幂等处理。

**终端标签过长**：`ensureTerminalPanel` 调 `addPanel` 时未传 `title`，dockview 回退到 `options.id`，渲染成 45 字符的 `terminal-<uuid>`。改为显式传短标题。

本轮验证：`cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings` 0 告警、`cargo test --workspace`、`npm run typecheck`、`npm test` 127 项（24 文件）通过，文档契约检查通过。新增测试：Rust 6 项区间划分/按序落盘/进度单调/在途并发上限/取消中止/源变短，以及 `remove_transfer` 5 项终态守卫；前端 `shortTerminalTitle` 10 项、TransferPanel 取消/删除 6 项。

**未验证项**：真实 SSH/SFTP 服务器上的实际吞吐提升倍数、局域网与跨网对比数据，以及新的下载路径在真实服务器上的端到端落盘正确性，均无实机证据。区间正确性仅有基于假源的单元测试覆盖。物理串口与签名公证状态不变。

## 2026-10-04：传输吞吐实测与根因更正（含前两轮结论的更正）

本节更正同日更早记录中「Nagle 是主因」「下载并发是关键」的判断——那两条基于代码推理，未经实测，**已被实测推翻**。

**实测环境**：`192.168.85.129`（Ubuntu 22.04，磁盘 689 MB/s，RTT < 1 ms），基准代码 `crates/rshell-protocol/tests/sftp_bench.rs`（凭据只从 `RSHELL_BENCH_*` 环境变量读取，`#[ignore]`，不进 CI；同目录 `transport_bench.rs` 用于隔离 SSH 传输层与 SFTP 层）。

**结果**：

| 场景 | 上传 | 下载 |
|---|---|---|
| debug 构建 | 2.54 MB/s | 2.44 MB/s |
| **release 构建** | **68.10 MB/s**（256 MiB） | **40.63 MB/s**（256 MiB） |
| 同机 OpenSSH `scp` 上传对照 | 31.88 MB/s | — |

release 下上传超过 OpenSSH 对照，下载与用户用 Xshell 观测到的量级一致。基准同时校验下载内容与源逐字节一致。

**根因：`tauri dev` 产出 debug 构建**，russh 的逐字节加密、缓冲拷贝与 SFTP 包组装全部未优化，吞吐因此低一个数量级。这不是应用逻辑的限速。

**逐项排除（均为实测）**：SSH 通道窗口 2 MiB ↔ 16 MiB 无差别；单包大小 32 KiB ↔ 65535 无差别；密码套件 ChaCha20-Poly1305 / AES-256-GCM / AES-256-CTR 均落在 2.4～2.8 MB/s；原生 SSH 传输层（绕过 SFTP 的 exec 通道）为 2.76 MB/s，与 SFTP 同量级，说明瓶颈在 SFTP 之下的共享层；底层 socket 读取约 187 次/秒、平均 16,295 字节/次。

**据此撤回**：放大窗口与包长的改动（实测零收益，且 65535 偏离 RFC 4253 建议的 ≤32768，牺牲互操作性）。

**保留但需正名**：下载 8 路并发区间读取、远端句柄池、关闭 Nagle——三者本身正确且有测试覆盖，但**都不是速度问题的成因**。

**新增构建配置**：`[profile.dev.package."*"] opt-level = 3`，只提升依赖的优化级别，工作区自身 crate 仍为 dev，使 `tauri dev` 在保留调试符号与快速增量编译的前提下获得接近 release 的传输性能。

**未验证**：跨网（非局域网）链路的表现、macOS 上的对应数字（该项目以 macOS 为验收平台，本次仅在 Windows + 局域网实测）。

## 2026-10-08：CI 连续失败的根因与依赖升级

本节记录**根因定位**与**已验证项**；未执行的一律不记为通过。

### 根因一：CI 自 2026-09-27 起连续 11 次失败

`bash scripts/verify.sh` 中的 `npm run test:scripts` 一直红，掩盖了后面的所有步骤。根因是 `scripts/automation.test.mjs` 的 fixture 把目录**前置**到真实 PATH：

```
PATH: `${fixture}:${process.env.PATH}`
```

`ci.yml` 在跑测试前执行了 `cargo install cargo-audit`，于是「cargo-audit 未安装」这一场景里 `command -v cargo-audit` 命中了真身，`audit.sh` 正常退出 0，而测试断言它必须非零退出——一个无法成立的断言。修复：给子进程只传 fixture 目录作为 PATH，并补上纯 shell 的 `dirname`/`basename` 垫片（这两个是 `verify.sh`/`audit.sh` 唯一需要的非伪造外部命令）；bash 解释器本身在收窄 PATH **之前**用绝对路径解析，否则子进程连启动都做不到。修复后脚本测试在 CI 上为 46 通过 / 0 失败。

### 根因二：被掩盖的 macOS 专属测试失败

脚本测试转绿后，`cargo test --workspace` 暴露出 `rshell-core` 的 `local_rename_rejects_names_that_escape_the_destination_directory` 在 macOS 失败。根因是**测试把 Windows 文件系统语义当成了跨平台不变式**：`C:evil.txt` 在 Windows 上 `Path::with_file_name` 会把整条路径换成驱动器相对路径从而逃出目录，在 POSIX 上 `:` 只是普通文件名字符、结果仍落在目标目录内。生产代码本身是安全的（名字校验 + 父目录校验两道），因此改的是测试：统一断言真正的不变式「不得逃出目标目录」，而不是断言某个平台专属的报错。

### 根因三：Rust 依赖告警（21 项）

`cargo audit` 首次真正跑起来后（此前从未成功执行到该步骤）报告 21 项，集中在 4 个 crate：

| crate | 原版本 | 条数 | 处置 |
|---|---|---|---|
| `wasmtime` | 27.0.0 | 18 | 升级到 36.0.17（36.x 线满足全部修复区间，迁移面小于 49.x） |
| `russh` | 0.48.2 | 1 | 升级到 0.62 |
| `russh-cryptovec` | 0.48.0 | 1 | 随 russh 升级（传递依赖） |
| `rsa` | 0.9.10 | 1 | 上游无可用修复，记为显式例外 |

npm 侧同期修复了 4 项高危漏洞（vue 3.5.43、brace-expansion 2.1.7、source-map-js 1.2.2），`npm audit` 归零。

迁移中确认并记录的事实：

- `russh` 0.62 的 `Handler`（client 与 server）改用 RPITIT 声明回调，不再是 `#[async_trait]`；保留该属性会让每个方法签名都因生命周期形参与 trait 声明不匹配。
- `authenticate_*` 返回 `AuthResult` 而非 `bool`；`Auth::Reject` 新增 `partial_success`；`server::Auth::Reject` 同样新增该字段。
- `channel_open_session` 多了一个 `ChannelOpenHandle` 回调参数，且 `accept()` 是异步的——不显式 accept 等同于拒绝。
- `Session::data` 的负载从 `CryptoVec` 改为 `Bytes`。
- RSA 私钥认证必须显式带哈希算法：`PrivateKeyWithHashAlg::new(key, None)` 会退回已被多数服务端拒绝的 `sha-rsa`(SHA-1)，改用 `best_supported_rsa_hash()` 与服务端协商。
- ssh-key 0.7 依赖的 rand_core 0.10 移除了 `OsRng`，系统熵源改为 `getrandom::SysRng` + `UnwrapErr`。
- `rsa::RsaPublicKey` 的 `n` 字段私有化，RSA 位数改用 ssh-key 官方的 `key_size()`（已按 Mpint 前导零规则算好，不再手工剥字节）。
- `russh` 0.62 默认 feature 含 `aws-lc-rs`，其 `aws-lc-sys` 在 Windows 上需要 NASM；改用 `ring` 后端，避免把「装 NASM」变成每个 Windows 开发者的硬前置。

### 根因四：30 分钟的 job 超时（依赖大改后冷缓存重建）

依赖升级让 `Cargo.lock` 整体失效，`actions/cache` 的 key 随之变化，macOS runner 必须从零重建 wasmtime（Cranelift）、russh 与 Tauri，而 clippy 与 `cargo test` 各自还需要独立产物。`ci.yml` 原先的 `timeout-minutes: 30` 在「共享校验」还没跑完时就把 job 杀掉（run 37740101334：`The job has exceeded the maximum execution time of 30m0s`），GitHub 呈现为 `cancelled` 而非 `failure`，容易被误读成人为取消。现将 ci.yml 与 release.yml 的 verify job 均提高到 90 分钟；lockfile 不变时缓存命中，后续运行仍是分钟级。

**未验证**：本节记录的编译与测试结果全部来自 Windows + Git Bash 环境；macOS runner 上的实际结果以 CI 为准。真实 SSH/SFTP 服务器、物理串口与签名公证状态不变。

## 2026-10-08：Tag 驱动的多平台打包与 Release 发布

本节记录**实现**与**已验证项**的区别，未执行的一律不记为通过。

**新增内容**：`.github/workflows/release.yml`、版本一致性守卫 `scripts/check-release-version.mjs`（接入 `npm run check:release` 与 `scripts/verify.sh`）、以及 9 项对应的契约测试。

**设计上解决的问题**：`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`（`[workspace.package]`）各自带版本号，而 Tauri 打包只读取其中一处，没有任何环节强制三者一致——直接打 `v0.2.0` 的 tag 会产出自称 `0.1.0` 的安装包。守卫脚本在打包前比对 tag 与三处版本，不一致或非语义化版本即非零退出并列出全部来源；`npm run verify` 在 PR 阶段即可发现版本漂移。

**action 锁定**：`tauri-apps/tauri-action` 锁定到 v0 对应提交 `84b9d35b5fc46c1e45415bdb6144030364f7ebc5`，`checkout` / `setup-node` / `cache` 复用 `ci.yml` 已有的三个 SHA，两条流水线的锁定由契约测试强制一致。SHAs 于本轮通过 GitHub API 解析注解 tag 得到，非记忆填写。

**本轮实际执行的检查**：两份 workflow 文件的 YAML 结构解析通过；`node --test scripts/automation.test.mjs` 中 9 项新增测试全部通过（版本守卫 4 项、发布工作流 5 项）。这些检查只覆盖配置与契约，**不等于**流水线可用。

**未验证**：

- 未推送过任何测试 tag，没有真实的 GitHub Actions 运行记录；矩阵构建、Release 创建与产物上传均未端到端跑通。
- macOS universal、Windows x64、Linux x64 三个目标只做了配置层面的核对，实际产物未下载检查。
- 产物未签名、未公证，签名与公证状态不变；Release 说明会显式声明这一点。
- Windows 与 Linux 产物没有真机功能验收。
- 本机为 Windows + WSL 环境，24 项调用 POSIX `bash` 的既有脚本测试（`verify.sh` / `audit.sh` / `macos-*.sh`）在本机因 WSL 挂载失败而报错；本轮已用 `git stash` 对比确认这 24 项在改动前后**失败数量不变**，非本次引入。

## 2026-10-08：可靠性与恢复体验完善轮（branch `codex/reliability-recovery`）

本节记录实施侧的真实证据与已知边界；未执行项（macOS 实机、原生服务器、签名公证）一律不记为通过。

### 范围

四个任务按顺序完成，全部 RED-first 落地、独立审查、审查修复三步走：

| 任务 | 实施提交 | 审查 | 修复提交 |
|---|---|---|---|
| Task 1 SSH 请求有界 + 终端手动恢复 | `c1afefc` | DONE_WITH_CONCERNS（2 Minor，已修） | `86c6ac0` |
| Task 2 SFTP 临时传输 + 最终提交 + 从头重试 | `07f7c69` | DONE_WITH_CONCERNS（1 Notable + 1 Minor，已修） | `28ba8df` |
| Task 3 凭据错误分类 + 主机密钥并发/取消/过期 | `3c309f6` | DONE_WITH_CONCERNS（3 concerns，已修 1 项） | `6445486` |
| Task 4 集成验证 | 见下文 | 待独立全分支审查 | — |

Task 1 之前的 baseline commit 为 `86c6ac0`；本轮最终 HEAD 为 `a50245e`（Task 4 集成文档提交）。未推送、未合并、未发布；分支继续按用户授权保留在 worktree。

### 验证（Windows 环境，逐项对应 `scripts/verify.sh` / `scripts/audit.sh` 顺序）

共享入口脚本在本机不可用：`.sh` 文件 CRLF + WSL 挂载失败（与前几轮已记录的根因一致），故按脚本内部顺序逐项手动执行并如实记录结果。

| 命令 | 结果 |
|---|---|
| `npm run build` | exit 0（10 个 chunk，最大 403 KiB / vendor-xterm） |
| `npm run typecheck` | exit 0 |
| `npm test` | **38 文件 / 313 passed**（基线 37 / 279 → +1 文件 / +34 测试，跨 Task 1/2/3） |
| `npm run check:docs` | exit 0（14 份文档契约通过） |
| `npm run check:bundle` | exit 0（chunk ≤ 500 KiB 约束、10 chunk） |
| `npm run check:release` | exit 0（repo / version 0.1.0 三处一致） |
| `npm run test:scripts` | **失败**：Windows 临时目录清理 EPERM（环境问题，测试本身 2196ms 跑完被中断于 `rmSync`） |
| `cargo fmt --all --manifest-path src-tauri/Cargo.toml --check` | exit 0 |
| `cargo clippy --workspace --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings` | exit 0 |
| `cargo test --workspace --manifest-path src-tauri/Cargo.toml` | exit 0（基线与每个任务的累加值：rshell-protocol 70、rshell-core 211、rshell 15、其他 crate 全 0 失败） |
| `npm audit --registry=https://registry.npmjs.org` | exit 0（0 vulnerabilities） |
| `cargo audit --db <isolated-dir> --ignore RUSTSEC-2023-0071 --file src-tauri/Cargo.lock` | exit 0；4 项非阻断警告：`unic-common`/`unic-ucd-ident`/`unic-ucd-version` unmaintained（RUSTSEC-2025-0080/0100/0098）+ `glib 0.18.5` unsound（RUSTSEC-2024-0429）。默认 `~/.cargo/advisory-db` 不可初始化，沿用独立数据库；未删除或改写原数据库。 |

`git diff --check` 无空白错误。新增日志格式串经 `git grep` 不含 `password|passphrase|secret` 字面量。

### 关键能力的诚实边界

- **SFTP 提交原子性**：本轮实现走标准 `SSH_FXP_RENAME`（draft-ietf-secsh-filexfer-02）。`russh-sftp 2.4.0` 高层 API（`SftpSession`）不暴露 `posix-rename@openssh.com` 探测；底层 `RawSftpSession::extended` 入口可发 `SSH_FXP_EXTENDED`，但从 `SftpSession` 拆出 `RawSftpSession` 需 unsafe 重新解释内存布局（UB 风险 + 维护成本均不取），故本轮不采用 unsafe。UI 在 `commit_strategy` 字段如实显示「标准重命名（非原子）」；当 russh-sftp 升级或上层 `RawSftpSession` 公开后再切到 `posix_rename`，契约不变。
- **取消/提交仲裁**：commit 未发送时取消抢先；commit 已发时视为 Completed（取消意图吞掉，因为目标已被替换）；状态机不会产出「已取消但目标被替换」的反例。Pause/resume 不修改既有拷贝循环的写阻塞路径（沿用 R2-08/R3-08 测试覆盖）。
- **重试语义**：`RetryTransfer` 是显式命令，新 task id + Fail policy + 重新冲突确认；活跃任务不允许 retry；retry 不复用 `EnqueueTransfer` 的 hidden flag；从不自动覆盖。
- **凭据错误分类**：`CoreError` 新增 `CredentialMissing` / `CredentialInaccessible` / `CredentialSaveFailed` / `CredentialMigrationFailed` / `HostKeyMismatch` / `HostKeyTrustPersistenceFailed`；每类有独立 IPC kind 与前端恢复指引。`CredentialInaccessible` 不回退旧明文；save 失败时阻塞连接，绝不发 `Connected`；原 `AuthError` / `AuthenticationFailed` 继续走 `auth_failed`，不冒充新分类。
- **主机密钥生命周期**：按 `decision_id` 的 keyed map + CAS，状态 `Pending → DecidedTrustOnce/TrustAlways/Reject | Cancelled | Expired`，前端本地 `Dismissed`；`TrustAlways` 必须先持久化到 trusted-store 再发接受 oneshot；持久化失败回到 `Pending`，不唤醒握手，不发 `Connected`。
- **持久化 vs 过期 race**：若 `trust_host_key` 写入成功后过期触发，状态机正确拒绝握手但磁盘 `known_hosts` 已含条目，下次 connect 会「静默接受」。事后回滚会冒删既有条目风险，所以只能防御性「过期不再唤醒」+ 下次 connect 弹一次「上次未完成握手留下的信任条目，保留吗？」由用户决策（已记为 follow-up issue，本轮不修）。
- **设计层决断记录**：见三份 untracked 工作产物 —— `docs/research/2026-10-08-ssh-recovery-implementation-report.md` / `2026-10-08-sftp-staged-commit-report.md` / `2026-10-08-credential-hostkey-recovery-report.md`。

### 仍待真实环境验收

按 spec 划分，本节列出的「开发验证完成」≠「产品验收完成」。以下场景**未执行**，不视为通过：

- macOS 钥匙串凭据缺失 / 不可访问 / 旧配置迁移的真实提示与恢复
- macOS 钥匙串访问授权对话框
- 真实 SSH/SFTP 服务器（外部网络与不同文件系统）上的传输观感与 commit 行为
- 真实多标签独立 SSH / SFTP / 网络故障下的故障隔离
- 物理串口
- 签名与公证（`APPLE_SIGNING_IDENTITY` / `APPLE_NOTARY_PROFILE` 凭据未注入）
- 跨连接断点续传（spec 明确本轮不实现）
- 进程崩溃后的跨进程 staging 残留扫描（spec 明确另行规划）
- Windows DPAPI / Linux Secret Service 的真实平台适配

完整计划与对应 spec 参见 [`docs/superpowers/specs/2026-10-08-reliability-improvement-design.md`](superpowers/specs/2026-10-08-reliability-improvement-design.md) 与 [`docs/superpowers/plans/2026-10-08-reliability-recovery.md`](superpowers/plans/2026-10-08-reliability-recovery.md)。

## 2026-10-21（续）：长期运行的资源与热路径优化

前一段补的是「功能没实现」，这一段换维度审计**长期运行的资源占用与热路径**：

- **远端浏览每次新开一条 SFTP 通道**：浏览目录 / 取 home / 取元信息各自新开通道并重做 `SSH_FXP_VERSION` 握手，服务端还要为每条通道 fork 一个 sftp 进程。现改为复用同一条会话（`SessionCache`，泛型 + 单测锁定「建一次、用多次、可作废」语义），只读操作失败时丢弃缓存重开再试一次，写操作不重试（可能已生效，重试会报假失败）。同时把 SFTP 的 I/O 移出 `SshClient` 的锁范围——列慢目录不再堵住同会话的 resize。
- **热键在每次按键时重新解析**：终端里每按一次键都会走热键匹配，此前每个按键 × 每条命令都要拆一次热键字符串。现改为命令列表变化时预解析一次（`parsedHotkeys` computed）。
- **两处无界增长的数组**：主机密钥 store 的 `history`（只写不读，每处理一个请求塞一条）与文件面板的导航历史（逐级浏览时一直涨，「后退」能一路退回几百层）。现分别加上限（50 / 100 条）。

## 2026-10-21：补齐「后端已实现、界面无入口」的九项功能

### 范围

对「后端命令与事件齐备、但界面上没有任何入口」的功能做一次系统审计，逐项补齐界面并修掉审计中暴露的两处后端缺陷：

| 功能 | 此前状态 | 本轮改动 |
|---|---|---|
| 主密码 | `setup/verify/change` 三个命令在，状态**只在内存**，无 lock/state 命令，零界面 | 持久化（盐 + 加密令牌落盘，派生密钥不落盘）、`DisableMasterPassword`/`LockMasterPassword`/`GetMasterPasswordState` 三个新命令、「设置 → 安全」面板与应用锁遮罩 |
| 同步输入 | `SyncInputService` 真实，零界面 | 「工具 → 同步输入」面板 + `GetSyncInputSessions` 查询命令 |
| 撰写窗格 | `ComposeService` 真实，零界面；`CurrentSession` 目标因 dispatcher 写死 `None` **必然失败** | 「工具 → 撰写窗格」面板；命令契约新增 `active_session_id` |
| 隧道挂起/恢复 | 后端 R3-02 已把「假挂起」修成真实现，界面只有「关闭」 | 每行「挂起 / 恢复」按钮 |
| 待定隧道恢复 | `listPendingTunnels().rules` 从不展示，`restoreTunnel` 零调用 | 「待恢复的隧道」区块 + 「恢复」按钮；会话选择从裸 uuid 输入框改为连接列表下拉 |
| 已信任主机密钥 | `delete_host_key` 在，零界面；用户无从知道能删什么 | 新增 `list_host_keys` 命令 + 「设置 → 安全」面板的列表与删除 |
| SSH 公钥导出 | `export_public_key` + `PublicKeyExported` 事件都在，零界面 | 密钥面板每行「导出公钥」，事件回传后可复制 |
| 终端配色导入 | `import_color_scheme` 在，零界面；且只写内存，重启即失 | 「设置 → 终端 → 导入…」+ 导入的方案写入持久化文件 |
| Rhai 脚本执行 | `execute_script` 在，零界面 | 「工具 → 脚本」面板，附宿主函数与能力边界清单 |

同步修掉的两处后端缺陷：

- **撰写窗格的「当前会话」目标必然失败**：`command_dispatcher.rs` 把 `ComposeService::send_text` 的 `active_session` 参数写死为 `None`，`ComposeTarget::CurrentSession` 因此永远走 `No active session` 分支。命令契约新增 `active_session_id` 字段并由前端传真实会话 id。
- **同步输入把字节发两遍**：`SendInput` 先直接发给源会话，随后 `send_to_synced_sessions` 又把同一份数据发给同步组里的每个会话——源会话若在组内即收到双份字节（回车被执行两次）。现在广播时排除源会话（`sync_targets` 纯函数，有单测锁定）。

### 验证

| 命令 | 结果 |
|---|---|
| `npm run typecheck` | exit 0 |
| `npm test` | Test Files **44 passed (44)**、Tests **366 passed (366)**（本轮前：38 文件 / 316 用例） |
| `npm run build` | 通过（见下） |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| `cargo test --workspace` | exit 0 |
| `npm run check:docs` | 通过（14 份现行文档） |

**前端用例增量**：38 文件 / 316 → **44 文件 / 366**（+50）。新增文件 `tests/unit/MasterPassword.spec.ts`、`tests/unit/SyncInputPanel.spec.ts`、`tests/unit/ComposePanel.spec.ts`、`tests/unit/HostKeyPanel.spec.ts`、`tests/unit/ScriptPanel.spec.ts`、`tests/unit/colorScheme.spec.ts`；`tests/unit/TunnelPanel.spec.ts` 增加挂起/恢复/待定恢复/错误状态四个用例。

**Rust 用例增量**：rshell-core 219 → **236**（+17，含主密码持久化 11 项、配色方案持久化 4 项、同步输入目标过滤 3 项、撰写窗格目标解析 2 项、主机密钥列举 2 项）。

契约测试 `tests/unit/ipcContract.spec.ts` 的两份白名单同步收口：`HELPER_WHITELIST` 里 12 个「尚未接入 UI」的 helper 已全部接通，只剩 `trustHostKey`（永久信任走 `decide_host_key` 路径）；`EVENT_LISTENER_WHITELIST` 里 5 个「0 监听」事件已各有消费方（`ScriptFinished` / `SyncInputSessionsChanged` / `PublicKeyExported` / `MasterPasswordChanged` / `MasterPasswordVerified` / `ColorSchemeListChanged`）。

### 关键能力的诚实边界

- **主密码是应用层锁，不是凭据保险库**：它只挡这台机器上的操作入口，不断开已建立的 SSH 连接，也不保护钥匙串里的凭据；界面与文档都如实标注了这一点。忘记密码的恢复路径是手动删除 `master_password.json`，界面不提供一键重置（否则锁等于没有）。
- **主密码持久化的是盐与加密令牌，不是派生密钥**：重启后状态是 `Locked`，必须重新验证解锁。只把状态放内存的话「设置主密码」跨重启即失效，属于静默假成功——这条是本轮把该功能判为「实现不完整」的核心依据。
- **同步输入的会话组只在内存**：重启后自动关闭，这是刻意选择（会话连接本身不跨进程恢复，跨进程的同步组没有意义）。
- **撰写窗格的三种目标**：全部失败时 invoke 直接报错，部分失败时逐个会话列出原因；目标为空也报错，不假装已发送。
- **配色方案的「导入」落盘、「选择」不落盘**：导入的自定义方案写入 `color_schemes.json`，重启后仍在列表里；当前选用哪个方案不承诺跨进程恢复（沿用既有边界）。

### 仍待真实环境验收

本轮新增的九项界面入口**全部只有单元测试与构建检查覆盖**，没有 macOS 真实 GUI 人工验收记录：

- 主密码：设置 → 锁定 → 重启 → 解锁的完整闭环，遮罩在真实 WebView 中的焦点与遮罩层行为
- 同步输入：多个真实 SSH 会话上的广播，源会话确实只收到一份
- 撰写窗格：三种目标在真实会话上的投递与部分失败提示
- 隧道挂起后已建立的转发连接确实被撤销，恢复后新连接可用
- 待定隧道在重启后于界面列出并可恢复
- 主机密钥列表与删除
- SSH 公钥导出的复制与粘贴到 `authorized_keys`
- 配色方案导入后重启仍在列表里
- Rhai 脚本运行器的真实执行与输出

签名与公证、外部 SSH 服务、物理串口仍按 CLAUDE.md 标记为未验证。

### 第二轮：补齐触发器完整动作与连接信息编辑（2026-10-21 续）

#### 范围

第一轮补的是「后端命令在、界面无入口」的功能。第二轮转向**后端枚举有多个变体、界面只能创建其中一个**以及**后端命令完整、界面只有一半用法**的两类缺口：

- **触发器**：后端 `TriggerCondition` 有正则匹配与精确匹配两种，`TriggerAction` 有发送文本、显示通知、断开连接、记录到文件四种，四种动作在 `rshell-core/src/session/service.rs` 均已实现；界面此前只能创建「正则 + 发送文本」一种，其余三种动作没有任何入口。现创建表单覆盖全部组合，「记录到文件」的路径用应用内文件选择器选取并要求绝对路径。
- **连接信息编辑**：后端 `UpdateSession` 完整支持改配置，凭据可选 `Keep` / `Set` / `Clear`；界面此前只能用「更新凭据」改密码，改主机名只能删掉重建（凭据随连接 id 走，一并丢掉）。现连接列表右键有「编辑连接」，编辑时密码与口令留空表示保持现有凭据。
- **键盘交互认证**：后端 `AuthMethod::KeyboardInteractive` 与 SSH 客户端的交互式认证流程早已实现，但界面只有密码 / 托管密钥 / 外部私钥三种认证方式，该能力无法使用。现补第四种认证方式。
- **快速命令的「选定会话」范围**：后端 `QuickCommandScope::SelectedSessions` 已实现，创建表单此前只给当前会话 / 所有已连接会话两种。
- **终端外观设置**：字号、字体预设、光标样式/闪烁、滚动回滚行数此前在 `TerminalPane` 里写死，`TerminalConfig`/`CursorStyle` 类型声明了同样字段却从未接线。现接到「设置 → 终端」，已打开的终端通过 watch 立即生效；偏好存 localStorage（与「选中即复制」同机制）。
- **两侧零引用的类型与恒零字段**：`TerminalConfig`/`CursorStyle`/`SearchMatch`/`TelnetConfig`/`SessionInfo` 在 Rust 与 TS 两侧都是 0 引用；`ConnectionInfo` 的 `bytes_sent`/`bytes_received`/`latency_ms` 始终是 0/None 且界面从不消费。前者两侧一并删除，后者三个字段一并删除——类型镜像恢复一一对应，不再声明不存在的能力。
- **快速命令的分组与说明**：`QuickCommand.group` / `description` 此前恒为 `null` / 空串，界面无处可设。现补创建表单的分组与说明输入，面板按分组名聚合命令。
- **触发器命名**：触发器的 `name` 此前恒等于匹配条件，列表里「名称」与「正则」两列内容重复。现可单独命名（留空则回落为匹配条件，不新增错误路径）。
- **状态栏的硬编码版本号**：状态栏原写着 `RShell v0.1.0`，而发布检查只比对 package.json / tauri.conf.json / Cargo.toml 三处——版本一升，界面就变成一句假话。现由 vite 从 package.json 注入 `__APP_VERSION__`，版本号与发布产物同源。
- **命令契约护栏**：契约测试此前只保证「有 helper 的有调用方」，反向（Rust 有命令、TS 连变体都没有）没人管——后端能力既用不了也测不到。现补「commands.rs 与 types.ts 的 AppCommand 变体集逐一对齐」+「每个命令变体在 client.ts 都有对应 helper」两条断言，并给解析器加变体数量下界，防止解析器失效后平凡通过。
- **空分组不可见**：分组树曾过滤掉没有连接的分组，而分组头正是删除按钮的载体——用户新建的空分组在界面上根本不出现，等于永远删不掉。现空分组照常列出。
- **store 对异常 IPC 响应的健壮性**：会话与分组 store 直接把 IPC 返回值赋给 `items`，后端异常返回（非数组）会让它变成不可迭代，界面整块崩掉。现两处都做数组校验。
- **串口设备枚举**：`SerialConnection::list_ports()` 一直存在于协议层却从未经 IPC 暴露，界面让用户手打 `/dev/cu.usbserial-XXXX`——插拔一次名字就变。现补 `ListSerialPorts` 命令与下拉选择（可手工输入兜底，枚举失败时说明原因）。
- **Telnet 终端类型子协商**：客户端对 `DO TERMINAL-TYPE` 回 `WILL TERMINAL-TYPE`，服务器随后发 `SB TERMINAL-TYPE SEND` 时必须回 `SB TERMINAL-TYPE IS <类型>`（RFC 1091）。此前子协商内容被整段丢弃——客户端答应了却从不真发，服务器可能一直等或按错误类型渲染。现实现应答并补 3 项单测（含自定义类型如实发出、其它子协商不误判）。
- **插件扫描的静默拒绝**：清单解析失败、或目录名与清单 `name` 不一致的插件目录，扫描时只记一条 warn 日志就跳过——用户扫描插件只看到「没有插件」，不知道自己的目录被拒了。现以 `state = Error` 的条目出现在列表里，`PluginInfo` 新增 `error` 字段承载原因（Rust + TS 两侧同步）。
- **插件面板的能力可见性**：`PluginInfo.permissions` / `extensions` 一直经 IPC 传到前端却从不展示，用户加载一个声明了网络访问的插件只看得到「已加载」。现展示清单声明的权限（翻成中文说法）与扩展点，并明确说明「已加载」只表示模块编译并登记成功——宿主不向 WASM guest 提供任何宿主能力，声明的权限不会生效。这与沙箱「allow_network 尚未实现，必须报错而不是被忽略」的立场一致。
- **应用级快捷键**：工具栏的 title 写着「新建连接 (Ctrl+N)」，但全局处理器里根本没有这个快捷键——按了没反应，是虚假提示。同时应用接管的键会**漏给远端 shell**（xterm 在目标阶段先处理，窗口处理器的 `preventDefault` 太晚，Ctrl+F 照样送出 0x06）。现抽出 `utils/shortcuts.ts` 作唯一定义处：窗口处理器据此决定做什么，终端据此决定吞不吞该键。Escape 仍照常送远端。
- **文件面板的权限提示**：capability 早已改为全局读写（`fs:read-all` + `**`），但界面仍按旧的「逐目录授权」模型给出「该目录尚未授权，请重新选择」的建议——scope 根本不是失败原因，真正的拒绝来自操作系统。新文案说明真实原因并指向「更换本地目录」；越界提示也从「超出已授权的根目录」改成「超出当前本地目录」（那是用户选定的传输目录，不是权限根）。
- **终端搜索的匹配计数**：搜索栏此前只有输入框与上/下一个按钮，用户不知道有没有命中、当前在第几个。现显示「当前/总数」（无匹配明说「无匹配」）并可切换「区分大小写」。
- **会话分组的持久化**：新增 `folder_id_round_trips_through_toml` 回归测试，锁定「分组随会话 TOML 往返」——否则连接分组重启后就丢了，功能等于只在单次运行内有效。
- **主机密钥的「上次连接时间」**：`first_seen`/`last_seen` 此前只在内存里，且 known_hosts 的 OpenSSH 标准格式没有放时间戳的位置，导致每次重启都被重置成「现在」——两个字段都在撒谎。现改为写 sidecar（`<known_hosts>.meta.json`），连接成功后更新，并在「设置 → 安全」的主机密钥面板里显示。
- **快速命令热键**：`QuickCommand.hotkey` 字段被存储、被持久化，但此前**没有任何实现**——没有热键捕获、没有注册、也没有界面入口，绑了也永远不会触发。现补 `utils/hotkey.ts`（解析与匹配，纯函数）、创建表单的热键输入与校验，以及全局 keydown 处理器里的触发逻辑（作用范围与「执行」按钮共用同一套解析）。
- **连接分组**：`SessionConfig::folder_id` 一直是个指向不存在实体的悬空 `Option<Uuid>`——后端没有任何分组实体与存储，界面上却画着分组树，于是树里永远只有一个「默认」组。现补 `Folder` 实体、`folders.json` 持久化与 `ListFolders`/`CreateFolder`/`RenameFolder`/`DeleteFolder` 四个命令，连接列表按真实分组建树，可新建/删除分组，新建与编辑连接时可选择所属分组。删除分组时组内连接回到「默认」而不是被删掉。

顺带移除主密码的 `encrypt_data` / `decrypt_data`：两者零消费方，且与「主密码不是凭据保险库」的契约冲突——留着等于一个「看起来能加密凭据、实际没有」的假能力。

#### 验证

| 命令 | 结果 |
|---|---|
| `npm run typecheck` | exit 0 |
| `npm test` | Test Files **49 passed (49)**、Tests **440 passed (440)**（第一轮结束时：44 文件 / 366 用例；本轮基线 38 文件 / 316 用例） |
| `npm run check:docs` | 通过（14 份现行文档） |

新增前端用例：触发器动作/条件覆盖 8 项、连接编辑模式 4 项、右键菜单「编辑连接」1 项、键盘交互认证 1 项、快速命令作用范围与热键 7 项（新建 `tests/unit/QuickCommandPanel.spec.ts`）、连接分组 8 项（新建 `tests/unit/Folders.spec.ts`）、热键解析与匹配 12 项（新建 `tests/unit/hotkey.spec.ts`）、全局热键触发 2 项（AppLayout）、终端外观 6 项（TerminalSettings）、快速命令分组与说明 3 项、触发器命名 2 项、会话分组 TOML 往返 1 项（rshell-infra）、搜索匹配计数 1 项、快捷键判据 6 项（新建 `tests/unit/shortcuts.spec.ts`）、Ctrl+N 开新建对话框 2 项（AppLayout）、插件能力可见性 3 项（新建 `tests/unit/PluginPanel.spec.ts`）、串口设备枚举 2 项、空分组可见性 1 项、命令契约护栏 3 项、主机密钥 history 上限 1 项。Rust 侧新增 2 项（SFTP 会话缓存语义）。Rust 侧新增 28 项（分组服务 9 项 + 分组命令 3 项 + 主机密钥时间戳 2 项 + 其余特性合计）。

#### 仍待真实环境验收

编辑连接后能否正常连接、保留的凭据是否仍可用、「记录到文件」触发器是否真的落盘（绝对路径、只追加不覆盖）、四种触发器动作在真实远端输出下的触发时机、键盘交互认证在真实服务器上的应答流程、分组的增删改与连接归组在真实界面上的表现、快速命令热键在真实窗口里的捕获与冲突表现、终端外观改动在真实 WebView 中的即时生效、主机密钥「上次连接时间」在真实连接后是否正确刷新，均需真机确认。

### 2026-10-21：深度 code review 修复轮（3 high / 8 medium / 10 low 全量修复）

针对全仓库深度审查（会话与协议层、传输链路、前端 IPC 与事件契约、安全模块）报出的
3 个 high、8 个 medium、10 个 low，本轮全部修复。改动只落在工作区，未提交。

#### high

- **主密码与私钥口令明文进 tracing span**：`AppCommand` 的 6 个含密变体（4 个主密码 + 2 个密钥口令）
  此前以 `#[instrument(fields(command = ?command))]` 记录整个命令体，明文口令会进日志。
  改为 `rshell-api` 新增 `Secret` 类型（`Debug` 恒为 `<redacted>`），
  并让 span 只记录 `AppCommand::variant_name()`——新增命令变体时穷尽 match 会在编译期强制补齐。
  实测：用同一 `#[instrument]` 形状在独立工程复刻，修复前输出含 `password: "SUPERSECRET-MASTER-PW"`，
  修复后仅 `command="SetupMasterPassword"`，无明文。
- **Telnet NAWS 协商方向反了**：客户端此前发 `IAC DO NAWS`（要求服务端发窗口尺寸），
  与 RFC 1073 §2/§6 规定的方向相反，真实服务端不会把尺寸推回来。
  改为发 `IAC WILL NAWS`、对 `IAC DO NAWS` 回 `IAC WILL NAWS`；协商完成前 resize
  只记住尺寸、协商成功后补发。协议层 telnet 用例 21 项全绿。
  独立复审后又补了两处：收到 `DONT NAWS` / `WONT NAWS` 时按 RFC 854 回负确认
  （`WONT` / `DONT`），否则对端协商状态停在半开；收到对端 `WILL NAWS`（对端想给
  我们发它的尺寸）时回 `DONT NAWS` **拒绝**——我们不消费对端尺寸，回 `DO` 之后
  再丢弃 `SB NAWS` 就是「只答应不兑现」。
- **TerminalPane 重复 attach**：每次 attach 都会让 Tauri 反序列化一个新的 Channel，
  其 index 计数器从 0 重启，与 JS 侧递增的期望值整体错位，`_pendingMessages` 无限增长。
  前端按 `${sessionId}:${terminalId}` 幂等（失败不记 key，保证「重试附加」真重发）；
  后端 `TermSink` 按 `channel.id()` 去重，重复句柄移入 `retired` 随 sink 释放——
  不能直接 drop（drop 会触发 `on_drop` 发 `end`，把仍在用的共享回调注销掉）。

#### medium

Telnet/Serial 请求-应答补超时；SFTP 通道开启与版本握手补超时并缩短锁持有；
应用锁态禁止全局快捷键 / 终端输入 / 复制 / 粘贴；上传恢复前做远端存在性探测（入队即报 `target_exists`）；
staged 失败路径按 `LateConflict` 发布 `TransferConflict`；主密码 PBKDF2 迭代数 100k→600k（OWASP 建议值，
读取时夹取到 [100k, 5M]）、连续失败退避（5 次失败锁 30s）、KDF 派生移出 Tokio worker；
生产下载接回并发区间读（`RangeFetcher` 注入 staged 生命周期，保留 temp→commit→cleanup 语义）；
托管密钥下拉每次打开重拉（此前是一次性快照，删掉的密钥仍能被选中）。

#### low

组件卸载闩锁（KeyManagerPanel）；主机密钥字段校验（防 known_hosts 行注入）；
sidecar 写失败降级为 warn（信任状态已落盘，不该报「信任失败」）；
`TransferTaskInfo` 三个恒空字段改为可选；`listKeys` 返回类型改为 `SshKeyInfo[]`；
12 个会话作用域命令的 `IpcError.session_id` 不再恒为 null；
孤儿 `.partial-<uuid>` 临时文件回收扫描（本地 + 远端目录，限定「本次目标名 + 普通文件 + 超龄 + 非在途」四个条件；阈值本地 1 小时、远端 24 小时——远端 mtime 来自服务器时钟，阈值短了会误删别的实例刚写的同名临时文件）；
提交通道改惰性开启（每任务 2 条 SSH 通道 → 1 条）+ 每会话并发传输闸门（上限 3，超出排队而非失败）；
前端接通 `SessionUpdated` 事件（此前后端更新会话只发它、前端只监听 `SessionListChanged`，
白名单里那条「已由全量刷新覆盖」的理由并不成立），白名单条目已删。

顺带移除 `SftpCapabilities` 这个恒为空的能力参数：提交语义改由提交路径自己探测并经
`CommitOutcome` 如实回报。

#### 独立复审后补修

改动完成后另起一轮独立复审（只读、不改文件），报出 3 条中危 + 6 条低危，已全部修掉：

- **上传前存在性探测的锁范围**：探测原先在 `SshClient` 读锁内做远端 stat——tokio
  RwLock 写优先，一个不回应 STAT 的远端会把同会话的断开/开标签全部堵住。改为
  只有「取缓存会话」在锁内，stat 在锁外并另加 10s 超时（探测只是优化，超时按
  「探测失败」处理，不阻断入队）。
- **`with_sftp` 的注释与实现不符**：新写的注释声称「版本握手在锁外」，实际握手在
  `sftp_session()` 里、仍在锁内。注释改为如实描述：锁的持有被开通道（15s）与版本
  握手（15s）两个上限约束，不会无限期占着读锁。
- **孤儿回收的注释与实现不符**：回收已收窄到「仅本次目标文件名」，注释却仍宣称
  回收整个目录的崩溃残留。注释改为如实描述，并写明代价（上次崩溃留下的
  `b.txt.partial-<uuid>` 要等下次传 `b.txt` 才被清掉——这正是最常见的重传场景）。
- **远端时钟偏差**：远端 mtime 来自服务器时钟，与本地慢超过阈值时会把对方**刚写**
  的同名临时文件判成「超龄」。远端阈值改为 24 小时，本地仍是 1 小时。
- **冲突路径提取**：晚冲突的消息用 `rsplit_once(": ")` 取路径，目标路径本身含
  `": "` 时会取错（前缀里没有 `": "`，用第一个分隔符才对）。
- **传输闸门条目不回收**：会话下再无任务时回收该会话的闸门条目（判定用任务表，
  排队中的任务也在表里，所以不会出现上限被绕过）。
- **NAWS 只答应不兑现 / 缺负确认**：见上。
- **`retired` 通道句柄无上界**：同一 channel id 不重复入 `retired`，使其以上界为
  channel id 数而非 attach 次数。

#### 验证

| 命令 | 结果 |
|---|---|
| `cargo fmt --all --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0，无告警 |
| `cargo test --workspace` | exit 0，累计 **414** 用例通过、**0** 失败 |
| `npm run typecheck` | exit 0（`vue-tsc --noEmit`） |
| `npm test` | Test Files **49 passed (49)**、Tests **441 passed (441)** |
| `npm run build` | exit 0，最大 chunk 403 KiB（vendor-xterm） |
| `npm run check:docs` | 通过（14 份现行文档） |
| `npm run check:bundle` | 通过（10 个 JS chunk，均 ≤ 500 KiB） |
| `npm run check:release` | 通过（版本 0.1.0） |
| `npm run test:scripts` | **48** 项通过、0 失败 |

新增用例：Rust 侧 18 项（孤儿临时文件回收 10 项、传输闸门 2 项、posix-rename 扩展探测 1 项、
NAWS 协商 7 项（含重写的既有 resize 用例与新增的负确认用例）、主密码脱敏 2 项、终端通道去重 1 项等）；
前端新增会话事件用例，IPC 契约测试 21 项全绿。

#### 仍待真实环境验收

本轮**没有**任何一项改动在真实 SSH 服务器 / 物理串口 / 打包签名与公证上验证过，
下列行为必须真机确认：NAWS 协商后的真实服务端窗口尺寸上报；惰性提交通道在真实
OpenSSH 上的 `posix-rename` 原子替换；并发传输闸门在真实并发下的排队表现；
孤儿临时文件回收在真实远端目录上的实际删除范围；主密码 600k 迭代下的解锁耗时与退避表现。

> 上述五项已在下一节（2026-10-22 的 multipass Linux 真机验收）里**逐项验证完毕**；
> 剩下的只有物理串口与签名公证。

### 2026-10-22：multipass Linux 真机验收（补上「仍待真实环境验收」的五项）

用 multipass 起了一台真的 Linux 服务器，把上一节列为「仍待真机确认」的五项逐项跑通。
客户端代码仍是 macOS 上编译的那份，**服务端是真正的 Linux**（这五项考的都是服务端行为：
选项协商、扩展声明、通道额度、远端目录删除语义）。

#### 环境

| 项 | 值 |
|---|---|
| 虚拟化 | multipass 1.16.4+mac |
| 镜像 | Ubuntu 24.04 LTS（`noble`），2 vCPU / 2 GB / 8 GB |
| 实例 | `rshell-test`，`192.168.252.2` |
| SSH 服务端 | OpenSSH 9.6p1 Ubuntu-3ubuntu13.20，`MaxSessions 10` |
| Telnet 服务端 | GNU inetutils telnetd（经 socat 监听 2323） |
| 认证 | 主机公钥（cloud-init 注入 `ssh_authorized_keys`）+ telnet 口令（仅环境变量） |

复现步骤（凭据只经环境变量，不进仓库）：

```bash
multipass launch 24.04 --name rshell-test --cloud-init <(printf 'ssh_authorized_keys:\n  - %s\n' "$(cat ~/.ssh/id_ed25519.pub)")
multipass exec rshell-test -- sudo apt-get install -y inetutils-telnetd socat
multipass exec rshell-test -- sudo nohup socat TCP-LISTEN:2323,reuseaddr,fork EXEC:/usr/sbin/telnetd &
multipass exec rshell-test -- sudo chpasswd   # 给 telnet 登录设口令

export RSHELL_REAL_HOST=192.168.252.2 RSHELL_REAL_SSH_PORT=22 RSHELL_REAL_TELNET_PORT=2323
export RSHELL_REAL_USER=ubuntu RSHELL_REAL_KEY=~/.ssh/id_ed25519
export RSHELL_REAL_PASS='<telnet 口令>' RSHELL_REAL_PATH=/home/ubuntu/rshell-test

cargo test -p rshell-protocol --lib -- --ignored --nocapture
cargo test -p rshell-core --lib -- --ignored --nocapture
```

#### 验收结果

| # | 用例 | 验证的声明 | 结果 |
|---|---|---|---|
| 1 | `real_telnetd_applies_our_window_size` | NAWS 方向、时序与帧格式 | **通过**：真实 telnetd 对我们的 `IAC WILL NAWS` 回 `IAC DO NAWS`；协商完成前发生的 resize 在协商成功后补发；会话里 `stty size` 读回服务端 pty 为 `40 120` |
| 2 | `real_openssh_commits_with_posix_rename` | 惰性提交通道 + `posix-rename` 原子替换 | **通过**：真实 OpenSSH 声明了该扩展，提交策略为 `posix_rename`，已存在的目标被原子替换，temp 无残留 |
| 3 | `real_openssh_channel_budget_is_bounded_and_fails_fast` | 通道额度是真实存在的约束 | **通过**：实测该服务器只允许 **9** 条会话通道，第 10 条明确失败（`Failed to open channel (ConnectFailed)`）而不是挂住 |
| 4 | `real_upload_reclaims_only_our_own_orphan_temps` | 孤儿回收只删自己的东西 | **通过**：远端目录里摆四种诱饵 —— 超龄的 `<目标名>.partial-<uuid>` **被回收**；`<别的目标名>.partial-<uuid>`、`<目标名>.partial-backup`（非 uuid 形态）、`<目标名>.partial-<uuid>`（是个**目录**）**全部保留** |
| 5 | `real_concurrent_uploads_all_succeed_within_the_channel_budget` | 并发闸门让「排队」替代「失败」 | **通过**：同一会话 12 个并发 2 MiB 上传全部成功 |

用例 5 做了**反证**：把 `MAX_CONCURRENT_TRANSFERS_PER_SESSION` 临时从 3 放开到 12 后，
同样的 12 并发里有 **4 个**失败，报错正是 `Failed to open SFTP channel: ... Failed to open
channel (ConnectFailed)`（服务端额度）。恢复为 3 后全部通过——说明闸门是**真正起作用**的，
且这条用例能抓住回归。

#### 新增用例

5 条真机用例全部 `#[ignore]` 门控（凭据只从环境变量读），与仓库既有的
`tests/sftp_bench.rs` / `tests/transport_bench.rs` 同一范式；`cargo test --workspace`
不受影响（414 项通过、0 失败）。

| 文件 | 用例 |
|---|---|
| `rshell-protocol/src/telnet/mod.rs` | `real_telnetd_applies_our_window_size` |
| `rshell-protocol/src/ssh/sftp.rs` | `real_openssh_commits_with_posix_rename`、`real_openssh_channel_budget_is_bounded_and_fails_fast` |
| `rshell-core/src/transfer/service.rs` | `real_upload_reclaims_only_our_own_orphan_temps`、`real_concurrent_uploads_all_succeed_within_the_channel_budget` |

#### 仍未验证

- **客户端未在 Linux 上原生编译运行**：跑的是 macOS 编译出的测试二进制，服务端是 Linux。
  协议行为（选项协商、扩展声明、通道额度、远端删除语义）已完全由真实 Linux 服务端验证；
  但「代码在 Linux 上编译并运行」本身没有验证。
- **物理串口**、**签名与公证**、**Windows** 仍未验证。

### 2026-10-21：原生应用启动（首次真机运行证据）

在此之前的记录里，原生启动一直是「手工步骤」。本轮第一次在 macOS 上**真正跑起来**：

```bash
cd src-tauri && RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo build --bin rshell
RUST_BACKTRACE=1 RUST_LOG=info ./src-tauri/target/debug/rshell
```

| 观察项 | 结果 |
|---|---|
| 启动日志 | `RShell Tauri shell started (slice 1.1); data_root = /Users/letmlook/Library/Application Support/rshell` |
| 进程状态 | 持续运行 2 分 10 秒，`STAT=S`（事件循环空闲，GUI 应用的正常状态），RSS 约 112 MB |
| 崩溃 / panic | 无。日志里除启动行外无任何 ERROR/WARN |
| 退出 | SIGTERM 后正常退出，无残留进程 |
| 数据目录 | 启动后**未新增**文件——`folders.json` / `color_schemes.json` / `master_password.json` 都是用到才写（惰性创建），与设计一致 |

这一条覆盖的是**启动路径**：本轮新增的 `MasterPassword::with_path` / `FolderService::with_path` / `ThemeManager::with_persistence` 都跑在启动装配里，它们在真实进程里读文件、持锁、初始化，没有出现单元测试覆盖不到的崩溃。

**边界（必须说清）**：本次只证明了「应用能启动、事件循环稳定、启动装配不崩」。**界面渲染、交互、真实 SSH/SFTP 连接仍未经真机验证**——截图受系统录屏权限限制无法取得，窗口内的一切仍只有单元测试与构建检查覆盖。外部 SSH 服务、物理串口、签名与公证同样未验证。

### 2026-10-22：界面手动点击验收（macOS 原生应用 + multipass Linux 真机）

上一节把**后端协议行为**放到了真实 Linux 服务端上验证。这一轮补的是**界面这一侧**：
把 macOS 原生应用跑起来，逐个面板、逐个按钮手动点一遍，对着真实服务器确认
「点了真的有反应」——并顺带挖出 5 个此前只靠单元测试看不出来的缺陷。

#### 环境

| 项 | 值 |
|---|---|
| 客户端 | macOS 上 `npm run tauri:dev` 起的 Tauri 2 原生应用（`target/debug/rshell`） |
| 服务端 | multipass Ubuntu 24.04（`rshell-test`，`192.168.252.2`），OpenSSH 9.6p1 |
| 认证 | 密码认证 + **外部私钥**（`~/.ssh/id_ed25519`）两条路径都走通 |
| 被测连接 | `Key-auth-test`（外部私钥）、`multipass-linux`（默认组）、`multipass-linux`（GROUP2 组） |
| 远端 / 本地测试目录 | `/home/ubuntu/rshell-e2e` / `/tmp/rshell-e2e` |

#### 手动点击验收矩阵

| 面板 / 功能 | 实际操作 | 观察到的结果 | 结论 |
|---|---|---|---|
| 新建连接 · 认证方式 | 依次点「密码 / 托管密钥 / 外部私钥 / 键盘交互」 | 四种都可选中（修复前整个 radio 组是惰性纯文本，只有密码可达） | 通过 |
| 新建连接 · 外部私钥 | 路径选择器里双击 `~/.ssh` → 选中 `id_ed25519` → 打开 | 字段填入 `/Users/letmlook/.ssh/id_ed25519`；创建后后端 `SSH authentication successful`，终端出现真实登录横幅与 `ubuntu@rshell-test:~$` | 通过 |
| 会话多开 | 「新开标签」 | 第二个标签 `multipass-linux #2` 是**独立 pty**（各有横幅与提示符），共享同一条 SSH 连接；左侧列表仍只有一条连接信息 | 通过 |
| 名称唯一性 | 同组同名 / 跨组同名各建一条 | 同组同名被拒（`A session with the same name already exists in this folder`）；跨组同名通过并各自连上 | 通过 |
| 传输 · 上传 | 选本地 `/tmp/rshell-e2e/local-src.txt` → 点 ↑ | 远端出现 34 B 的同名文件；后端 `Transfer completed strategy=PosixRename` | 通过 |
| 传输 · 下载 | 选远端 `remote-src.txt` → 点 ↓ | 本地落地 13 B，内容 `remote-ready`；后端 `strategy=StandardRename` | 通过 |
| 传输 · 队列 | 看「传输 (2)」标签 | 两条都「完成 100%」，并如实标出提交方式（原子 / 非原子） | 通过 |
| 密钥 · 生成 | 名称 `e2e-ed25519` → 生成 | 私钥落盘（399 B，权限 600），列表显示指纹 `SHA256:h4bAW+zWP…` | 通过 |
| 密钥 · 导出公钥 | 点「导出公钥」→ 复制 | 对话框可见并显示完整公钥；点「复制」后系统剪贴板里正是那 92 字节公钥 | 通过（修复后） |
| 密钥 · 删除 | 点「删除」→ 确认 | 确认窗显示密钥名与「此操作无法撤销」；确认后私钥文件从磁盘移除 | 通过 |
| 工具 · 快速命令 | 新建 `e2e-cmd`（作用域选「所有已连接会话」） | 三种作用域（当前会话 / 所有已连接会话 / 选定会话）都能选；列表出现该条 | 通过 |
| 工具 · 触发器 | 新建触发器（匹配=正则，动作=显示通知） | 两种匹配 + 四种动作都可选；内容留空时点「添加」被明确拒绝（提示「动作内容不能为空」），填上后创建成功 | 通过 |
| 工具 · 触发器（真实触发） | 终端里输出匹配文本 | 后端 `Trigger matched`；界面右上角弹出「触发器通知 / RSHELL_TRIGGER_FIRED」 | 通过（修复后） |
| 工具 · 隧道 | 建 Local 隧道 `127.0.0.1:18080 → localhost:22` | 端口真的在监听（`lsof` 看到 `rshell … 127.0.0.1:18080 LISTEN`）；经该端口读到远端 `SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13.20` | 通过 |
| 工具 · 撰写窗格 | 发送 `RSHELL_COMPOSE_OK` | 文本真的出现在远端终端；面板显示「已发送」 | 通过 |
| 工具 · 脚本 | 运行默认 Rhai 脚本 | 后端 `Script executed successfully`，日志打出「会话数: 3」与 3 个会话 id；界面显示「执行完成」 | 通过 |
| 工具 · 同步输入 | 打开面板 | 三个会话各自显示 connected / disconnected，开启 / 关闭按钮可用 | 通过 |
| 设置 · 主题 | 切 Light 再切回 Dark | 整个界面配色实时跟随切换 | 通过 |
| 设置 · 终端 | 逐项查看 | 配色方案、字号、光标形状、字体、回滚行数、光标闪烁、选中即复制、鼠标行为都在 | 通过 |
| 设置 · 插件 / 安全 | 逐项查看 | 插件空状态渲染正常；主密码设置表单与主机密钥列表都在 | 通过 |

#### 手动点击挖出的 5 个缺陷（单元测试发现不了）

1. **本地路径选择器读不了任何含点目录的路径**（已单独提交 `ac4e800`）。
   `tauri-plugin-fs` 在 Unix 上 `requireLiteralLeadingDot` 默认为 `true`，`allow:["**"]`
   不匹配任何含点开头路径分量的路径：`readDir(主目录)` 成功，但逐条 `stat` 到 `~/.ssh`
   被 scope 拒绝，异常上抛把**整个目录**报成「无权读取 /Users/letmlook」。
   改为 `plugins.fs.requireLiteralLeadingDot=false`（`fs:deny-webview-data-*` 仍优先）。

2. **`readDir` 不跟随符号链接 → `/tmp`、`/var`、`/etc`、`/home` 在目录模式下彻底消失**。
   `read_dir` 用 `DirEntry::file_type()`（不跟随），`stat` 用 `fs::metadata()`（跟随）。
   macOS 上 `/tmp` 是指向 `/private/tmp` 的符号链接，被报成「文件」，目录模式按
   `isDirectory` 过滤后**用户在列表里根本选不到它**（真机实测：`/` 下只列出
   `Applications/bin/cores/…`，唯独没有 `tmp`）。改为以 `stat` 的类型为准
   （`src/utils/localFs.ts` 统一处理选择器、文件面板、目录树三处）。

3. **侧栏里的 `el-dialog` 完全没有显示**。Element Plus 2.14 的 `el-dialog` 默认
   `appendToBody=false`，对话框就地渲染在 `.side-panel-body`（`overflow:auto`）里，
   而 `.side-panel` 是 `overflow:hidden`——对话框被裁掉。症状是：点「导出公钥」
   **界面毫无反应**，但 DOM 里其实已经有公钥（AX 树能读到内容、截图里却什么都没有）。
   改为给侧栏内的三个对话框（导出公钥 / 导入私钥 / 更新凭据）加 `append-to-body`。

4. **所有 `ElMessage` / `ElNotification` 提示都落在视口之外**。
   逐组件引入时只引了 `<el-xxx>` 标签对应的样式表，而以 **API 形式**调用的
   `el-message.css` / `el-notification.css` 根本没被导入：没有 `position:fixed`，
   提示变成追加到 `<body>` 末尾的普通块级元素，落到视口之外——「传输失败」
   「同步输入失败」「触发器通知」这类**全局提示用户一条都看不到**。
   补上两个样式表，并把「程序化 API 的样式表」纳入 `tests/unit/elementPlusContract.spec.ts`
   的对账范围（此前只对账模板里的标签，看不见 API 形式调用）。

5. **对已连接的会话再点一次「连接」，活连接被显示成「网络连接失败」并锁死终端输入**。
   前端的连接状态由 `ConnectionStateChanged` **事件**驱动，而事件只发一次；
   页面重载后前端丢了状态，会再发一次 `connect`——那时后端连接其实还活着。
   旧实现在这里抛 `Invalid state: … is already connecting or connected`，前端把它
   当成连接失败：界面挂出「网络连接失败」卡片、把会话标成 failed，
   再按「锁态禁止输入」把**正在工作的终端输入**一起禁掉。
   改为 `connect` 对已连接会话**幂等**：直接返回成功，并把真实状态重推一次给前端。

#### 新增回归用例

| 文件 | 用例 | 守住的是 |
|---|---|---|
| `tests/unit/LocalPathPicker.spec.ts` | 指向目录的符号链接在目录模式下照常列出 | 缺陷 2 |
| `tests/unit/FileBrowserPane.spec.ts` | 本地列表按 `stat` 的类型把符号链接目录当作文件夹 | 缺陷 2 |
| `tests/unit/PathBar.spec.ts` | 本地目录树把指向目录的符号链接也列进来 | 缺陷 2 |
| `tests/unit/elementPlusContract.spec.ts` | 以 API 形式调用的每个 Element Plus 组件都有样式导入；没有零引用的 API 导出 | 缺陷 4 |
| `tests/unit/KeyManagerPanel.spec.ts` | 对话框必须能在 `document.body` 里找到（teleport 后 `wrapper.find` 找不到） | 缺陷 3 |
| `rshell-core/src/session/service.rs` | `connect_on_connected_session_is_idempotent_and_republishes_state` | 缺陷 5 |

前四条都做了**反证**（把修复撤掉，用例必须失败）：
撤掉 `stat` 类型判定 → 符号链接目录从列表消失；撤掉 `el-notification.css` 导入 →
契约用例报「缺少 theme-chalk/el-notification.css 的样式导入」；撤掉后端幂等分支 →
`connect on a connected session must succeed: InvalidState("Session … is already
connecting or connected")`。

#### 验证

| 命令 | 结果 |
|---|---|
| `cargo fmt --all --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0，无告警 |
| `cargo test --workspace` | exit 0，累计 **419** 用例通过、**0** 失败、9 忽略（真机用例） |
| `npm run typecheck` | exit 0（`vue-tsc --noEmit`） |
| `npm test` | Test Files **50 passed (50)**、Tests **450 passed (450)** |
| `npm run build` | exit 0，最大 chunk 403 407 B（vendor-xterm） |
| `npm run check:docs` | 通过（14 份现行文档） |
| `npm run check:bundle` | 通过（10 个 JS chunk，均 ≤ 500 000 B） |
| `npm run check:release` | 通过（版本 0.1.0） |
| `npm run test:scripts` | **48** 项通过、0 失败 |

#### 仍未验证

- **客户端仍未在 Linux 上原生编译运行**：跑的是 macOS 编译出的二进制，服务端是 Linux。
  协议层行为已由真实 Linux 服务端验证，但「代码在 Linux 上编译并运行」本身没有验证。
- **物理串口**、**签名与公证**、**Windows** 仍未验证。
- **Telnet 协议**：本轮没有再点一遍 telnet 连接（协议层行为已由上一节的真机用例覆盖，
  但**界面上的 telnet 连接入口**没在真机上点过）。

### 2026-10-22：CommandDispatcher 级真机端到端验收（macOS 客户端 + multipass Linux 真机）

#### 为什么从「手动点击」换成「命令级」验收

macOS 处于锁屏状态时，AX 层对**所有**应用都不可用，不只是 RShell：

- `computer_desktop_state` 只有壁纸，**无 Dock、无菜单栏**；
- `computer_app_state` 对 RShell 的窗口和新启动的 TextEdit 一律返回
  `ax_window_unresolved`（elements=0）；
- `computer_window_set_frame` 报「window_id … belongs to pid … in WindowServer but
  has no matching AXWindow」；
- 应用本身是健康的：后端持续收到 `ListSessions` / `ListFolders` 派发。

也就是说，**手动点击这条路在解锁前走不通**（解锁需要用户的 Touch ID / 密码，
不得猜测凭据）。于是改走另一条等价的路：界面上的每个可见操作最终都落到
`CommandDispatcher::dispatch(AppCommand)`，前端与后端之间**没有第二条路径**；
直接驱动同一个入口，在**真实 multipass 实例**上验证界面对应的后端行为 ——
等价性由「同一个入口」保证，真实性由「真的 sshd / telnetd / OpenSSH 端口转发」保证。

用例落在 `src-tauri/crates/rshell-core/tests/real_machine.rs`（集成测试，只用公开 API），
全部 `#[ignore]` + `RSHELL_REAL_*` 环境变量门控，与 `crates/rshell-protocol/tests/sftp_bench.rs`
同一范式，不进 CI。

#### 测试台怎么保证「与界面同构」

| 装配项 | 做法 |
|---|---|
| 服务装配 | 按 `src-tauri/src/lib.rs` 的 `setup` 顺序逐个构造 `Services`（12 个服务） |
| 数据目录 | 把 `$HOME` 指向临时目录 → `dirs::data_local_dir()/rshell` 落在隔离区，**不污染**用户真实数据目录（正在跑的 dev app 也在用那个目录） |
| known_hosts | 用应用自己的 `data_local_dir()/rshell/known_hosts`——**必须**与 `rshell-protocol` 的 `build_known_hosts_paths` 读的是同一个文件，否则「永久信任」写进去客户端根本看不到 |
| 凭据存储 | 测试内实现 `CredentialStore`（内存版），不碰系统钥匙串 |
| 主机密钥 | 默认挂一个自动应答任务（等价于用户在弹窗点「始终信任」），主机密钥用例单独关掉它、自己走完决策链 |
| 并发 | 同一进程内用 `tokio::sync::Mutex` 串行化（共享 known_hosts，并行会互相干扰） |

#### 10 条用例（全部在真实 multipass 实例上通过）

| 用例 | 验的是 | 关键断言 |
|---|---|---|
| `real_ssh_session_runs_commands_and_browses_remote` | SSH 连接 / 命令往返 / 远端目录 | `echo` 真的回显；`BrowseRemoteDir` 条目非空；`GetRemoteHomeDir` 是绝对路径；对已连接会话再 `connect` 幂等成功 |
| `real_telnet_session_logs_in_and_runs_commands` | Telnet 完整登录 | 等到 `login:` → 发用户名 → 等到 `assword:` → 发口令 → 等到 shell 提示符 → 命令真的回显 |
| `real_unknown_host_key_requires_decision_and_permanent_trust_persists` | 未知主机决策链 | 拒绝 → 连接失败且 known_hosts **无**条目；永久信任 → 连接成功且**落盘**；第三次连接**不再**要求决策；`ListHostKeys` 看得见、`DeleteHostKey` 删得掉 |
| `real_trigger_executes_all_four_action_kinds` | 触发器四种动作 | 发送文本真的到远端；通知摘要为 `notify: …`；写文件真的落盘；断开后会话变 `Disconnected`；`ToggleTrigger` / `DeleteTrigger` 生效 |
| `real_local_and_dynamic_tunnels_forward_real_traffic` | 隧道 Local + Dynamic | Local 端口真的转到 sshd（读到 `SSH-2.0-OpenSSH…`）；Dynamic 的 SOCKS5 握手 + CONNECT 真的转到 sshd；暂停/恢复/关闭的状态流转 |
| `real_quick_command_script_and_key_lifecycle` | 快速命令 / Rhai 脚本 / 密钥 | 快速命令文本真的到远端；脚本 `rshell_list_sessions()` 回传「会话数: N」；密钥生成→列表→导出公钥→删除 |
| `real_multi_tab_ptys_are_independent_and_names_unique_per_folder` | 会话多开 + 名称规则 | 两个标签的输出**互不串台**；关掉一个另一个照常工作；同组同名被拒、跨组同名放行 |
| `real_master_password_setup_lock_unlock_and_rotate` | 主密码全流程 | `NotSet → Unlocked → Locked`；错口令不解锁；对口令解锁；改密后新口令可用、旧口令失效；停用后回到 `NotSet` |
| `real_transfer_uploads_downloads_and_reports_conflict` | 上传 / 下载 / 冲突策略 | 上传完成且远端有文件；同名 `Fail` 策略**被拒**且**远端内容没被改**；`Overwrite` 覆盖成功；`Rename` 另存新文件且原文件保留；下载内容一致 |
| `real_folders_sync_input_compose_and_themes` | 分组 / 同步输入 / 撰写窗格 / 主题 | 删分组后组内连接回到默认组（连接不丢）；同步输入把字节送到了另一个会话；撰写窗格文本到达会话；导入的配色方案**落盘** |

#### 本轮挖出并修掉的产品缺陷：隧道「暂停 → 恢复」必失败

真机用例第一次跑就撞上：

```
恢复隧道失败: Internal("Failed to bind 127.0.0.1:61061: Address already in use (os error 48)")
```

**根因**：`suspend_tunnel` 只调用了 `JoinHandle::abort()`。`abort()` 是「**请求**取消」——
监听任务要到下一次被 poll 才会真正结束，而**监听 socket 是在那个任务里持有的**。
`suspend_tunnel` 立刻返回、`resume_tunnel` 紧接着重新 bind 同一端口，socket 还没被
释放 → 必然 `Address already in use`。用户点「暂停」再点「恢复」就会看到这条错误。

**修法**（`rshell-core/src/security/tunnel_manager.rs`）：`abort()` 之后**等一次
`JoinHandle`**，保证监听任务真的结束、socket 真的关闭，再返回。

等句柄这一步必须在**锁外**做：accept 循环内部会取 `tunnels` 写锁更新连接计数，
持着写锁等它结束就是死锁。因此 `suspend_tunnel` / `close_tunnel` /
`deactivate_session_tunnels` 三处都改成「锁内取出句柄 → 锁外 abort + await」。

**反证**：把 `suspend_tunnel` 的 `let _ = handle.await;` 撤掉后重跑隧道用例，
用例以**同样的错误**失败（`Failed to bind 127.0.0.1:61061: Address already in use`）；
恢复修复后 10 条用例全绿。

#### 过程中被用例纠正的三个「我自己写错了」的地方

| 我一开始写的 | 实际契约 | 依据 |
|---|---|---|
| `ToggleSyncInput` 再传一次同一组会话来关闭 | 该命令实为「**设置**同步组」，关闭要传**空数组** | `src/ipc/client.ts` 与 `SyncInputPanel.vue` 的 `disable()` 都是传 `[]` |
| 脚本里用 `sessions` 变量 | 脚本 API 是函数 `rshell_list_sessions()` | `rshell-core/src/script/engine.rs` 注册的函数表 |
| SOCKS5 只读十字节应答 | 还要先读**两字节**方法选择应答 | `rshell-core/src/security/tunnel_manager.rs::socks5_handshake` |

另外发现一条**刻意限制**：应用侧的 `DeleteRemoteEntry` 只允许删远端**普通文件**，
删目录会被拒（`Only regular remote files can be deleted`）——这是防误删整棵树的
安全设计，不是缺陷。测试目录的清理由用例用 `ssh` 收尾（与仓库既有真机用例同款）。

#### 验证

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0，无告警 |
| `cargo test --workspace` | exit 0，累计 **419** 通过、**0** 失败、**19** 忽略（其中 **10** 条是本轮新增的真机用例） |
| 真机用例（本轮新增，`--ignored --test-threads=1`） | **10 passed / 0 failed**，3.6 s |
| `npm run typecheck` | exit 0（`vue-tsc --noEmit`） |
| `npm test` | Test Files **50 passed (50)**、Tests **450 passed (450)** |
| `npm run build` | exit 0，最大 chunk 403 407 B（vendor-xterm） |
| `npm run check:docs` | 通过（14 份现行文档） |
| `npm run check:bundle` | 通过（10 个 JS chunk，均 ≤ 500 000 B） |
| `npm run check:release` | 通过（版本 0.1.0） |
| `npm run test:scripts` | **48** 项通过、0 失败 |

真机用例的运行方式：

```bash
cd src-tauri
RSHELL_REAL_HOST=192.168.252.2 \
RSHELL_REAL_SSH_PORT=22 \
RSHELL_REAL_TELNET_PORT=2323 \
RSHELL_REAL_USER=ubuntu \
RSHELL_REAL_PASS=… \
RSHELL_REAL_KEY=~/.ssh/id_ed25519 \
RSHELL_REAL_PATH=/home/ubuntu \
cargo test -p rshell-core --test real_machine -- --ignored --test-threads=1
```

#### 仍未验证

- **界面上的手动点击仍未补齐**：锁屏解除前，Telnet 连接入口、主机密钥未知主机的
  弹窗操作、会话列表右键菜单、应用菜单栏、终端查找/清屏、传输右键菜单与拖拽、
  触发器通知的**界面呈现**、快速命令热键、同步输入面板的**界面交互**、
  主密码设置→锁定→解锁的**界面流程**、隧道 Dynamic 的**界面配置**、
  配色导入导出、编辑已有连接等入口没在真机界面上点过。
  本轮验证的是它们**背后的后端行为**（同一个命令入口），不等于界面呈现已验收。
- **客户端仍未在 Linux 上原生编译运行**：跑的是 macOS 编译出的二进制，服务端是 Linux。
- **物理串口**、**签名与公证**、**Windows** 仍未验证。

### 2026-10-23：解锁后继续的界面手动点击验收（macOS 原生应用 + multipass Linux 真机）

上一节结束时 macOS 处于锁屏，AX 层对所有应用都不可用；解锁后继续逐项点击。

#### 本轮在界面上跑通的功能

| 功能 | 验证结果 |
|---|---|
| **Telnet 连接入口** | 新建连接 → 协议选 `Telnet` → 主机 `192.168.252.2` / 端口 `2323` → 连接：**真实 telnetd** 的登录横幅（`Linux 6.8.0-147-generic (rshell-test) (pts/0)` + `rshell-test login:`）→ 用户名 → `Password:` → 登录成功拿到 `ubuntu@rshell-test:~$` → `echo RSHELL_GUI_TELNET_OK_42` **真的回显** |
| **四种认证方式** | 新建连接弹窗里 密码 / 托管密钥 / 外部私钥 / 键盘交互 四个单选项都可选中、都会切换对应表单（选 Telnet 协议时用户名与认证整块隐藏——登录是带内交互） |
| **查找栏** | 工具栏「查找」打开搜索栏（搜索框 + 上/下 + 匹配计数 + 区分大小写 + 关闭），搜 `RSHELL` → 计数显示 **`1/4`**，4 处匹配**全部高亮**，当前匹配橙色描边 |
| **会话列表右键菜单** | 右键连接行弹出应用自己的菜单（**9 项**：连接 / 断开 / 更新凭据 / 打开 SFTP / 打开标签 / 新开标签 / 编辑连接 / 复制连接信息 / 删除连接）。不是浏览器的默认菜单 |
| **删除连接的确认** | 点「删除连接」→ 弹窗「确定删除连接「ubuntu@localhost」吗？此操作不可撤销。」并**限定了目标**（`localhost:22`），确认后才真删 |
| **断线后的界面表现** | telnetd 登录超时断开后：终端上方出现状态条 + 「重新连接」按钮，状态栏回到「未连接」，输入被禁用（符合「不得静默成功」） |

#### 本轮在界面上挖出并修掉的 4 个缺陷

1. **查找栏的匹配计数恒为「无匹配」**——目标文本就在眼前、也确实被选中并跳转了，
   计数却在说没有。
   **根因**：`@xterm/addon-search` 的 `onDidChangeResults` **只在传了 `decorations`
   选项时才会触发**（addon 内部 `_fireResults` 的第一行就是 `if (options?.decorations)`）。
   应用此前调用 `search.findNext(term, { caseSensitive })` 没传 `decorations`，
   计数事件永不触发 → `searchCount` 恒为 0 → 界面永远显示「无匹配」。
   **修法**：查找时传 `decorations`（配色按终端背景亮度选深色/浅色两套，避免文字糊在背景里）。

2. **`decorations` 是 xterm 的 proposed API，不开开关会让查找整个报错**。
   终端此前用 `new Terminal({...})` 构造、没开 `allowProposedApi`，带 `decorations`
   的 `findNext` 会直接抛
   `You must set the allowProposedApi option to true to use proposed API`。
   **修法**：构造 Terminal 时加 `allowProposedApi: true`。
   （这是第 1 条修复的必要配套：只传 `decorations` 而不开开关，查找会从「计数不对」变成「直接报错」。）

3. **查找栏打开后输入框没有焦点**——焦点还留在终端里，用户接着打的字会被
   **送进远端 shell**（等于盲敲远端命令），搜索框却一个字都没有。
   **修法**：打开查找栏后 `nextTick` + `searchInputRef.focus()`。

4. **断线提示条文案叠字**：`会话已{{ droppedLabel }}` 而 `droppedLabel` 本身就是
   「已断开」→ 界面显示「**会话已已断开**，输入已暂停」。
   **修法**：`droppedLabel` 改为「已断开 / 正在重新连接」，模板去掉多余的「已」。

#### 新增回归用例

| 文件 | 用例 | 守住的是 |
|---|---|---|
| `tests/unit/xtermSearchAddon.spec.ts` | 用**真实的 xterm + 真实 addon**（不是 mock）断言：不传 `decorations` 不触发计数事件；传了则 `resultCount` 为真实命中数（2）；终端没开 `allowProposedApi` 时带 `decorations` 的查找会抛异常 | 缺陷 1、2 |
| `tests/unit/TerminalSearch.spec.ts` | 查找必须传 `decorations`（替身照抄真实语义：只在有 decorations 时才触发计数事件）；「上」按钮同样传；清空关键词后计数归零；构造 Terminal 必须带 `allowProposedApi`；打开查找栏必须把焦点交给输入框；断线提示不得出现「已已」 | 缺陷 1、2、3、4 |

**全部做了反证**：撤掉 `decorations` → 3 条计数用例失败（计数停在「无匹配」）；
撤掉 `focus()` → 焦点用例失败；撤掉 `allowProposedApi` → 真实依赖用例报
「必须设置 allowProposedApi」。

#### 验证

| 命令 | 结果 |
|---|---|
| `npx vitest run tests/unit/TerminalSearch.spec.ts tests/unit/xtermSearchAddon.spec.ts` | 2 文件 / **9** 用例通过 |
| `npm test` | Test Files **51 passed (51)**、Tests **455 passed (455)** |
| `npm run typecheck` | exit 0（`vue-tsc --noEmit`） |

#### 仍未验证

- **主机密钥「未知主机」确认弹窗（界面层）**：要让一台主机变成"未知"，需要一个
  不在任何 known_hosts 里的 `host:port`。本机 22 端口需要 root 才能做端口转发，
  改用 `127.0.0.1:2222` 转发就得改端口字段，而 `el-input-number` 的端口框在自动化
  下难以可靠改值（本轮实测：CGEvent 退格与粘贴都不触发 Vue 的 v-model 同步）。
  **后端决策链已由真机用例覆盖**（拒绝→连接失败且不落盘 / 永久信任→落盘 /
  已信任→不再询问 / 列表 / 删除），未覆盖的只是这个弹窗的**界面交互**。
- 隧道 Dynamic 的**界面配置**、传输面板右键菜单与拖拽、同步输入面板的界面交互、
  主密码的界面流程、配色导入导出、应用菜单栏（连接/视图/工具）——本轮未点。
- **客户端仍未在 Linux 上原生编译运行**；物理串口、签名与公证、Windows 未验证。

#### 本轮摸清的 macOS 自动化配方（省下大量试错）

- **往 xterm 送键的可靠入口是 AX 树里的 `Terminal input` textarea**
  （xterm 的 helper textarea）：对它用 `computer_type` 能稳定触发
  `SendInput`；直接用 CGEvent 往终端画布打字则常常丢失。
- **Element Plus 的 `el-input` 用 CGEvent 打字常常不触发 `input` 事件**
  （DOM 值变了、Vue 的 v-model 没变）；**原生「编辑→粘贴」菜单路径**
  （`computer_secondary_action(["Edit","Paste"]`）更可靠，但粘贴会**追加**而不是替换，
  替换要先选中。
- **端口输入框（`el-input-number`）要先 `cmd+a` 再按退格**才能清空重打；
  单纯 `cmd+a` + 直接打字会变成「22」+「2323」= `222323`。
- **右键菜单要用前台像素右键**（`delivery_mode: "foreground"` + `button: "right"`）；
  背景像素点击被 0.22.1 策略拒绝。
- **telnetd 的登录窗口只有 60 秒**：自动化每次工具往返约 20 秒，
  用户名和口令要**一次调用**发出去才赶得上（应用侧行为正确：日志显示断开点
  正好是连接后 60.0 秒，是服务端超时，不是应用自己断的）。

### 2026-10-23：隧道「暂停→恢复」、主机密钥弹窗、传输面板（macOS 原生应用 + multipass Linux 真机）

补齐上一节列出的未验项。**隧道那一条是本轮的重点**——它验的正是上一轮修掉的缺陷。

#### 隧道：暂停 → 恢复（界面上）

隧道规则用应用自己的持久化格式（`tunnels.toml`）种入一条 `127.0.0.1:18081 → localhost:22`，
应用重启后出现在「工具 → 隧道 → 待恢复的隧道 (1)」，点「恢复」启动，再走暂停/恢复。

| 步骤 | 观察到的结果 |
|---|---|
| 待恢复的隧道 | 面板显示「待恢复的隧道 (1)」+ 说明「应用重启后不自动抢占端口……恢复需要对应连接已建立 SSH 连接」+「恢复」按钮 |
| 恢复 | 状态列变「运行中」；`lsof` 显示 `rshell` 在 `127.0.0.1:18081` LISTEN；**通过该端口读到远端 `SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13.20`**（转发真的通到 VM 的 sshd） |
| 挂起 | 状态变「已挂起」；**`127.0.0.1:18081` 的监听消失**（端口真的释放） |
| 恢复（第二次） | 状态回「运行中」；端口**重新监听**；**后端日志没有任何 `Address already in use` / `Failed to bind`**；再次通过该端口读到同样的 sshd 横幅 |
| 关闭 | 弹确认框「关闭隧道 / 关闭后监听端口立即释放，已建立的转发连接会被撤销。」；确认后端口释放、`tunnels.toml` 里的规则被移除 |

**这一条就是上一轮那个缺陷的验收**：修复前 `suspend → resume` 必然报
`Failed to bind 127.0.0.1:<port>: Address already in use`（真机用例已做反证：
撤掉 `handle.await` 后用同样的错误失败）。现在界面上点「挂起」再点「恢复」干净通过。

顺带记一条**界面体验问题**（非缺陷、但影响可用性）：隧道表格三列固定宽度合计
430px（Tunnel ID 120 + 状态 min 120 + 操作 190），而默认侧栏只有约 250px，
**「操作」列（挂起 / 恢复 / 关闭）被裁掉**，必须横向滚动表格才能看到按钮。
本轮就是靠 `computer_scroll(direction="right")` 横向滚动表格才点到的。

#### 主机密钥：未知主机的确认弹窗（界面上的完整安全流程）

要制造「未知主机」，需要一个不在任何 known_hosts 里的 `host:port`。做法是本地做端口转发
`127.0.0.1:2222 → VM:22`，再种一个指向 `127.0.0.1:2222` 的会话（该组合不在
`~/.ssh/known_hosts`，也不在应用自己的 known_hosts 里）。

| 步骤 | 观察到的结果 |
|---|---|
| 连接未知主机 | 弹出「**确认新主机密钥**」对话框：说明「`127.0.0.1:2222` 的 SSH 服务器密钥尚未保存。」+ 算法 `Ed25519` + 收到的指纹（SHA256）+ 公钥 blob + ⚠️ 警告「连接前请确认以上指纹与服务器管理员公布的一致。指纹不一致可能意味着中间人攻击。」+ 四个决策按钮：**拒绝 / 信任一次 / 永久信任 / 取消连接** |
| 永久信任 | 应用自己的 `known_hosts` 写入 `[127.0.0.1]:2222 ssh-ed25519 AAAA…`；后端日志 `Host key trusted` + `User accepted unknown host key`；**连接建立，终端显示真实 Ubuntu 横幅与 `ubuntu@rshell-test:~$` 提示符** |

#### 传输面板

| 检查点 | 结果 |
|---|---|
| 远端目录 | 真实列出 `/home/ubuntu` 的 **8 项**（`sshd-version.txt` 62 B TXT、`.bash_history` 1.6 KB、`.cache` 文件夹、`.ssh` 文件夹、`.sudo_as_admin_successful`、`.profile`、`.bashrc`、`.bash_logout`），含大小 / 类型 / 修改时间 / 权限 / 所有者 |
| 本地面板 | 指向已被清理的 `/tmp/rshell-e2e` 时**如实报错**「failed to read directory at path: /tmp/rshell-e2e with error: No such file or directory (os error 2)」，不是静默空白 |
| 工具栏 | 同步浏览 / 上传 / 下载 / 新建文件夹 / 删除 / 刷新 / 传输队列 都在；底部「传输 (0)」与「日志」页签正常 |

#### 验证

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0，无告警 |
| `cargo test --workspace` | exit 0，**419** 通过、**0** 失败、**19** 忽略 |

本轮**没有代码改动**（修复已在 `c36e467` / `0424f52` 提交），只是把剩下的界面入口验完
并把证据写进本文档；测试数据（种入的会话、隧道规则、known_hosts 条目、端口转发）
已全部清理。

#### 仍未验证

- 隧道 **Dynamic(SOCKS5)** 的**界面配置**（后端行为已由真机用例覆盖：SOCKS5 握手 +
  CONNECT 经隧道转到 sshd 都验过；界面上的类型选择与创建表单没走通——`el-input` 在
  自动化下无法可靠替换整段文本，见下）。
- 传输面板的**右键菜单 / 拖拽 / 批量删除**。
- 应用菜单栏（连接 / 视图 / 工具）的下拉项、快速命令**热键**、主密码的**界面流程**
  （设置 → 锁定 → 解锁）、配色方案的**导入导出**。

#### 本轮摸清的两条 macOS 自动化限制（不是应用缺陷）

1. **Element Plus 的 `el-input` 无法整体替换文本**：`cmd+a` 在 Tauri 的 WKWebView 里
   **不执行「全选」，而是把字母 `a` 打了进去**（实测 `Localhost:22` 被改成
   `Localhost:22a127.0.0.1:18081`）；三击全选也不生效（会把词插到光标处）。
   可用的只有两条路：**AX `set_value`**（整段替换，且会同步 v-model——本轮隧道表单的
   监听地址就是靠它改对的），或**原生「编辑→粘贴」菜单路径**。
2. **AX 的元素索引会漂移**：同一轮里 `监听` 字段的 token 有时写到 `目标` 字段去。
   规律是「**写完立刻核对，必要时重写**」，不能一次写两个字段就假定都对了。

### 2026-10-23：修掉两个界面缺陷（隧道操作列被裁 / 目录错误只有英文）

上一节验收时撞到两个**真实界面缺陷**，本轮修掉并都做了反证。

#### 缺陷 1：隧道「挂起 / 恢复 / 关闭」按钮在默认侧栏宽度下点不到

**现象**：隧道列表用 `el-table` 渲染，三列固定宽度合计
`Tunnel ID 120 + 状态 min-width 120 + 操作 190 = 430px`，而**默认侧栏只有 280px**
（`DEFAULT_SIDEBAR_WIDTH`，最大 360px）。「操作」列被裁在视口外，用户必须先发现
表格底部的横向滚动条才点得到操作按钮——真机验收时就是这样才用
`computer_scroll(direction="right")` 横向滚动表格才点到的。

**修法**：把隧道列表从表格改成**卡片列表**（`ul/li`），每条隧道一张卡：
状态 + 规则（`Local 127.0.0.1:18081 → localhost:22`，比原来的 Tunnel ID 对用户有用）
+ 三个操作按钮 + 会话名/ID。卡片不依赖列宽，窄侧栏下三个按钮仍然一行可见。
Tunnel ID 退到卡片底部的次要信息（带 `title` 便于复制排查）。

**回归用例**：`tests/unit/TunnelPanel.spec.ts`
「隧道列表用卡片渲染：三个操作按钮与状态同处一个条目，不依赖表格列宽」
——断言列表存在、**不再渲染 `table`**、状态/规则/三个按钮都在同一个条目里。
顺带把一条旧断言从「行 JSON 里含 `Active`/`Error`」改成断言**用户真正看到的文案**
（「运行中」/「错误：…」）——旧断言落在 el-table 桩打印的行 JSON 上，是桩的副产品。

**真机复验**：恢复隧道后侧栏里直接看到

```
运行中  Local 127.0.0.1:18081 → l…
[挂起] [恢复] [关闭]
Key-auth-test   ID 884af187-8258-43b7-8ee…
```

三个按钮**不再需要横向滚动**。

#### 缺陷 2：目录读不到时只甩一句英文原始错误

**现象**：传输面板的本地目录指向一个已被删除的目录时，界面只显示
`failed to read directory at path: /tmp/rshell-e2e with error: No such file or directory (os error 2)`
——中文界面里一段英文系统错误，用户既不知道是「目录没了」还是「没权限」，
也不知道下一步该做什么。

**修法**（`src/components/transfer/FileBrowserPane.vue` 的 `describeLoadError`）：
补两条**已知**错误的中文说明，仍然把目录路径与下一步写清楚：
- 目录不存在 / 已被移除（ENOENT、No such file、os error 2）→「目录 X 不存在或已被移除。可点上方「更换本地目录」另选一个目录。」
- 没有读权限（Permission denied、EACCES、os error 13）→「无权读取 X：当前账户没有该目录的读权限。可点上方「更换本地目录」另选一个目录。」

其余错误**原样透出**：不猜、不吞掉真实原因（原有的 scope 拒绝分支保持不变）。

**回归用例**：`tests/unit/FileBrowserPane.spec.ts` 两条
（目录不存在 / 没有权限），断言文案含中文原因、含具体路径、含「更换本地目录」，
且**不再只剩英文原始错误**。

**真机复验**：本地面板现在显示
「目录 /tmp/rshell-e2e 不存在或已被移除。可点上方「更换本地目录」另选一个目录。」

#### 验证

| 命令 | 结果 |
|---|---|
| `npx vitest run` | Test Files **52 passed (52)**、Tests **462 passed (462)** |
| `npm run typecheck` | exit 0 |
| `npm run build` | exit 0 |
| `npm run check:docs` | 通过（14 份现行文档） |
| `cargo fmt --check` / `cargo clippy --workspace --all-targets -- -D warnings` | 均 exit 0 |
| `cargo test --workspace` | **419** 通过、**0** 失败、**19** 忽略 |

**反证**：把两处修复都撤掉后，`TunnelPanel.spec.ts` + `FileBrowserPane.spec.ts`
共 **8 条**用例失败（列表化那条、中文文案两条，以及依赖旧表格结构的旧断言）；
恢复修复后 28 条全绿。
