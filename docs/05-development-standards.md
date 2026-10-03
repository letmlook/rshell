# 开发规范

更新：2026-10-02（体检修复轮），适用于当前 Rust + Tauri + Vue 实现。

先复现，再添加回归测试，最后作最小修复。不扩展协议、平台或界面范围。可见操作必须真实执行或明确禁用/移除，不使用空回调或错误时切换假数据。

Rust API 和 TypeScript 类型一起维护。命令修改核对 Tauri 注册、客户端参数和结果；事件修改核对外部标签序列化及监听释放。

IPC 参数键契约：`src-tauri/src/commands.rs` 全部 `#[tauri::command]`（含 `cmd!` 宏模板）声明 `rename_all = "snake_case"`，前端 invoke 参数键与 Rust 形参名一致；Tauri 2 默认是 camelCase，不得裸加命令属性。`tests/unit/ipcContract.spec.ts` 对两侧做双向对账，新增命令必须同步两侧。

## 并发与错误

- 克隆句柄后释放注册表锁，再等待网络或 reply。
- 注意 if-let 临时锁 guard 的作用域。
- 后台任务只清理自己拥有的连接，旧任务不覆盖新任务。
- 取消 future 不得丢失阻塞读取结果。
- 持久化失败返回错误，不先更新内存并宣告成功。
- 不记录密码、私钥或可能包含凭据的脚本正文。

文件删除需要明确目标、确认与后端路径校验。保留无法迁移的旧配置，不静默改成默认协议。不用 shell 命令替代受控文件 API。

## 质量门

根目录执行 `npm run typecheck`、`npm test`、`npm run test:scripts`、`npm run build`、`npm run check:docs`。在 `src-tauri/` 执行 `cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`。

测试使用临时目录和回环服务。忽略测试、环境限制和构建警告如实记录，不省略后称全量通过。

Git 提交使用 `letmlook <letmlook@aliyun.com>`，不提交构建产物、认证数据或本地工具状态。
