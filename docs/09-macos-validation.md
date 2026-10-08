# macOS 验证记录

本记录按轮次分节记日期（最新一轮：2026-10-03）。分支：`codex/release-hardening`（凭据加固与发布就绪合并入 main）。此记录区分代码检查、自动测试、原生进程启动和真实交互，不将未执行项记为通过。

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
