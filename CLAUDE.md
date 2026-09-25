# RShell 开发上下文

更新：2026-09-26。当前实现是 Tauri 2 + Vue 3 + TypeScript + Rust，优先加固 macOS 已有功能。

## 范围

保留 SSH/SFTP、Telnet、Serial、终端、会话、密钥与主密码、Local/Dynamic 隧道、快速命令、触发器、Rhai、主题和本地 WASM 模块加载。RDP 已删除，Remote Forward 不属于当前支持契约。不要根据历史规划重新增加这些功能或扩大 Windows 工作范围。

## 边界

前端只通过 `src/ipc/client.ts` 和终端 Channel 调用后端。Tauri 命令在 `src-tauri/src/commands.rs` 注册，并在 `lib.rs` 的 handler 列表中公开。命令名称遵循 PascalCase 到 snake_case 映射。

`rshell-api` 定义 AppCommand、AppEvent、CommandOutcome。Rust 的 Serde 外部标签枚举必须与 TypeScript 一致：无负载事件是字符串，有负载事件是单键对象。列表命令直接返回数据，不通过列表事件回应列表请求。

业务逻辑归 `rshell-core`；协议 I/O 归 `rshell-protocol`；Tauri 层只做初始化、参数转换、错误映射和路由。xterm.js 负责终端渲染。

## 生命周期

SSH 输出接收器独立于发送客户端锁。Telnet/Serial 由会话服务持有连接任务，通过请求通道接收发送、resize 和断开操作。不要在等待远端输入时阻塞发送路径。

Tauri 初始化期间不能嵌套调用运行时的 block_on。初始化参数应同步注入；后台任务使用 Tauri 提供的运行时入口。组件和 store 的事件订阅必须释放，并处理异步订阅完成晚于 unmount 的情况。

主机密钥决定来自用户；永久信任保存成功后才接受握手。不要记录私钥、密码或脚本中可能含有的凭据。

## 命令

仓库根目录：
```bash
npm ci
npm run typecheck
npm test
npm run build
npm run tauri:dev
```

Rust 工作区：
```bash
cd src-tauri
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

本机 stable 自动更新冲突时，为命令设置 `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1`，不要把工具链故障误报为项目编译失败。

## 验收

可见按钮必须调用真实操作或明确禁用，不能静默成功、使用假数据回退或发布并未发生的 Connected/Active 状态。文件删除必须确认并限定目标；SFTP 写操作只接受绝对、非根、无歧义路径。

文档以 [验证记录](docs/09-macos-validation.md) 和代码为准。外部 SSH 服务、物理串口、签名与公证未验证时，应明确标记。
