# 构建与文档检查脚本

更新：2026-10-02（体检修复轮）。macOS 的正式入口是根目录的 `npm run tauri:dev` 和 `npm run tauri:build`。

## 仓库统一入口

所有本地验证与审计都通过下列脚本完成，CI 与文档只调用这些入口，不重复脚本内部命令：

- `npm run verify`（底层 `bash scripts/verify.sh [--skip-install]`）：从仓库根目录运行前端类型检查、单元测试、构建、文档契约、bundle 体积检查、发布版本一致性检查、脚本测试，再依次执行 `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`。任一子命令失败立即返回非零退出码，不会被静默吞掉。
- `npm run audit`（底层 `bash scripts/audit.sh`）：`npm audit --registry=https://registry.npmjs.org` 加 `cargo audit --file src-tauri/Cargo.lock`。`cargo-audit` 缺失时给出明确安装提示并以非零退出，不允许“未执行”冒充通过。

`scripts/audit-ignored.txt` 是**唯一**的例外清单：`audit.sh` 把其中每个 ID 作为 `cargo audit --ignore` 传入，并逐条回显到 stderr，使 CI 日志能看到被豁免的告警。文件缺失会被当作仓库配置错误而非“没有例外”直接失败——否则删掉文件就等于悄悄关掉整道门禁。维护规则：只允许写上游没有可用修复的条目（凡有修复版本的一律靠升级依赖消除），每条必须写明原因与复查条件，其余任何新告警仍然非零退出。

当前唯一例外：`RUSTSEC-2023-0071`（rsa 的 Marvin 时序攻击）。上游 RustCrypto/RSA 有意把 `patched` 留空——最新稳定版 0.9.10 与最新预发布 0.10.0-rc.18 均仍受影响；关闭 russh 默认的 `rsa` feature 会失去 rsa-sha2-256/512 主机密钥算法，导致仅提供 RSA 主机密钥的服务器无法连接。

Cargo 命令一律通过 `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1` 触发，避免 rustup 自动更新冲突；脚本永远不会打印或接受签名、notarytool 或会话凭据。

## 旧脚本保留范围

- `bash scripts/build.sh [target-triple]`：`npm run tauri:build` 的薄包装，签名、公证、安装验收仍不由本脚本自动完成。
- 历史 Windows 脚本 `scripts/build.ps1` 与 `scripts/build.cmd` 已删除（2026-09-30）：它们按旧仓库布局在仓库根执行 `cargo build`，而 Rust workspace 现位于 `src-tauri/`（仓库根已无 Cargo.toml），按现状必然失败。Windows 本地构建同样使用 `npm run tauri:build`，由 Tauri CLI 进入 src-tauri workspace 并处理目标三元组。
- `scripts/read-version.ps1`：读取 `src-tauri/Cargo.toml` 版本号的历史辅助脚本；原调用方（上述两个已删脚本）已移除，当前无入口使用，仅为避免误删保留。
- `npm run test:scripts` 检查包装脚本（包含 build、check-docs、新增的 automation）的命令、工作目录、target、env 与错误透传，不执行完整发布构建。
- `npm run check:docs` 检查当前用户文档中的架构、启动命令、移除范围说明、凭据边界与本地链接；带日期的历史设计与执行计划不作为产品手册扫描。

## macOS 发布预检与产物校验

- `bash scripts/macos-release-preflight.sh [--unsigned] [app-path]`：确认 Bundle ID、`Contents/Info.plist` 完整、app 路径存在。`--unsigned` 模式只校验这些条件，CI 与本地都可跑；不传则要求同时设置 `APPLE_SIGNING_IDENTITY` 与 `APPLE_NOTARY_PROFILE`，并执行 `codesign --verify --deep --strict`、`xcrun notarytool submit --wait`、`xcrun stapler staple`、`spctl --assess`。脚本不会回显任何凭据，也不会与 `--unsigned` 同时接受签名/公证变量。
- `bash scripts/macos-verify-app.sh <app-path>`：对已经构建好的 `.app` 验证 Bundle ID、严格代码签名和 Gatekeeper 评估；不执行签名或公证，仅作为产物验收工具。
- `app-path` 默认为 `src-tauri/target/release/bundle/macos/RShell.app`（或调试包 `src-tauri/target/debug/bundle/macos/RShell.app`），调试 `.app` 体积不作为发布体积结论。

## Tag 发布流水线

`node scripts/check-release-version.mjs [tag]`：比对 `package.json`、`src-tauri/tauri.conf.json` 与 `src-tauri/Cargo.toml` 的 `[workspace.package]` 版本。不带参数时只要求三处一致；带 tag（如 `v0.2.0`）时额外要求版本等于 tag。不一致或非语义化版本一律非零退出，并列出全部来源，避免改错文件。该检查已进入 `npm run verify`，PR 阶段即可发现版本漂移；CI 在打包前再带 tag 跑一次，防止 `v0.2.0` 打出自称 `0.1.0` 的安装包。

`.github/workflows/release.yml` 在推送 `v*` tag 时触发，也可通过 `workflow_dispatch` 指定已有 tag 重建：

1. `verify` job 在 macOS runner 上先跑 `node scripts/check-release-version.mjs "$RELEASE_TAG"`，再跑 `bash scripts/verify.sh --skip-install`，作为所有打包的前置门禁；不复制 verify 内部命令。
2. `build` job 以矩阵并行构建 macOS universal（`universal-apple-darwin`）、Windows x64（`x86_64-pc-windows-msvc`）与 Linux x64（`x86_64-unknown-linux-gnu`）；`fail-fast: false`，单个平台失败不取消其余平台。
3. `tauri-apps/tauri-action` 负责创建/复用该 tag 的 Release 并上传产物，全部第三方 action 按提交 SHA 锁定；Release 说明里显式声明产物未签名、未公证。

发布边界：CI 不注入 `APPLE_SIGNING_IDENTITY`、`APPLE_NOTARY_PROFILE` 或任何 Tauri 签名私钥，签名与公证仍未接入流水线；应用未集成 updater 插件，因此 `includeUpdaterJson: false`，不发布无人消费的更新清单。Linux aarch64 不在本矩阵内。

完整环境说明见 [macOS 开发环境](../docs/07-project-setup-guide.md)，证据见 [验证记录](../docs/09-macos-validation.md)。