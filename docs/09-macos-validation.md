# macOS 验证记录

日期：2026-09-27。分支：`codex/release-hardening`（凭据加固与发布就绪合并入 main）。此记录区分代码检查、自动测试、原生进程启动和真实交互，不将未执行项记为通过。

## 环境

macOS 本机，Rust stable 1.98.1，Tauri 2 + Vue 3 + xterm.js。依赖由 `npm ci` 和 Cargo.lock 锁定。Rust 命令在 `src-tauri/` 执行，并使用 `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1` 避开本机 rustup 自动更新组件冲突。

## 自动检查

本轮同时运行仓库内统一入口脚本（与 CI 复用同一份逻辑，命令清单见 [scripts/README.md](../scripts/README.md)）：

- `npm run verify`（`scripts/verify.sh`）：前端 `typecheck` / `test` / `build` / `check:docs` / `check:bundle` / `test:scripts`，Rust `fmt` / `clippy --workspace --all-targets -- -D warnings` / `test --workspace`，全部 0 失败。
- `npm run audit`（`scripts/audit.sh`）：`npm audit --registry=https://registry.npmjs.org` 与 `RUSTUP_TOOLCHAIN=stable cargo audit --file src-tauri/Cargo.lock`，本地开发机需先 `cargo install cargo-audit --version 0.22.2 --locked`，否则脚本立即以非零退出并指出安装命令。

`npm test` 累计 75 项通过（20 文件），`node --test scripts/*.test.mjs` 累计 32 项通过；`npm run check:bundle` 在 macOS 调试包构建后扫描 `dist/assets/` 并断言每个 JS chunk ≤ 500 KiB、无 `.map`，最大 chunk 403 KiB（vendor-xterm）。

同时构建 Tauri 与运行 rustdoc 时曾出现 `E0463`（找不到 tauri crate）；停止重叠构建后，文档测试及全工作区测试均串行复跑通过，未通过禁用文档测试规避错误。

早期 App 测试出现 jsdom Canvas 能力提示，已通过 mock 与测试无关的终端子组件消除；真实渲染测试边界保持明确。Vite 仍报告 bundle 大小与注解警告。初始 `npm audit` 报告 6 项 advisory；Vite 6.4.3、Vitest 4.1.11、nanoid 3.3.19 更新后重新安装，最终 audit 报告 0 项。该结果仅对应 npm 已知漏洞数据库，不代表完整安全审计。

macOS 构建脚本回归通过（2/2），覆盖图标资源存在性、工作目录、Tauri 调用、target 参数和失败退出码。前端最终类型检查与 65 项测试通过，当前文档契约通过（14 份文档）。其中 3 项标题栏集成测试执行 Cargo.lock 对应的 Tauri 原生拖动脚本和生成权限清单；本机全部实际执行，没有跳过。仅前端安装且缺少 Cargo 依赖/权限清单时，这 3 项会明确跳过，基础权限断言仍会执行。

独立审查发现的 SSH 子通道输出隔离、连接取消/删除竞争、退出清理、协议请求锁、串口取消丢字节、Telnet resize、通知文案、新终端配色和传输错误详情均已修复并通过定向复审。新增本地真实 SSH 传输回归覆盖 shell 输入、尺寸、独立子通道数据与 EOF；它不等于完整 SFTP 文件系统或外部服务器验收。

本轮另修复：SFTP 传输改为 64 KiB 分块拷贝并按 200ms 节流广播进度，速度随 `TransferTaskInfo` 快照下发；删除不可达的 `HostKeyManager::check_host_key` 及其连带死状态，前端对 nil `decision_id` 决策事件做防御；断开/删除会话与传输队列读取失败不再静默；pause/resume 核心接口无界面入口已在功能与限制中记录。真实 SSH/SFTP 服务器上的进度观感、面板错误展示等 GUI 项仍属未执行验收，不因单元测试通过而勾选。

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
