# 构建与文档检查脚本

更新：2026-09-26。macOS 的正式入口是根目录的 `npm run tauri:dev` 和 `npm run tauri:build`。

`bash scripts/build.sh [target-triple]` 是打包命令的薄包装：切换到仓库根目录，调用 Tauri 构建，透传可选 target 和错误码。依赖需先用 `npm ci` 安装。默认产物位于 `src-tauri/target/release/bundle/`；指定 target 时位于对应 target 子目录。签名、公证和安装验收并未由此自动完成。

`npm run test:scripts` 检查包装脚本的命令、工作目录、target 和错误透传，不执行完整发布构建。

`npm run check:docs` 检查当前用户文档中的架构、启动命令、移除范围说明和本地链接。带日期的历史设计与执行计划不作为产品手册扫描。

`build.ps1`、`build.cmd`、`read-version.ps1` 是保留的历史 Windows 脚本，本轮未更新或验证，不作为当前构建入口。Windows 不在本轮范围内。

完整环境说明见 [macOS 开发环境](../docs/07-project-setup-guide.md)，证据见 [验证记录](../docs/09-macos-validation.md)。
