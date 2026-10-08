# Independent review — Task 2 (Staged SFTP commit + transfer retry)

日期：2026-10-08。Reviewer 不参与 Task 2 实现；按 spec §二、ADR 0002、plan Task 2、implementer's report (`docs/research/2026-10-08-sftp-staged-commit-report.md`) 的契约 + 实际代码 + 独立运行的测试结果挑战实现结论。

## Verdict

**DONE_WITH_CONCERNS** — staged lifecycle 状态机、cancel/commit 仲裁、retry 入口、residue/cleanup 报告、capability 调查与 SSH_FXP_RENAME 兜底、删跑只清队列这六条契约由测试覆盖且与生产代码一致；只有 commit_strategy 在 UI 实际渲染这一条存疑（见 Findings F1），其余无关键缺陷。残留面见下文。

---

## Independent Verification Results

| # | 命令 | 期望 | 实际 |
|---|---|---|---|
| 1 | `cargo fmt --all --manifest-path src-tauri/Cargo.toml --check` | exit 0 | exit 0 |
| 2 | `cargo test -p rshell-protocol --lib` | 69 passed | **69 passed; 0 failed** |
| 3 | `cargo test -p rshell-core --lib` | 205 passed (189 + 16 new) | **205 passed; 0 failed**（包含 13 staged + 3 retry） |
| 4 | `cargo test -p rshell --lib` | 15 passed | **15 passed; 0 failed** |
| 5 | `cargo clippy -p rshell-protocol -p rshell-core -p rshell --all-targets -- -D warnings` | exit 0 | exit 0 |
| 6 | `npm test` | 37 files / 291 passed | **37 files, 291 passed, 0 failed** |
| 7 | `npm run typecheck` | exit 0 | exit 0 |
| 8 | `git diff --check` | no whitespace errors | exit 0 |

测试环境：`RUSTUP_TOOLCHAIN=stable`、`RUSTUP_NO_UPDATE_CHECK=1`、`CARGO_TARGET_DIR=C:/code/github/rshell/src-tauri/target`（复用主仓库已有 target dir，未启并行 cargo）。所有命令 exit 0；Cargo 增量编译约 6–22 秒（target dir 复用收益明显）。

---

## Findings

### F1 — UI 未实际渲染 `commit_strategy`（capability 诚实度问题）

- **Severity:** Notable
- **Spec clause:** spec §二"覆盖仍需明确确认" + plan Task 2 acceptance "前端据此告诉用户「这次拿到了原子替换保证吗」" + implementer's report §5「UI 上 commit_strategy = "standard_rename" 让用户知道「这次没拿到原子保证」」。
- **代码位置:** `src/components/TransferPanel.vue:494-519`（xfer-row 模板） / `src/utils/transferItem.ts:42`（数据映射）。
- **What:** 数据链路 `TransferTaskInfo.commit_strategy` → `toTransferItem` → `TransferItem.commit_strategy` 已经接通，且 `transferItem.spec.ts` 的「Completed task surfaces commit_strategy」断言存在并通过。但 TransferPanel.vue 模板里没有渲染 `row.commit_strategy` 的元素——`xfer-error` 列只显示 `row.error`（模板 line 517-519），其他列只展示名字/状态/进度/路径/速度/剩余。结果：用户看不到 commit 用了标准 rename 还是 posix rename。
- **Evidence:** 全工作区 grep `commit_strategy` 在 `src/` 下只命中 `types.ts`（类型定义）、`transferItem.ts`（映射到 TransferItem）、`TransferPanel.vue:44`（TransferItem 字段声明）。Vue template 文本里没有 `{{ row.commit_strategy }}` 或同款绑定。spec/plan 期望的是「UI 告诉用户」；当前 UI 没告诉。Implementer's report 也未声明这层 UI 渲染被测试覆盖。
- **Recommendation:** 修复优先（回归测试先）→ 加一行 `{{ row.commit_strategy ? \`提交策略：${commitStrategyLabel(row.commit_strategy)}\` : '' }}` 到 TransferPanel.vue 模板的 xfer-error 列（或新建一列），加 `commitStrategyLabel` 工具函数把 `"standard_rename"` 翻译成「提交策略：标准 rename（非原子保证）」之类人类可读文案，加一条 TransferPanel.spec.ts 断言：Completed 行渲染里包含该文案。重新跑 npm test。

