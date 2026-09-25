# RShell

RShell 是面向 macOS 的远程终端与 SFTP 文件管理客户端，使用 Tauri 2、Vue 3、TypeScript 和 Rust。终端由 xterm.js 渲染，业务逻辑通过类型化 IPC 与界面分离。

当前版本：0.1.0。本文对应 2026-09-26 的 macOS 加固分支。Windows 本轮不作为交付目标。RDP 已移出产品范围，相关运行时代码、公共类型和直接依赖已删除。

## 当前功能

- 会话：SSH、Telnet、Serial 配置、保存、连接和断开；已保存会话在启动时恢复。
- 终端：输入、远端输出、窗口尺寸同步、搜索和清屏；连接状态来自后端事件。
- SFTP：真实远程目录浏览、本地目录选择、上传、下载、远程目录创建和普通文件删除；传输队列显示任务结果和错误。
- SSH 隧道：Local 和 Dynamic/SOCKS5。旧 Remote 配置被跳过并保留原文件，不自动建立监听。
- 安全：主机密钥确认、一次信任和永久信任；SSH 密钥生成、导入、删除和公钥导出接口；主密码服务。
- 自动化：快速命令的创建、删除、执行和保存；触发器创建、开关、删除和保存。界面支持正则匹配后发送文本，核心接口还支持通知、断开和日志动作。
- Rhai：核心脚本接口支持会话发送、列会话和执行快速命令，执行错误会返回调用方。
- 主题：应用主题和终端配色切换，颜色应用到界面与 xterm.js。
- WASM：扫描本地插件清单、校验并编译模块、加载和卸载。插件扩展点声明不等同于已实现动态界面或宿主权限接口。

Telnet 不加密流量。Serial 需要可访问的 macOS 串口设备路径，例如 `/dev/cu.usbserial-...`。SFTP 和 SSH 隧道需要 SSH 会话。

## 启动

需要 Node.js 20+、npm、Rust stable，以及 macOS Xcode Command Line Tools。

```bash
npm ci
npm run tauri:dev
```

仅启动前端开发服务器使用 `npm run dev`；普通浏览器不具备 Tauri IPC 和本地文件权限。

构建桌面应用：

```bash
npm run tauri:build
```

本机 Rust 工具链曾遇到自动更新组件冲突；已有可用 stable 时可临时使用：

```bash
RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 npm run tauri:dev
```

详细环境和排障见 [开发环境指南](docs/07-project-setup-guide.md)。

## 验证

```bash
npm run typecheck
npm test
npm run test:scripts
npm run build
npm run check:docs
cd src-tauri
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

自动化检查、真实启动结果及尚未执行的外部设备/服务器场景，统一记录在 [macOS 验证记录](docs/09-macos-validation.md)。测试通过不能替代真实 SSH/SFTP 服务端或物理串口验证。

## 代码结构

- `src/`：Vue 组件、Pinia 状态和 Tauri IPC 客户端。
- `src-tauri/src/`：Tauri 初始化、命令薄壳、事件桥和终端字节通道。
- `src-tauri/crates/rshell-api/`：命令、事件、结果和共享数据类型。
- `rshell-core/`：会话、传输、安全、自动化和主题服务。
- `rshell-protocol/`：SSH/SFTP、Telnet 和串口协议。
- `rshell-infra/`：存储、加密和平台基础设施。
- `rshell-plugin-sdk/`：插件清单、加载器和 WASM 沙箱。

以上 crate 路径均位于 `src-tauri/crates/`。

## 数据与边界

macOS 数据目录为 `~/Library/Application Support/rshell/`。会话、密钥、主机密钥、隧道及自动化配置由对应服务管理。会话 TOML 可能包含明文密码/口令；主密码服务尚未加密会话存储。请勿把会话配置、私钥或日志提交到版本库，备份和分享前检查认证信息。

当前不提供 RDP、Remote Forward、FTP/FTPS、Windows ConPTY，也不把截图、录屏、虚假暂停/恢复按钮作为已交付功能。当前支持状态见 [功能与限制](docs/08-incomplete-features.md)。

## 文档

[项目计划](docs/02-project-plan.md) · [详细设计](docs/03-detailed-design.md) · [开发规范](docs/05-development-standards.md) · [测试策略](docs/06-test-strategy.md) · [贡献指南](CONTRIBUTING.md)

Apache-2.0 licensed. See [LICENSE](LICENSE).
