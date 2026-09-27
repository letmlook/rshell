# 贡献指南

本轮优先保证 macOS 上现有能力可靠。提交前阅读 [详细设计](docs/03-detailed-design.md) 和 [功能边界](docs/08-incomplete-features.md)。

## 开发流程

1. 从明确的问题和可复现行为开始。
2. 修改最小必要范围；协议、业务、Tauri 薄壳和 Vue 界面各自保持职责。
3. 修复用户可观察的错误，并补充能暴露该错误的测试。
4. 运行相关检查，再运行交付质量门禁。
5. 同步更新受影响的用户文档和验证记录。

不要新增假数据、空回调、忽略错误的成功提示或无实际作用的控件。删除、覆盖、密钥信任等操作必须有清晰的目标和错误反馈。

## 环境与检查

所有本地校验都走仓库内共享脚本；命令细节见 [scripts/README.md](scripts/README.md)，不要在文档或 PR 描述里复制脚本内部命令。

```bash
npm run verify   # scripts/verify.sh：前端类型检查/测试/构建/文档/脚本测试 + Rust fmt/clippy/test
npm run audit    # scripts/audit.sh：npm audit (官方 registry) + cargo audit (Cargo.lock)
```

快速复跑可用 `bash scripts/verify.sh --skip-install` 跳过 `npm ci`。

桌面调试和打包命令分别为 `npm run tauri:dev`、`npm run tauri:build`。macOS 调试 `.app` 的预检用 `bash scripts/macos-release-preflight.sh --unsigned <app-path>`，已构建产物的合规检查用 `bash scripts/macos-verify-app.sh <app-path>`。详情见 [环境指南](docs/07-project-setup-guide.md)。

## 代码约定

- Rust 使用 rustfmt；错误传递到调用者，避免静默吞掉网络或持久化失败。
- Vue 使用 TypeScript，优先复用现有组件和样式 token。
- 共享字段改动必须同时检查 Rust API、TypeScript 类型、Tauri handler 和调用方。
- 不在持锁等待远端输入时阻塞其他会话操作。
- 原始终端输出使用 Channel；低频状态变化使用 AppEvent。
- 事件订阅、窗口监听器和后台连接任务应有明确的清理路径。
- 测试数据使用临时目录和本地回环服务，不依赖真实用户文件或凭据。

## 提交

所有 Git 提交使用 `letmlook <letmlook@aliyun.com>`。保持提交可审查，不提交密钥、会话密码、构建产物或本地数据目录。提交说明包含行为变化和实际运行的验证；不要把未执行的集成测试写为通过。

历史设计保留在 Git 历史和带日期的设计文件中。修改当前实现时，以当前文档为准。
