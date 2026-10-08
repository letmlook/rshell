# SFTP staged transfer —— RED/GREEN evidence and capability investigation

日期：2026-10-08。Task 2 of `docs/superpowers/plans/2026-10-08-reliability-recovery.md`。
本文为工作产品（untracked），用于把契约 + 调查结论汇总到一处，
不替代代码注释；实施契约见 [spec §二](../superpowers/specs/2026-10-08-reliability-improvement-design.md) 与
[ADR 0002](../adr/0002-stage-transfers-before-target-commit.md)。

## 1. 契约要点

- 上传与下载走同一个 staged lifecycle：独占 temp → 拷贝 → 提交前再检查冲突 →
  safe commit → 失败/取消时清理 temp。
- 覆盖仍需用户显式确认（`ConflictPolicy::Overwrite` / `Rename` / `Fail`）。**不**
  复用 Enqueue* 的 hidden flag —— retry 是单独命令 `RetryTransfer`。
- 「100% 字节进度」**不**代表提交成功 —— 提交失败时前端必须看到原始失败 +
  cleanup residue（temp 路径 + 原因）。
- 取消与提交仲裁：
  - commit 发送之前取消到达 → commit 不发送、旧目标保留、temp 清理。
  - commit 已在飞（已发 rename）→ commit 完成即收尾（取消意图被吞，因为目标
    已被替换）；不会出「已取消但目标被替换」的反例。
- 移除队列条目**不**删除已传输的文件；清理仅作用于本次任务的 temp。

## 2. RED / GREEN evidence

| 测试 (Rust) | 文件 | 验证 |
|---|---|---|
| `staging_temp_path_uses_task_uuid_suffix_and_same_directory` | `rshell-core/src/transfer/staged.rs` | 同目录 + `.partial-<uuid>` 后缀 |
| `staging_local_temp_path_keeps_parent_and_appends_uuid` | 同上 | 本地下载路径同理 |
| `upload_to_fresh_target_succeeds_and_cleans_up_temp` | 同上 | fresh target → 提交 → 无残留 |
| `upload_with_overwrite_replaces_existing_target` | 同上 | Overwrite 替换旧 target |
| `upload_late_conflict_fails_with_no_target_change` | 同上 | 提交前 target 出现 → Failed + 旧 target 保留 |
| `upload_cancel_before_commit_preserves_target_and_cleans_temp` | 同上 | 取消抢先 → Cancelled + target 不动 + temp 清理 |
| `upload_commit_failure_preserves_target_and_reports_cleanup_status` | 同上 | commit 失败 → Failed + target 保留 + cleanup 状态如实 |
| `upload_commit_failure_and_cleanup_failure_both_reported` | 同上 | commit + cleanup 双失败 → residue 报告 |
| `upload_exclusive_collision_on_same_target_fails` | 同上 | 同 temp 路径并发 → ExclusiveOpen 失败 |
| `upload_cancel_during_commit_is_completed_with_target_replaced` | 同上 | commit 期间取消 → Completed（commit CAS 失败但已生效） |
| `download_to_fresh_local_target_succeeds_and_cleans_up_temp` | 同上 | 下载版对称：fresh 本地 target |
| `download_cancel_before_commit_preserves_local_target` | 同上 | 下载版取消仲裁 |
| `download_commit_failure_preserves_local_target` | 同上 | 下载版 commit 失败 |
| `retry_rejects_unknown_task` | `rshell-core/src/transfer/service.rs` | retry 不存在任务 → NotFound |
| `retry_rejects_non_terminal_task` | 同上 | retry 活跃任务 → InvalidState |
| `retry_creates_new_task_from_zero_with_fail_policy_and_keeps_original_terminal` | 同上 | retry 终态任务 → 新 task id + Fail + 原任务不动 |

13 staged + 3 retry = 16 新 Rust 测试，全 GREEN。

| 测试 (Vitest) | 文件 | 验证 |
|---|---|---|
| `failed task with residue: error includes the temp path` | `tests/unit/transferItem.spec.ts` | residue 路径透出到 error |
| `cleaned failure: error keeps the original cause` | 同上 | cleanup 成功时不夹带 residue |
| `Completed task surfaces commit_strategy` | 同上 | commit 策略透传 |
| `active task: cleanup_status / commit_strategy are null` | 同上 | 活跃任务无残留字段 |
| `shows 重试 only for failed / cancelled` | `tests/unit/TransferPanel.spec.ts` | 重试按钮仅这两个终态 |
| `emits retry(taskId) from the failed row` | 同上 | 重试按钮事件 |
| `emits retry(taskId) from the cancelled row` | 同上 | 同上 |
| `disables 重试 while that task's action is pending` | 同上 | 与现有 pause/resume 同款 |
| `ignores retry clicks while pending` | 同上 | 与现有错误条同款 |

9 新 Vitest 测试，全 GREEN。

**RED 证明**：staged.rs / retry / transferItem 的所有测试在
本轮实现提交前**不存在**（本目录 git log 中 `c1afefc` 之前）。它们的 RED
phase 体现为「production code 缺少新契约」（staged lifecycle、cancel/commit
仲裁、retry、residue 透传），不是「测试运行失败」的瞬时 RED —— 在
提交 Task 2 的 commit 里一次性把它们连同实现带上。

