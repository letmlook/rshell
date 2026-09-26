# macOS 验证记录

日期：2026-09-26。分支：`codex/macos-hardening`。此记录区分代码检查、自动测试、原生进程启动和真实交互，不将未执行项记为通过。

## 环境

macOS 本机，Rust stable 1.98.1，Tauri 2 + Vue 3 + xterm.js。依赖由 `npm ci` 和 Cargo.lock 锁定。Rust 命令在 `src-tauri/` 执行，并使用 `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1` 避开本机 rustup 自动更新组件冲突。

## 自动检查

以下结果来自最终审查修复及 Rust 格式化后的重新执行：

- `npm run typecheck`：通过。
- `npm test`：18 个文件，65 项通过。
- `npm run test:scripts`：2 项通过。
- `npm run build`：通过，Vite 6.4.3。
- `npm run check:docs`：14 份当前文档通过。
- `cargo fmt --all --check`：通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。
- `cargo test --workspace --quiet`：154 项单元/集成测试通过，1 个既有文档示例忽略，其余文档测试无失败。

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

限制：未提供用于验收的真实 SSH/SFTP 服务器凭据或物理串口；没有把单元测试/回环服务替代为上述实测。Windows 不属于本轮验收。

## 范围审计

当前用户文档不再使用旧 UI 架构或旧启动命令。RDP 仅作为已删除/拒绝输入的说明；运行时公共协议为 SSH、Telnet、Serial。历史设计和执行计划保留在 `docs/superpowers/`，不能作为当前功能承诺。

会话配置可能含明文认证数据，主密码服务未成为持久化凭据保险库。该限制与其他 API/界面边界见 [功能与限制](08-incomplete-features.md)。
