# macOS 开发环境

更新：2026-09-26。当前架构是 Tauri 2 + Vue 3 + Rust。

安装 Xcode Command Line Tools、Node.js 20+、npm、Rust stable。仓库 `rust-toolchain.toml` 使用 stable 和 rustfmt/clippy/rust-analyzer；本次验证使用 Rust 1.98.1，依赖版本以 lock 文件为准。

## 启动与构建

根目录执行：

```bash
npm ci
npm run tauri:dev
```

Tauri 自动启动 Vite，开发地址为 `http://localhost:51820`。仅 `npm run dev` 是网页，没有 Tauri IPC、原生对话框和文件权限。

打包使用 `npm run tauri:build`。开发启动和前端构建不等于签名、公证或分发验证。

## 检查

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

Rust workspace 在 `src-tauri/`，不要从根目录运行 cargo 检查。

本机 rustup 曾因 rust-analyzer 自动更新冲突失败。已有完整 stable 时：

```bash
RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 npm run tauri:dev
```

Rust 命令可设置同样变量；这是避开本机更新故障，不是跳过编译。

jsdom 缺少 Canvas/WebGL，终端测试可能有 Canvas 提示；真实渲染需 macOS WebView。Vite bundle 大小警告不等于构建失败。

## 数据与权限

默认数据位于 `~/Library/Application Support/rshell/`。会话可能含明文密码，私钥、快速命令、触发器和日志也可能敏感，不要共享整个数据目录或提交到 Git。主密码服务尚未加密会话存储。

本地文件先通过目录选择器授权，SFTP 先连接 SSH 会话。Telnet 使用实际服务端 TCP 端口；Serial 使用实际 `/dev/cu.*` 或 `/dev/tty.*` 和匹配参数。

自动化工具无法识别未打包开发进程时，应记录限制，不能把进程存在当作窗口交互通过。详见 [验证记录](09-macos-validation.md)。
