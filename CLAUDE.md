# RShell 开发上下文

更新：2026-09-26。当前实现是 Tauri 2 + Vue 3 + TypeScript + Rust，优先加固 macOS 已有功能。

## 范围

保留 SSH/SFTP、Telnet、Serial、终端、会话、密钥与主密码、Local/Dynamic 隧道、快速命令、触发器、Rhai、主题和本地 WASM 模块加载。RDP 已删除，Remote Forward 不属于当前支持契约。不要根据历史规划重新增加这些功能或扩大 Windows 工作范围。

## 边界

前端只通过 `src/ipc/client.ts` 和终端 Channel 调用后端。Tauri 命令在 `src-tauri/src/commands.rs` 注册，并在 `lib.rs` 的 handler 列表中公开。命令名称遵循 PascalCase 到 snake_case 映射。

`rshell-api` 定义 AppCommand、AppEvent、CommandOutcome。Rust 的 Serde 外部标签枚举必须与 TypeScript 一致：无负载事件是字符串，有负载事件是单键对象。列表命令直接返回数据，不通过列表事件回应列表请求。

业务逻辑归 `rshell-core`；协议 I/O 归 `rshell-protocol`；Tauri 层只做初始化、参数转换、错误映射和路由。xterm.js 负责终端渲染。

### 未实现功能清单在哪里

`tests/unit/ipcContract.spec.ts` 的两份白名单就是现成的待办清单：

- `HELPER_WHITELIST`：`src/ipc/client.ts` 里「已导出但 src/ 内零调用方」的 helper，每个都注明了为什么还没接线（例：`trustHostKey` 走 `decide_host_key` 路径，不另开入口）。接通一个就删一条。
- `EVENT_LISTENER_WHITELIST`：后端有发布点、前端却没有消费方的 `AppEvent` 变体。同样是接通一个删一条。

契约测试会在两侧增删变体、helper 改名或漏掉一方时立即失败——改 IPC 契约后必须一起更新它。

「后端已实现但界面无入口」的功能不只在这两份白名单里。审计时按这几类扫：

- **后端枚举的多个变体，界面只接上一个**：触发器的四种动作与两种条件、快速命令的三种作用范围、认证方式的四种（密码 / 托管密钥 / 外部私钥 / 键盘交互）、`AppCommand` 的四种触发条件等。两侧都要能产生。
- **字段声明了但恒为空**：`QuickCommand.group`/`description` 曾恒为 `null`/空串、`ConnectionInfo` 的字节与延迟字段恒为 0、`HostKeyEntry.first_seen`/`last_seen` 每次重启被重置。字段要么真的被填，要么删掉——不能留着一个永远为假的字段。
- **两侧零引用的类型**：Rust 与 TS 同时没有使用方的类型，是在声明不存在的能力。
- **虚假提示**：界面写着但做不到的事（工具栏 title 里的快捷键）、给错原因的提示（scope 早已全局化却还在说「尚未授权」）、只答应不兑现的协议行为（Telnet 回了 `WILL TERMINAL-TYPE` 却不真发终端类型）。

契约测试只覆盖其中一部分：`HELPER_WHITELIST` / `EVENT_LISTENER_WHITELIST` 管「有实现无消费」，`OUTCOME_RETURN_MAP` 管返回值标注，「IPC 命令契约」组管 Rust 命令变体与 TS 变体的逐一对齐。其余几类目前靠人工审计——新增能力时记得同步看一遍。

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

真机用例（不进 CI，需要真实的 multipass Linux 实例）：
```bash
cd src-tauri
RSHELL_REAL_HOST=<ip> RSHELL_REAL_USER=<user> RSHELL_REAL_PASS=<pass> \
RSHELL_REAL_KEY=~/.ssh/id_ed25519 RSHELL_REAL_TELNET_PORT=2323 \
cargo test -p rshell-core --test real_machine -- --ignored --test-threads=1
```
这批用例直接驱动 `CommandDispatcher::dispatch(AppCommand)`——界面上每个可见操作
最终都落到这个入口，前端与后端之间没有第二条路径。改命令契约或服务装配后，
它能替代（不能替代界面呈现的）手动点击验收。

## 验收

可见按钮必须调用真实操作或明确禁用，不能静默成功、使用假数据回退或发布并未发生的 Connected/Active 状态。文件删除必须确认并限定目标；SFTP 写操作只接受绝对、非根、无歧义路径。

文档以 [验证记录](docs/09-macos-validation.md) 和代码为准。外部 SSH 服务、物理串口、签名与公证未验证时，应明确标记。