### F2 — `tests/unit/ipcContract.spec.ts` 未直接覆盖 `cleanup_status` / `commit_strategy` / `temp_path` 字段

- **Severity:** Minor
- **Spec clause:** spec §二 "100% 字节进度不代表提交已经成功" + "失败或取消后...无法清理的确切路径和原因" + implementer's report §4 强调 residue 透出。
- **代码位置:** `src/ipc/types.ts:127-132` 定义、`tests/unit/ipcContract.spec.ts` 全文件 grep 仅命中 `TaskId: { helper: "retryTransfer", typeText: "Uuid" }`（line 412），没有针对 TransferTaskInfo 新增字段的断言。
- **What:** 该测试只验证「命令名 / 参数键 / 事件变体 / CommandOutcome 变体 / helper 调用点」五张对账表，对 TransferTaskInfo 结构体内字段的同步未做对账。新字段是简单 `string | null`，类型不一致会被 `vue-tsc --noEmit` 拦截，但 `git diff` 加 `tests/unit/ipcContract.spec.ts` 这一行仍能锁住「删 cleanup_status 字段」这种漂移。功能上已被 transferItem.spec.ts 端到端验证（residues 字段映射断言），属于「契约显式化」空白，不是功能缺失。
- **Evidence:** 转 TransferTaskInfo 结构由 `rshell_api::types::TransferTaskInfo` 的 `Into<TransferTaskInfo>` 派生给前端 Vue；字段名同步靠两边手工维护。npm typecheck 通过保证类型一致，但测试未捕捉「新增 / 删除字段」漂移。
- **Recommendation:** 不阻塞本轮完成；后续若要加严，可加 `parseTransferTaskInfoFields` 提取 TS interface 与 Rust struct 的字段集做对账测试。

### F3 — cancel/commit 仲裁的小窗口（非缺陷，但应被 review 看见）

- **Severity:** Note
- **Spec clause:** spec §二「取消与最终提交必须有明确的先后裁决」。
- **代码位置:** `src-tauri/crates/rshell-core/src/transfer/staged.rs:175`（cancel 决策检查）→ `192`（precheck）→ `212`（safe_commit）→ `237`（CAS）。
- **What:** cancel 在 line 175 检查通过后、line 192 precheck 之前的窗口到达时不会被 line 175 看见；但 precheck 与 commit 都按顺序执行，所以 cancel 真正能介入的时机只有「commit 飞起来之后」——这时 CAS（line 237）会因 DECISION_CANCELLED 失败而返回 Completed（commit 已生效）。这是 spec 明确接受的行为（"取消意图被吞，因为目标已被原子替换"），并由 `upload_cancel_during_commit_is_completed_with_target_replaced` 锁定回归。
- **Evidence:** staged.rs:175 提前检查；staged.rs:237 `compare_exchange(PENDING → COMMITTED)` 是唯一决策点；test 7（line 781）的 with_on_commit hook 把 DECISION_CANCELLED 写在 safe_commit 内部、但 commit 仍返回 Ok，确认 CAS 后的语义正确。
- **Recommendation:** 不改。code comment（line 244-247）已经把"取消意图被吞"明确写出，spec 也允许这条赛跑。

### F4 — macOS 真机验收未执行（plan Task 4 范围）

