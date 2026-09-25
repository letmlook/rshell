# RShell macOS 现有功能加固设计

## 目标

将 RShell 收束为一款在 macOS 上可构建、可测试、可手动验证的 SSH/SFTP、
Telnet 与串口客户端。此轮只兑现仓库已暴露的产品能力：不新增协议、不做
Windows ConPTY，也不保留未交付能力的入口或宣传。

## 已确认的边界

- **保留并加固**：SSH（含 SFTP）、Telnet、Serial、终端、会话、密钥与主密码、
  host-key 决策、Local/Dynamic SSH 隧道、快速命令、撰写窗格、同步输入、
  触发器、Rhai、主题和 WASM 插件。
- **移除**：RDP 的 Rust 模块、公共类型、依赖、前端类型、测试与所有用户文档
  中的产品承诺。
- **不支持并移除入口**：Remote SSH forwarding。现有 `russh` 客户端版本没有
  可靠的 `tcpip-forward` 请求实现；为它升级或替换 SSH 库属于扩展范围。Local
  与 SOCKS5 Dynamic forwarding 继续支持。
- **平台**：macOS 是本轮唯一运行验收平台。Windows 实现不改动，但文档不再把
  Windows 的运行验证列为本轮交付条件。

## 当前问题

项目已经从 README 所述的 GPUI 架构迁移到 Tauri 2、Vue 3、xterm.js 和
Dockview，但顶层文档及若干设计文档仍描述旧实现与 RDP。前端还有已经渲染却
没有动作的 SFTP 工具栏操作；侧栏文件区域也仍是占位。后端的 session recv
循环、触发器动作、脚本回调和主题数据桥接存在“类型或界面已在、运行闭环未
完成”的情况。`docs/08-incomplete-features.md` 已混合历史完成项和真正遗留项，
不能再作为当前状态报告。

## 架构与数据流

现有分层保持不变：Vue 组件只能通过 `src/ipc/client.ts` 调用 Tauri command，
并订阅 Tauri event；`src-tauri/src/commands.rs` 是命令适配层；业务规则留在
`rshell-core`；协议 I/O 留在 `rshell-protocol`。

每一项可见控件必须遵循同一条闭环：

```
Vue action → typed IPC client → Tauri command → core service
         ← typed result / event ← dispatcher  ← protocol operation
```

控件没有对应的后端操作时，不能保留静默的空回调。应当接入已有操作；若后端
能力不存在且不在本轮范围内，则从界面移除并在文档的支持矩阵中标记为不支持。

## 设计：RDP 与隧道范围

删除 `rshell-protocol::rdp`、`RdpConfig`、协议枚举的 `RDP` 成员以及 IronRDP、
rustls 图形相关的直接依赖。所有反序列化的协议输入必须拒绝已删除的 `RDP`
值，并提供清楚的迁移错误；已有 session 配置不得被静默当作 SSH 执行。

隧道的公开类型、命令、面板选项和说明只提供 `Local`、`Dynamic`。旧配置中
的 Remote 规则在载入时报告为不支持、跳过启动且保留文件内容，避免破坏用户
的配置文件或误建直连隧道。

## 设计：SFTP 和工作区操作

`TransferWorkspace` / `FileBrowserPane` 成为传输工作区唯一的文件操作来源。
上传和下载必须先由用户选取源/目标路径，再调用已经存在的队列/传输 API；
刷新、创建目录和删除必须操作当前激活窗格，并在没有活跃 SFTP session 或
没有选择条目时禁用。删除要求前端确认，后端再校验路径，不接受空路径或根
目录删除。操作结束后使用事件或显式 refresh 更新两个窗格与传输队列。

侧栏的“文件”面板要么复用同一文件浏览实现并要求已选 SSH/SFTP session，
要么在没有活跃 session 时显示说明性空态；不得显示伪文件列表。终端工作区
的工具栏不承担 SFTP 文件操作。

## 设计：自动化、主题与事件可靠性

session 输出循环以 UTF-8 可解析的文本片段驱动 trigger engine。每种 trigger
action 都有一个确定去向：发送文本经 session sender；运行脚本经 script
engine；通知经 event bus；快速命令复用 quick-command service。断开连接会
停止该 session 的触发处理，触发失败产生可见 event 而不是吞掉错误。

Rhai host functions 通过窄的 service trait 注入，提供已声明的发送、会话列表
和快速命令能力；不把 Tauri 或 Vue 状态泄漏进 core。主题命令返回完整的
`ThemeColorSet`，Vue store 将其转换为 CSS variables；应用启动、手动切换和
“跟随系统”三条路径使用同一个更新函数。

所有订阅都在 Vue 卸载时解除；IPC 错误保持带上下文的 `Result`/rejection，
组件向用户展示可行动的信息，不以 `console.error` 作为唯一结果。

## 测试与验收

自动测试必须覆盖以下最低行为：

1. RDP 类型及依赖已不存在；旧 RDP session 输入被拒绝。
2. Local/Dynamic 隧道仍可序列化、载入和启动；Remote 规则安全跳过并产生
   明确错误。
3. 文件操作不会在无连接、空路径、根目录或无选择时执行；确认后的有效操作
   精确调用对应 IPC。
4. 每种 trigger action 都从终端输出到可观察结果，含断连和动作失败路径。
5. 脚本 host API 仅调用目标 service，错误不被吞掉。
6. 主题的后端结果完整传至 CSS variables，订阅在卸载时释放。
7. IPC command 和前端 client 的参数、结果、事件名由共享类型或生成契约约束。

macOS 验收命令为 `npm ci`、`npm run typecheck`、`npm test`、`cargo fmt --check`、
`cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo test --workspace`
（Rust 命令从 `src-tauri/` 执行）。此外手工启动 `npm run tauri:dev`，完成 SSH
连接、终端输入/复制、SFTP 上传下载与目录操作、Local/Dynamic 隧道、主题切换、
触发器和插件加载的清单式检查。无法提供本地真实设备的 Serial 操作可用 mock
和枚举/配置测试替代，但不将其标成已在物理串口上验证。

## 文档交付

完成后，README 成为唯一简明产品状态页，准确说明 Tauri/Vue 架构、支持协议和
macOS 验收边界。`docs/02` 至 `docs/08`、`CLAUDE.md`、`CONTRIBUTING.md` 与
CHANGELOG 同步删除 RDP/GPUI 旧结论、更新命令和功能矩阵。历史研究资料可以保留
“已移出范围”的记录，但不得把 RDP 表述为当前承诺；不再维护一份过期的“未完成
功能”清单，改为实际剩余风险和已验证范围。

## 非目标

- 实现 RDP、FTP/FTPS、Windows ConPTY 或新的传输协议。
- 引入新的 UI 框架、替换 SSH 库，或重新设计既有工作区布局。
- 宣称跨平台运行测试或真实远端服务验证尚未取得的结果。

## 完成定义

当仓库中没有 RDP 的运行时/公共 API/产品承诺，所有可见 macOS 控件要么完成
真实操作要么被移除，自动质量门通过，macOS 手测清单完成，且所有顶层和用户
面向文档与实际 Tauri/Vue 实现一致时，本轮完成。
