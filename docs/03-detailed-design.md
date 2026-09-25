# 当前架构与关键约束

更新：2026-09-26。桌面壳为 Tauri 2，前端为 Vue 3 + TypeScript + Pinia + Dockview，终端由 xterm.js 渲染，业务层使用 Rust/Tokio。

## 分层

- `src/`：组件、状态、样式、IPC 客户端、终端展示。
- `src-tauri/src/`：Tauri 初始化、命令适配、AppEvent 桥和终端 Channel。
- `rshell-api`：AppCommand、AppEvent、CommandOutcome 与共享类型。
- `rshell-core`：会话、传输、终端、隧道、安全、自动化、主题服务。
- `rshell-protocol`：SSH、SFTP、Telnet、Serial 协议 I/O。
- `rshell-infra`：存储、密码学和平台工具。
- `rshell-plugin-sdk`：TOML 清单和 WASM 编译、加载、卸载。

crate 均位于 `src-tauri/crates/`。Vue 不访问协议对象，Tauri 不承担业务规则。

## IPC 与事件

用户操作经 `src/ipc/client.ts` 调用 snake_case Tauri 命令，再进入 dispatcher/core。列表和文件浏览通过命令结果返回。AppEvent 使用 Serde 外部标签：无负载事件是字符串，有负载事件是单键对象。

高频终端字节走 Channel。监听器在组件卸载/store 释放时解除；异步订阅完成晚于卸载时也要清理。

## 连接与文件

SSH shell 输出与 SFTP/转发通道隔离，接收等待不占用发送锁。Telnet/Serial 由后台任务持有连接，通过请求通道发送、resize 和断开。取消、退出、删除都必须清理连接；旧任务不能覆盖新连接状态。串口阻塞读在阻塞任务中运行，取消等待不能丢失已读取字节。

SFTP 写操作校验绝对、非根、无歧义路径；删除只支持普通文件，前端确认不能代替后端校验。上传检查源文件，下载检查目标父目录。本地目录选择后才取得文件访问范围。

Local/Dynamic 隧道建立监听前必须取得 SSH 连接，不得降级为直连代理。历史 Remote 规则不启动，不因载入改写原文件。

## 安全与存储

主机密钥未知或变化时等待用户决定；永久信任保存成功后才继续握手。用户的 known_hosts 和应用自己的 known_hosts 用于验证。

macOS 默认数据位于 `~/Library/Application Support/rshell/`：`sessions/`、`keys/`、`known_hosts`、`tunnels.toml`、`quick-commands.json`、`triggers.json`。自动化配置损坏时保留文件并阻止覆盖。

重要限制：会话 TOML 按 SessionConfig 保存，可能包含明文密码/口令。主密码服务是独立的内存加密服务，并未成为会话存储的加密保险库。不要公开这些文件、私钥或含凭据的自动化。主题选择与传输队列不保证跨启动恢复。

## 自动化与插件

触发器匹配可解析的 UTF-8 输出片段，不保证跨网络分片匹配。动作包括发送、通知、断开和追加日志；失败以类型化事件返回。

Rhai 宿主注入真实会话与快速命令服务，错误传播到调用方。操作数限制不等于完整的不可信脚本安全边界。

插件的 `plugin.toml` 名称必须匹配目录，模块来自 `plugin.wasm`。加载成功表示模块编译并登记，不表示声明的扩展点已执行；当前无动态库插件执行入口。
