# 构建与文档检查脚本

更新：2026-09-27。macOS 的正式入口是根目录的 `npm run tauri:dev` 和 `npm run tauri:build`。

## 仓库统一入口

所有本地验证与审计都通过下列脚本完成，CI 与文档只调用这些入口，不重复脚本内部命令：

- `npm run verify`（底层 `bash scripts/verify.sh [--skip-install]`）：从仓库根目录运行前端类型检查、单元测试、构建、文档契约、脚本测试，再依次执行 `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`。任一子命令失败立即返回非零退出码，不会被静默吞掉。
- `npm run audit`（底层 `bash scripts/audit.sh`）：`npm audit --registry=https://registry.npmjs.org` 加 `cargo audit --file src-tauri/Cargo.lock`。`cargo-audit` 缺失时给出明确安装提示并以非零退出，不允许“未执行”冒充通过。

Cargo 命令一律通过 `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1` 触发，避免 rustup 自动更新冲突；脚本永远不会打印或接受签名、notarytool 或会话凭据。

## 旧脚本保留范围

- `bash scripts/build.sh [target-triple]`：`npm run tauri:build` 的薄包装，签名、公证、安装验收仍不由本脚本自动完成。
- `scripts/build.ps1`、`scripts/build.cmd`、`scripts/read-version.ps1`：保留的历史 Windows 脚本，本轮未更新或验证，不作为当前构建入口。
- `npm run test:scripts` 检查包装脚本（包含 build、check-docs、新增的 automation）的命令、工作目录、target、env 与错误透传，不执行完整发布构建。
- `npm run check:docs` 检查当前用户文档中的架构、启动命令、移除范围说明、凭据边界与本地链接；带日期的历史设计与执行计划不作为产品手册扫描。

## macOS 发布预检与产物校验

- `bash scripts/macos-release-preflight.sh [--unsigned] [app-path]`：确认 Bundle ID、`Contents/Info.plist` 完整、app 路径存在。`--unsigned` 模式只校验这些条件，CI 与本地都可跑；不传则要求同时设置 `APPLE_SIGNING_IDENTITY` 与 `APPLE_NOTARY_PROFILE`，并执行 `codesign --verify --deep --strict`、`xcrun notarytool submit --wait`、`xcrun stapler staple`、`spctl --assess`。脚本不会回显任何凭据，也不会与 `--unsigned` 同时接受签名/公证变量。
- `bash scripts/macos-verify-app.sh <app-path>`：对已经构建好的 `.app` 验证 Bundle ID、严格代码签名和 Gatekeeper 评估；不执行签名或公证，仅作为产物验收工具。
- `app-path` 默认为 `src-tauri/target/release/bundle/macos/RShell.app`（或调试包 `src-tauri/target/debug/bundle/macos/RShell.app`），调试 `.app` 体积不作为发布体积结论。

完整环境说明见 [macOS 开发环境](../docs/07-project-setup-guide.md)，证据见 [验证记录](../docs/09-macos-validation.md)。