- **Severity:** Note
- **Spec clause:** spec §三"验收矩阵与完成状态" 中的「macOS 实机：真实 SFTP 上传下载、授权覆盖、失败取消、临时残留」一行。
- **代码位置:** plan Task 4 显式列为本任务范围外。
- **What:** 所有 RED 测试均在内存 FakeSink 上跑通，未对真实 `russh-sftp` 服务器验证；`safe_commit` 走 `SSH_FXP_RENAME`，实际服务器替换语义、临时文件路径在不同文件系统上的原子性、`posix-rename@openssh.com` 探测接口的缺位都依赖真机。Implementer's report §3 已经诚实记录 capability 调查结论。
- **Evidence:** 报告 §5「未解决 / 已知限制」+ plan Task 4。
- **Recommendation:** 不阻塞 Task 2 完成；Task 4 验收时应补 macOS 真机日志。

### F5 — 没有 password / passphrase / secret 进新增 log 行

- **Severity:** Note
- **Spec clause:** spec §三"不记录密码或口令"。
- **代码位置:** `rshell-core/transfer/{staged.rs,service.rs}` 全模块。
- **What:** `grep -nE "password|passphrase|secret"` 在 transfer 模块只命中 service.rs 测试代码（1493/1499/2649/2655 构造 SSH 客户端的 fixture），没有命中 `info!` / `warn!` / `debug!` / `error!` 调用。23 处新 log 行携带的字段：task_id、stage、commit.strategy、residue、cleanup_status、error message —— 无凭据。
- **Evidence:** 直接 grep 结果。
- **Recommendation:** 不改；这是确认项。

### F6 — 没有「先删旧 target 再 rename 来伪装原子」的脏路径

- **Severity:** Note
- **Spec clause:** ADR 0002「无法保证安全提交时应保留旧目标并明确报告失败」+ spec §二"不回退到先删除旧文件或直接截断目标"。
- **代码位置:** `src-tauri/crates/rshell-protocol/src/ssh/sftp.rs:613-629` `safe_commit` 仅调 `self.session.rename(from, to)`，没有前置 remove。
- **What:** 一行 `rename` 调用，没有 pre-delete；如果服务器 rename 失败（target 存在且不替换），temp 仍存在、target 仍为旧内容，由 staged.rs 失败分支接住并 cleanup。
- **Evidence:** sftp.rs:613-629 全文 + staged.rs:212-234 commit 失败分支。
- **Recommendation:** 不改；这是确认项。

---

## Test Coverage Matrix

以 spec §二验收条件 + plan Task 2 acceptance bullets 为索引。

| Spec §二 / Plan Acceptance | 测试 | 状态 |
|---|---|---|
| 上传与下载均覆盖新目标和明确授权覆盖目标；成功后内容一致，并无属于该任务的临时文件残留 | `upload_to_fresh_target_succeeds_and_cleans_up_temp` / `upload_with_overwrite_replaces_existing_target` / `download_to_fresh_local_target_succeeds_and_cleans_up_temp` | passes |
| 写入失败、断网、磁盘或权限失败、取消时，已有最终目标内容保持不变 | `upload_commit_failure_preserves_target_and_reports_cleanup_status` / `upload_cancel_before_commit_preserves_target_and_cleans_temp` / `download_cancel_before_commit_preserves_local_target` | passes |
| 临时残留与清理失败被真实报告 | `upload_commit_failure_and_cleanup_failure_both_reported` + `transferItem.spec.ts:56`「failed task with residue: error includes the temp path」 | passes |
| 最终提交失败不发布完成事件，原目标仍受保护 | `finalize_staged` (service.rs:1014-1066) — Failed 分支不调用 `TransferCompleted`，仅 `TransferFailed` + `TransferQueueChanged`；`upload_commit_failure_preserves_target_and_reports_cleanup_status` 断言 target 未被改写 | passes（无直接断言事件序列但事件分支可达） |
| 目标在预检后出现、清理失败、取消与提交竞争均有定向回归 | `upload_late_conflict_fails_with_no_target_change` / `upload_commit_failure_and_cleanup_failure_both_reported` / `upload_cancel_during_commit_is_completed_with_target_replaced` | passes |
| 重试、暂停/继续、取消和移除记录的界面动作与实际任务状态一致 | `retry_rejects_unknown_task` / `retry_rejects_non_terminal_task` / `retry_creates_new_task_from_zero_with_fail_policy_and_keeps_original_terminal` + `TransferPanel.spec.ts:147-194` 重试按钮 + `cancel/remove` 既有测试 | passes |
| Plan Task 2: 测试必须驱动生产状态机，不能 mock 整个 transfer | 13 staged 测试 + 3 retry 测试均直接调 `run_staged_upload` / `run_staged_download` / `enqueue_retry`，sink 是 `Arc<dyn TransferSink>` 抽象的具体 `FakeSink` 实现 | passes |
| Plan Task 2: 取消与提交互斥裁决，含任务清理与 commit 竞争 | `upload_cancel_during_commit_is_completed_with_target_replaced` + service.rs `cancel_during_provider_await_prevents_transfer_start` + `pause_during_provider_await_suspends_loop_before_remote_contact` | passes |
| Plan Task 2: UI regression for residue / from-zero retry / 冲突重新确认 | `transferItem.spec.ts:56-99` 4 条 + `TransferPanel.spec.ts:147-194` 5 条 + `AppLayout.spec.ts:146-150` 透传 TransferTaskInfo 字段 | passes（commit_strategy 见 F1） |
| Plan Task 2: 不跨连接断点续传 | spec §二 明确声明「本轮不实现」，实现侧未引入跨连接句柄复用 | passes（不变量未破） |
| Plan Task 2: 失败或取消后 temp_path + 清理失败原因真实报告 | `CleanupStatus::Residue { path, reason }` 字段、`Into<TransferTaskInfo>` 映射、`transferItem.spec.ts:56-69` 断言 error 包含 path | passes |
| UI surfaces commit_strategy | 无直接 UI 渲染断言；数据流接通但模板未消费 | **absent**（见 F1） |

