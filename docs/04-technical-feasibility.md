# 技术选型与验证边界

更新：2026-09-26。本文描述仓库实际使用的技术，不把设计设想当作已验证成果。

Tauri 2 提供 macOS WebView 壳与 IPC；Vue 3/TypeScript 提供界面；xterm.js 渲染终端；Rust/Tokio 承担业务并发。SSH/SFTP 使用现有 russh 系列库，串口使用 serialport，脚本使用 Rhai，WASM 使用 wasmtime。

编译通过只证明接口和依赖可以共同编译，不证明任何服务端、串口驱动或插件都能运行。

## 能力边界

- SSH/SFTP、Telnet、Serial 保留，Telnet 仍是明文通信。
- Local/Dynamic 隧道保留，不为 Remote Forward 替换 SSH 库。
- RDP 已删除，不引入替代远程桌面实现。
- WASM 当前验证发现、编译、加载与卸载，不承诺完整宿主生态。
- Windows 不属于本轮运行验证。

## 风险与实证

自动检查、原生进程启动和 GUI 操作属于不同证据层级，详见 [验证记录](09-macos-validation.md)。

发布前需要真实服务器上的 SSH/SFTP、隧道、认证与断连验证；物理串口验证；macOS 窗口、剪贴板、权限体验；发布包签名、公证和安装检查。不能从单元测试推导吞吐量、长期稳定性或安全审计结论。
