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

仓库的本地校验只走两条共享入口脚本；命令细节以 [scripts/README.md](../scripts/README.md) 为准，不要在文档或 PR 里复制脚本内部的命令清单。

```bash
npm run verify   # 走 scripts/verify.sh：前端类型检查/测试/构建/文档/脚本测试 + Rust fmt/clippy/test
npm run audit    # 走 scripts/audit.sh：npm audit (官方 registry) + cargo audit (Cargo.lock)
```

`scripts/verify.sh --skip-install` 跳过 `npm ci`，适用于依赖已安装的快速复跑。Rust workspace 在 `src-tauri/`，不要从根目录运行 cargo 检查；脚本会自动切到正确目录并设置 `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1`。

## CI 与发版环境

`.github/workflows/ci.yml` 在 macOS runner 上跑共享校验与 `--unsigned` 预检；`.github/workflows/release.yml` 在推送 `v*` tag 后构建三个平台的安装包并发布到 GitHub Release。两条流水线都只调用仓库脚本，不复制脚本内部命令，且第三方 action 全部按提交 SHA 锁定。

本地若要复现某个平台的构建，除上面列出的工具外还需要对应平台的原生依赖。macOS 需要 `rustup target add aarch64-apple-darwin x86_64-apple-darwin` 才能产出 universal 包；Windows 需要 MSVC 工具链，MSI 打包由 Tauri 内置的 WiX 完成；Linux 需要 `libwebkit2gtk-4.1-dev`、`libayatana-appindicator3-dev`、`librsvg2-dev`、`libxdo-dev`、`libssl-dev` 等系统库，缺失时 Tauri 会在 `tauri::generate_context!` 处报错而不是静默降级。CI 只构建 macOS x64/arm64 合并包、Windows x64 与 Linux x64，未覆盖 Linux aarch64 与 Windows aarch64。

本机 rustup 曾因 rust-analyzer 自动更新冲突失败。已有完整 stable 时：

```bash
RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 npm run tauri:dev
```

Rust 命令可设置同样变量；这是避开本机更新故障，不是跳过编译。

jsdom 缺少 Canvas/WebGL，终端测试可能有 Canvas 提示；真实渲染需 macOS WebView。Vite bundle 大小警告不等于构建失败。

## 数据与权限

默认数据位于 `~/Library/Application Support/rshell/`。SSH 密码和密钥口令存放在 macOS 钥匙串（Keychain），会话 TOML 只含凭据元数据；旧版会话的明文凭据会在读取时迁移到钥匙串并从文件移除。钥匙串条目缺失时连接失败关闭，编辑会话、重新输入凭据并保存可恢复连接；主密码用于应用锁定/验证，不是凭据保险库。私钥、快速命令、触发器和日志也可能敏感，不要共享整个数据目录或提交到 Git。

本地文件先通过目录选择器授权，SFTP 先连接 SSH 会话。Telnet 使用实际服务端 TCP 端口；Serial 使用实际 `/dev/cu.*` 或 `/dev/tty.*` 和匹配参数。

自动化工具无法识别未打包开发进程时，应记录限制，不能把进程存在当作窗口交互通过。详见 [验证记录](09-macos-validation.md)。