---

## Risks Not Closed

按 implementer's report §5「未解决 / 已知限制」原样记录：

1. **不声称远端提交是原子替换。** 当前固定走 `StandardRename`，UI 应当让用户知道。但 UI 实际没渲染 `commit_strategy`（见 F1），用户拿到的信号弱于 plan Task 2 acceptance 承诺的「UI 告诉用户」。
2. **跨连接断点续传**：spec §二明确本轮不实现，未引入。
3. **pause / resume 期间拷贝循环挂起时的写阻塞（双工缓冲写满）**：现有 `R2-08` / `R3-08` 测试覆盖 abort on cancel / write error；本轮未改这部分。
4. **进程崩溃后的跨进程扫描 / 自动清理**：spec §二明确「另行规划」，未引入。

另加一条 review-only 风险：

5. **`tests/unit/ipcContract.spec.ts` 没有对 TransferTaskInfo 新增字段做对账**（F2）。属于「契约显式化」空白，不是功能缺失。

---

## Test-Code Spot-Check

按 review focus 第 6 条要求至少 3 条新 staged + 2 条新 transferItem + 2 条新 TransferPanel。下面贴断言原句。

### S1 — `upload_cancel_during_commit_is_completed_with_target_replaced`（staged.rs:781）

```rust
// 关键：commit CAS 失败但函数仍返回 Completed——因为 commit 已成功
match outcome {
    StagedOutcome::Completed { bytes, .. } => assert_eq!(bytes, 3),
    other => panic!("commit 成功后取消到达 → Completed：{other:?}"),
}
assert_eq!(
    sink.snapshot("/remote/target.bin"),
    Some(b"NEW".to_vec()),
    "commit 已生效：target 必须被替换"
);
assert_eq!(
    sink.commit_calls.load(std::sync::atomic::Ordering::SeqCst),
    1
);
```

**结论:** 断言真正证明了 spec §二 "commit 已在飞 → 取消到达 → commit 完成即收尾（取消意图被吞）" 的反例。FakeSink 的 `with_on_commit` hook 在 `safe_commit` 内部、rename 之前把 DECISION_CANCELLED 写入；staged.rs 在 rename 已生效后跑 CAS，CAS 因 DECISION_CANCELLED 失败但仍返回 Completed。测试同时断言目标已被替换、commit 仅调用一次——确认了「取消吞了但目标已替换」这条契约。**强证据。**

