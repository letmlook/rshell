# 测试策略

更新：2026-09-26。

## 自动化

前端通过 Vitest/Vue Test Utils 验证 IPC、文件操作条件、确认、错误反馈、主题和事件清理。TypeScript 类型检查与 Vite 构建分别检查契约和产物。

Rust 测试覆盖协议类型、会话与配置持久化、路径守卫、隧道兼容、主机密钥、自动化、主题和 WASM。回环 TCP 测试验证 Telnet 协商与 I/O；串口取消测试使用受控读结果，不冒充硬件测试。

## 回归重点

- RDP 旧输入拒绝，旧 Remote 规则不启动且原文件保留。
- 文件操作的无会话、无选择、根路径和目录删除守卫。
- SSH shell 与 SFTP/转发数据隔离，远端关闭清理状态。
- 连接取消/删除后的迟到结果不复活会话。
- 协议读结束与请求竞争不死锁，取消等待不丢字节。
- 触发器错误可见，快速命令和触发器重启恢复。
- 主题和事件反复挂载不泄漏，新终端使用当前配色。
- 损坏配置、无效 WASM 不以成功状态呈现。

## 运行

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

本机 stable 自动更新异常时，Rust 命令前可设置 `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1`。

## macOS 验收

用 `npm run tauri:dev` 启动。GUI、真实 SSH/SFTP、隧道、物理串口分别按 [验收记录](09-macos-validation.md) 检查。浏览器 mock、jsdom、进程存活不能代替桌面交互验收。发布前另查签名、公证与安装。

最新测试数和结果统一记在验收记录中，避免重复维护而漂移。