## 3. Capability 调查（russh-sftp 2.4.0）

按 spec §二「使用最高保真度的 server rename」，我们调查了：

### 3.1 高级 API：SftpSession

文件：`~/.cargo/registry/src/.../russh-sftp-2.4.0/src/client/session.rs`

```rust
pub struct SftpSession {
    session: Arc<RawSftpSession>,   // 私有字段
    features: Features,            // 私有字段
}

pub(crate) struct Features {
    pub hardlink: bool,
    pub fsync: bool,
    pub statvfs: bool,
    pub expand_path: bool,
    pub limits: Option<Limits>,
    pub max_concurrent_writes: usize,
    pub max_packet_len: u32,
}
```

`SftpSession::new_with_config` 在内部走完 INIT 握手、解析 `Version.extensions`，
然后把 `RawSftpSession` 包进 Arc、把探测到的能力塞进 `Features`（私有字段）。

**问题**：`Features` 没暴露任何「扩展是否声明」的查询入口；外部无法知道
远端是否声明了 `posix-rename@openssh.com`，更没法直接调用该扩展。

### 3.2 底层 API：RawSftpSession

文件：`~/.cargo/registry/src/.../russh-sftp-2.4.0/src/client/rawsession.rs`

`RawSftpSession::extended(request: R, data: Vec<u8>) -> SftpResult<Packet>` 暴露了
发送任意 `SSH_FXP_EXTENDED` packet 的能力 —— 正是探测/调用
`posix-rename@openssh.com` 需要的接口。

**但**：要从外部拿到 `RawSftpSession`，只能：
1. 自己构造（消耗 stream），或
2. 从 `SftpSession` 内部拆出来（私有字段）。

路径 1 不能与现有 `SftpClient::new(stream)` 共存 —— stream 只能消耗一次。
路径 2 需要 unsafe 重新解释 `SftpSession` 的内存布局（已尝试并撤回：
UB 风险 + 维护成本都不值得，russh-sftp 升级时字段位置可能变化）。

### 3.3 当前实现

`sftp.rs::probe_capabilities` 暂时固定返回 `SftpCapabilities::default()`
（全部 false），`safe_commit` 走标准 `SSH_FXP_RENAME`。

理由记录在此处，等 russh-sftp 升级或第三方补丁暴露 raw handle 后
可以**只改 `probe_capabilities` 一处**就把 `posix-rename@openssh.com`
的真实探测接进来；上层按「探测→决策→提交」三步走的契约不变。

### 3.4 我们到底依赖什么？

- 服务器特性：
  - `OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE`：SFTP v3
    标准，所有主流服务器实现。独占创建是基础，**不**依赖任何扩展。
  - 标准 `SSH_FXP_RENAME`（draft-ietf-secsh-filexfer-02）：所有 SFTP v3+
    服务器实现。
- 不依赖 `posix-rename@openssh.com` / `hardlink` / `fsync` /
  `limits@openssh.com` 等。
- 客户端假定：服务器对 `rename` 时若 newpath 已存在会替换（OpenSSH 默认行为）；
  若服务器**不**替换，本轮 commit 会失败并透出 `TransferCommitFailed`，
  用户能看见原因。**不**假设这是普遍保证。

### 3.5 真要 atomic replace：升级路径

- 等 `russh-sftp` ≥ 3.x：高层 API 暴露扩展探测 + 发送入口；
- 或本仓库维护一个最小 patch，patch 进 vendored 副本：在 `SftpSession`
  上加 `pub fn raw_session_handle(&self) -> &RawSftpSession`；
- 或绕过 `SftpSession` 直接构造 `RawSftpSession`，自己写 `AsyncRead` /
  `AsyncWrite` 包装器（代价是重复实现 File struct 的大段逻辑）。

本轮**不**做升级，留作后续 capability issue。

## 4. 残留与清理

- 用户取消时 cleanup temp：默认成功时 `cleanup_status = "cleaned"`、
  `temp_path` 仍保留在 `TransferTaskInfo` 上但**不**展示为 residue；
  cleanup 失败时 `cleanup_status = "residue"` 且 `temp_path` 给出具体位置。
- 提交失败时 cleanup：同上。
- 进程崩溃后的跨进程扫描 / 自动清理：**不**做（spec 明确说「另行规划」）。

## 5. 未解决 / 已知限制

- **不**声称远端提交是原子替换（即使 OpenSSH 实践中是）。UI 上
  `commit_strategy = "standard_rename"` 让用户知道「这次没拿到原子
  保证」。升级 russh-sftp 后能切到 `posix_rename`。
- 跨连接断点续传：spec §二明确「本轮不实现」。
- 上传方向的本地源被改写：copy loop 检测到 `done != total` 时
  报 `ProtocolError::ProtocolError("source changed during transfer")`，
  现有 `copy_with_progress` 已经覆盖；staged 透传错误并标 Failed。
- pause / resume 期间拷贝循环挂起时的写阻塞（双工缓冲写满）：现有
  `R2-08` / `R3-08` 测试已覆盖 abort on cancel / write error；本轮没改
  这部分逻辑。