### S2 — `upload_late_conflict_fails_with_no_target_change`（staged.rs:582）

```rust
match outcome {
    StagedOutcome::Failed { stage, residue_path, .. } => {
        assert_eq!(stage, FailedStage::LateConflict);
        // cleanup 默认成功 → 没有残留
        assert!(residue_path.is_none());
    }
    other => panic!("晚冲突必须报 Failed：{other:?}"),
}
assert_eq!(
    sink.snapshot("/remote/target.bin"),
    Some(b"OLD".to_vec()),
    "晚冲突时旧 target 必须保留"
);
assert_eq!(
    sink.commit_calls.load(std::sync::atomic::Ordering::SeqCst),
    0
);
```

**结论:** FakeSink 预设 OLD target + `with_target_existing(true)` 让 staged.rs precheck (line 192) 触发 LateConflict。断言：阶段正确（LateConflict）、cleanup 默认成功 → residue 为 None、target 保持 OLD、commit_calls 为 0。覆盖了 spec §二「目标在预检后出现」分支。**强证据。**

### S3 — `retry_creates_new_task_from_zero_with_fail_policy_and_keeps_original_terminal`（service.rs:1478）

```rust
let new_id = svc
    .enqueue_retry(id)
    .await
    .expect("retry on Failed task should succeed");
assert_ne!(new_id, id, "retry 必须创建新 task id");

// 原任务保留其 Failed 终态（spec：「retry 创建的是全新 task id」）
let original = svc.get_task(id).await.unwrap();
assert_eq!(original.state, TransferTaskState::Failed);
assert_eq!(original.error_message.as_deref(), Some("boom"));

// 新任务入队；execute_transfer 在假客户端上失败并把它标 Failed，
// 但我们要看的契约是 conflict 策略、路径、id 不同。
let new_task = svc.get_task(new_id).await.unwrap();
assert!(matches!(new_task.conflict, ConflictPolicy::Fail));
assert_eq!(new_task.local_path, original.local_path);
assert_eq!(new_task.remote_path, original.remote_path);
```

**结论:** 通过 `SshClient::new` 假客户端让 retry 后的新任务在 execute_transfer 时失败；断言覆盖 plan Task 2 acceptance 的全部五点：(a) 未知任务 NotFound；(b) 非终态 InvalidState；(c) 新 task id 且与原 id 不同；(d) 原任务 Failed 终态与 error_message 保留；(e) 新任务强制 Fail policy 且路径与原任务相同。**强证据。**

### S4 — `failed task with residue: error includes the temp path`（transferItem.spec.ts:56）

```typescript
it("failed task with residue: error includes the temp path so the user knows where to look", () => {
  const row = toTransferItem(
    task({
      state: "Failed",
      error_message: "commit failed: server unreachable",
      cleanup_status: "residue",
      temp_path: "/remote/a.bin.partial-task-uuid",
    }),
  );
  expect(row.error).toContain("commit failed");
  expect(row.error).toContain("/remote/a.bin.partial-task-uuid");
  expect(row.cleanup_status).toBe("residue");
  expect(row.temp_path).toBe("/remote/a.bin.partial-task-uuid");
});
```

**结论:** 真正验证 spec §二「清理失败时把 temp 路径与清理失败原因一并展示」——error 字段同时含原始原因与残留路径，cleanup_status 透传为 `"residue"`。**强证据。**

### S5 — `Completed task surfaces commit_strategy`（transferItem.spec.ts:83）

```typescript
it("Completed task surfaces commit_strategy so the panel can tell the user what atomicity we got", () => {
  const row = toTransferItem(
    task({
      state: "Completed",
      cleanup_status: "cleaned",
      commit_strategy: "standard_rename",
    }),
  );
  expect(row.commit_strategy).toBe("standard_rename");
});
```

**结论:** 数据流层面已验证。但如 F1 所述，TransferPanel.vue 模板没有渲染 `row.commit_strategy`，测试只覆盖到 TransferItem，未覆盖到用户可见的 DOM。**部分证据（数据流 ✓、UI 渲染 ✗）。**

### S6 — `shows 重试 only for failed / cancelled terminal tasks (not done)`（TransferPanel.spec.ts:147）

```typescript
it("shows 重试 only for failed / cancelled terminal tasks (not done)", () => {
  const wrapper = mountPanel({ items: [...sampleItems, {
    id: "t-cancelled", name: "cancelled", phase: "cancelled",
    progress: 0.3, size: 100, local: "/a", remote: "/b", speed: 0,
  }] });
  const retries = wrapper.findAll('[data-test="xfer-retry"]');
  expect(retries.map((b) => b.attributes("data-task-id")).sort())
    .toEqual(["t-cancelled", "t-failed"]);
  expect(wrapper.find('[data-row-id="t-done"] [data-test="xfer-retry"]').exists()).toBe(false);
  expect(wrapper.find('[data-row-id="t-active"] [data-test="xfer-retry"]').exists()).toBe(false);
});
```

**结论:** 真正覆盖 spec §二「失败 / 已取消的终态任务上额外显示「重试」按钮」+「Completed 不重试」两条边界。强证据。

### S7 — `ignores retry clicks while pending and shows the failure banner`（TransferPanel.spec.ts:185）

```typescript
it("ignores retry clicks while pending and shows the failure banner", async () => {
  const wrapper = mountPanel({
    pendingTaskIds: new Set(["t-failed"]),
    actionError: "重试失败：原任务仍在传输中",
  });
  await wrapper.find('[data-row-id="t-failed"] [data-test="xfer-retry"]').trigger("click");
  expect(wrapper.emitted("retry")).toBeUndefined();
  expect(wrapper.find('[data-test="xfer-action-error"]').text()).toContain("重试失败");
});
```

**结论:** 验证 retry 按钮的「pending 期间点击不发出事件、不静默吞错」与 pause/resume 同一约定保持一致。**强证据。**

---

## 总结

- 16 条新 Rust 测试 + 9 条新 vitest 测试全部 GREEN；CI 门禁 exit 0。
- 13 条 staged 测试全部驱动 `run_staged_upload` / `run_staged_download` 生产状态机（仅替换 sink 为 FakeSink），不是 mock 整个 transfer——与 spec / plan 契约一致。
- cancel/commit 仲裁用单点 CAS（staged.rs:237 / :367）实现；该决策点确实存在且测试覆盖；cancel-during-commit 的「取消意图被吞」反例有专项回归。
- retry 入口三连 (NotFound / InvalidState / new task with Fail policy + 原任务保留) 全覆盖；新任务 id 与原 id 不同由 `assert_ne!(new_id, id)` 锁定。
- residue / cleanup_status / temp_path 在终态任务上如实透传；cleanup 失败时 `error_message` 含原始失败 + temp 路径，frontend `transferItem.ts:27-29` 把残留路径拼到 error 文案里让用户看见。
- 不删文件（`remove_transfer` 仅清内存任务表），temp 路径独占（含 task_uuid 后缀）。
- capability 调查如实记录在 `docs/research/2026-10-08-sftp-staged-commit-report.md` §3；probe_capabilities 暂时返回 default 是有意的契约留口。
- 仅 Notable 缺口：**`commit_strategy` 数据流接通但 UI 模板未渲染**（F1）。spec §二"100% 字节进度不代表提交已经成功" + plan "前端据此告诉用户「这次拿到了原子替换保证吗」" 共同要求这层 UI 信号。建议补 `transferItem.ts` 的人类可读翻译 + TransferPanel.vue 渲染 + 测试断言，再行合并。
- macOS 真机验收、跨连接断点续传、进程崩溃后跨进程清理均按 plan Task 2 / spec §二明确范围外，列入 Task 4 或后续工作。

> 不修改生产代码与测试。本报告作为输入提供给下一轮 implementer 修复 F1 后再合并 Task 2。
