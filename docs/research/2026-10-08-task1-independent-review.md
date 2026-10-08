# Task 1 独立审查：SSH 请求有界与终端恢复

- 日期：2026-10-08
- 审查范围：`c1afefc fix: bound SSH requests and require manual terminal recovery`
- 工作树：`C:\Users\lp\.codex\worktrees\reliability-recovery\rshell`
- 审查员：独立审查子代理（无先前上下文）

## Verdict

DONE_WITH_CONCERNS

实现正确覆盖了规格「一」的全部 5 条验收要点，关键回归测试对过期输入隔离、关闭绕开写队列、客户端锁有界、独立连接隔离、一次性持续提示与手动重连等行为都有直接断言；4 项命令（protocol/core/npm/typecheck/clippy）的实际结果与实现报告声明完全一致；剩余风险均不构成阻挡开发完成的缺陷，主要是测试覆盖细节、跨 crate 门禁执行面与 macOS 产品验收。

## Independent Verification Results

下列命令均使用文档要求的 PowerShell 环境变量在工作树根执行（rustfmt 与 Tauri 单元测试作为额外补跑）。

| 命令 | 实现报告声明 | 实际结果 | 一致？ |
|---|---|---|---|
| `cargo test -p rshell-protocol --lib` | 69 passed, exit 0 | 69 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.14s, exit 0 | 是 |
| `cargo test -p rshell-core --lib` | 189 passed, exit 0 | 189 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.33s, exit 0 | 是 |
| `npm test` | 37 files, 279 passed, 3 skipped, exit 0 | Test Files 37 passed (37); Tests 279 passed \| 3 skipped (282); Duration 8.65s, exit 0 | 是 |
| `npm run typecheck` | exit 0 | exit 0 | 是 |
| `cargo clippy -p rshell-protocol -p rshell-core --all-targets -- -D warnings` | exit 0 | exit 0 | 是 |
| `cargo fmt --all --manifest-path src-tauri/Cargo.toml --check` | exit 0（实现报告隐含） | exit 0 | 是 |
| `cargo test -p rshell --lib`（补跑） | 实现报告未声明 | 15 passed（含 `terminal_recovery_has_a_structured_ipc_kind`），exit 0。首次 `cargo test -p rshell --lib --no-run` 因缺 `dist/` 失败（`frontendDist = "../dist"`），先 `npm run build` 后再编译通过；编译期间出现 1 条 Windows 链接器信息 `linker stdout: 正在创建库 ... .lib 和对象 ... .exp`（实现报告已在 Concerns 中提到，非阻断）。 | 实际行为如预期 |
| `cargo clippy -p rshell --lib --all-targets -- -D warnings`（补跑） | 实现报告未声明 | exit 0 | 是 |

注：Tauri 单元测试在实现报告列出的 5 个命令之外，需要 `dist/` 已存在才可编译。该测试在补跑中通过，确认 `terminal_recovery_required` 字符串契约在 IPC 序列化中成立。

## Findings

### Finding 1 — `kind_strings_are_stable` 未包含 `TerminalRecoveryRequired`（Minor）
- Spec 条款：规格「一」用户行为契约 → 「一次输入超时即可看到持续错误和恢复入口；结果不确定时文案不声称未执行」（由 IPC `kind` 触发前端分支）。
- 代码位置：`src-tauri/src/error.rs:137-148`。
- What：`kind_strings_are_stable` 列出 9 个 `IpcErrorKind` 的稳定字符串断言，但跳过新加入的 `TerminalRecoveryRequired`；其稳定字符串 `"terminal_recovery_required"` 仅由 `terminal_recovery_has_a_structured_ipc_kind` 间接覆盖（通过 `json["kind"]`）。
- 证据：当前测试只断言到 `Storage`，新变体的字符串契约缺乏与其它 kind 同级强度的回归覆盖；如未来有人把 `as_str` 改成 `"recovery"` 等其它字符串，旧测试仍全部通过，但前端 `e.kind === "terminal_recovery_required"` 分支会静默失效。
- 建议：在 `kind_strings_are_stable` 末尾补一行 `assert_eq!(IpcErrorKind::TerminalRecoveryRequired.as_str(), "terminal_recovery_required");`。这条建议属于测试硬化而非修复缺陷。

### Finding 2 — `core_error_mapping_is_exhaustive` 测试面与新 kind 脱节（Minor）
- Spec 条款：规格「一」实施约束 → 「沿用现有业务层、协议层和 Tauri 薄壳职责」。
- 代码位置：`src-tauri/src/error.rs:151-186`。
- What：`core_error_mapping_is_exhaustive` 的 `cases` 列表未包含 `CoreError::TerminalRecoveryRequired`，且允许的 5 个稳定 kind 不包含 `"terminal_recovery_required"`。当前测试仍通过（因为 case 未覆盖），但如有人把 `TerminalRecoveryRequired` 加入 cases 数组，会立即失败——证明测试对该映射缺少独立保护。
- 证据：实现报告 `Concerns` 一节已经声明这一点。
- 建议：在 `cases` 加入 `CoreError::TerminalRecoveryRequired`，并把 `"terminal_recovery_required"` 加入允许列表（同时考虑 6 个 kind 是否仍属于「5 个稳定 kind 之一」注释，应更新文档/常量）。

### Finding 3 — Tauri 单元测试不在任务范围门禁内运行（Note）
- Spec 条款：实施计划 Task 1 验收 → `cargo test -p rshell-protocol --lib`、`cargo test -p rshell-core --lib`、`npm test`、`npm run typecheck`。
- 代码位置：`src-tauri/src/error.rs:196-202`。
- What：实现报告列出的 4 项验证命令**不**包含 `cargo test -p rshell --lib` 或 `cargo test --workspace`。新增的 `terminal_recovery_has_a_structured_ipc_kind` 测试只在 Tauri 二进制 crate 内部，因此不在 task-scoped 验证中执行；其结果直到根上 `cargo test --workspace`（即 `scripts/verify.sh`）才会被检查。本次独立审查补跑 `cargo test -p rshell --lib` 后确认通过。
- 证据：见上「Independent Verification Results」补跑。
- 建议：实施报告的 Concerns 已经显式提示该风险；下一任务（Task 4 整体门禁）必须跑 `scripts/verify.sh` 或等效命令，至少执行 `cargo test --workspace`，否则 IPC kind 契约未被任何已执行命令断言。

### Finding 4 — macOS 产品验收未执行（Note）
- Spec 条款：方案文档 §「验收矩阵与完成状态」→ macOS 实机层级。
- What：本次提交的 RED/GREEN 全部在 Windows 环境的 paused-time + 真 loopback SSH 上完成；没有 macOS 实机 SSH、真实对端 zero-window、网络抖动、多标签关闭与晚到事件等真实场景覆盖。
- 证据：实现报告 `Concerns` 自述「No macOS real SSH/network/multi-label acceptance was possible in this Windows environment」。
- 建议：标记为「开发验证完成」，按方案约定与 macOS 实机产品验收分开报告；这是规格允许的两阶段，不构成 Task 1 范围阻塞。

### Finding 5 — External-server zero-window 恢复未复现（Note）
- Spec 条款：规格「一」用户行为契约 → 「重连建立真实可用终端后才恢复输入」。
- What：现有 loopback 测试（`idle_session_survives_bidirectional_silence_via_keepalive`、`ssh_loopback_*`、`recovery_saturated_primary_queue_*`）模拟对端不读 stdin / 队列饱和，但都是本地控制 writers；不能复现外部服务器 keepalive 超时、TCP zero-window、广告窗口变化等真实场景。
- 证据：实现报告 `Concerns` 同样承认。
- 建议：在 macOS 真实服务器上做定向验证；Windows 自动化层面已不能再向前推进。

### Finding 6 — close 抢占客户端锁失败被诚实报告（Note，验证充分）
- Spec 条款：规格「一」验收 → 「关闭标签、断开连接和后续连接操作应在有界时间内返回」「其他可用连接不会被故障终端阻塞」。
- 代码位置：`src-tauri/crates/rshell-protocol/src/ssh/client.rs:866-874`、`src-tauri/crates/rshell-core/src/session/service.rs:1049-1059`、`1086-1090`。
- What：`close_terminal` 在协议层是同步操作（移除 map + 设 state=Closed + 关闭 route），但仍需 `&mut self` 即 SshClient 写锁。核心层用 `terminal_request`（10 秒）兜底。`recovery_close_bypasses_saturated_request_queue` 测试只覆盖了客户端锁空闲的场景（`bare_client`），不覆盖 SshClient 读锁被并发 send 持锁的情况。实现报告与 `docs/08-incomplete-features.md` 都明确：客户端锁不可得时返回结构化失败、不假装清理成功，依赖后续 disconnect/连接 teardown 兜底。
- 证据：`recovery_disconnect_lock_wait_is_bounded_and_retires_old_generation` 与 `recovery_ssh_lock_admission_is_bounded_and_other_connection_is_independent` 都断言锁等待有界；`releasePty` 兜底路径保留。
- 建议：当前实现与文案符合「不伪造清理」要求；可在后续 macOS 实机环节验证高争用下 close 的体感。

### Finding 7 — Unmount 清理与新生命周期共存（Note，验证充分）
- Spec 条款：实施约束 → 「不模拟连接状态」「输入不可用状态应与连接状态区分」。
- 代码位置：`src/components/TerminalPane.vue:82-158, 632-650`；`tests/unit/TerminalPaneUnmount.spec.ts:110-202`。
- What：`unmounted` 闩锁、`ioGeneration`、新的 `recoveryRequired`/`recoveryError`/`terminalReady`/`reconnecting` 四个 ref 在原有 R3-05 / R3-06 测试套件（R3-05 自动补挂、R3-06 卸载与在途 async 竞态）中继续通过（npm test 全套 279 通过），证明新生命周期未破坏 unmount 清理路径。
- 证据：原测试套件 `R3-06` 与 `R3-05` 用例未被改动；`SSH uncertain input recovery` 三条新用例全部通过。
- 建议：保留现有 unmount 测试，并把它们与新的 `recoveryRequired` 状态机结合后再次复跑确认。

### Finding 8 — Secret / 输入内容未出现在新增日志中（Note，验证充分）
- Spec 条款：实施约束 → 「不记录私钥、密码或脚本中可能含有的凭据」。
- 代码位置：`src-tauri/crates/rshell-protocol/src/ssh/client.rs:47, 268, 275, 330, 401, 411, 427, 465, 474, 488, 518, 613, 651, 656, 661, 802, 818, 925, 934`；`src-tauri/crates/rshell-protocol/src/ssh/requests.rs`（无日志调用）；`src-tauri/crates/rshell-core/src/session/service.rs:818, 877, 881, 890`。
- What：新增/改动的日志调用只引用 `session_id`、`terminal_id`、`cols/rows`、`channel`、`error` 与时长，不引用输入字节、密码或口令字符串。`grep` 在两个 crate 的改动文件中确认无 `password` / `passphrase` / `secret` 字面量出现在新增日志格式串中。
- 证据：`grep -E "log::|tracing::|eprintln!|println!"` 仅返回 `use tracing::{...}` 一行（导入语句），无新增调用点携带敏感字段。
- 建议：保持现状。

### Finding 9 — 取消已入队但未执行的请求被显式丢弃（Note，验证充分）
- Spec 条款：规格「一」验收 → 「已过期且尚未开始的输入不会在对端恢复后被悄悄发送」。
- 代码位置：`src-tauri/crates/rshell-protocol/src/ssh/requests.rs:50-97, 121-161`。
- What：`request` 在 writer 取出前显式检查 `reply.is_closed() || Instant::now() >= request.deadline`，过期的请求直接 `continue`，字节不会到 `writer.execute()`。`CancelOnDrop` 在 caller future 被 drop（含外层 `terminal_request` 超时）时把 state 切到 `Recovery`，旧 PTY 不再被任何后续请求复用。
- 证据：测试 `recovery_expired_and_abandoned_queued_input_never_executes` 直接断言 `sent.lock().unwrap().is_empty()`。
- 建议：保持现状。

## Test Coverage Matrix

| 规格/计划验收要点 | 覆盖测试 | 状态 |
|---|---|---|
| 对端停止读取、队列饱和时，输入、resize、关闭、断开在规定预算内返回 | `recovery_saturated_primary_queue_has_end_to_end_deadline`（输入，11s 截止）<br>`recovery_primary_reply_has_deadline`（resize，11s 截止）<br>`recovery_close_bypasses_saturated_request_queue`（close，100ms 截止）<br>`recovery_disconnect_lock_wait_is_bounded_and_retires_old_generation`（断开，16s 截止）<br>`recovery_ssh_lock_admission_is_bounded_and_other_connection_is_independent`（核心层 send/resize/close，11s 截止） | 全部 GREEN |
| 一次输入超时即可看到持续错误和恢复入口；结果不确定时文案不声称未执行 | `TerminalPaneUnmount.spec.ts:SSH uncertain input recovery / one uncertain input stops keyboard, backspace and paste until real reconnect finishes`<br>`terminal_recovery_required` 文案「输入结果不确定，远端可能已执行部分输入」「重连不会重放输入」 | GREEN |
| 已过期且尚未开始的输入不会在对端恢复后被悄悄发送 | `recovery_expired_and_abandoned_queued_input_never_executes`（同时覆盖已过期 reply 与已 drop reply 两类）<br>`recovery_stalled_execution_isolated_and_independent_terminal_still_writes`（含隔离 queued 请求） | GREEN |
| 手动重连成功后输入恢复，旧终端请求不进入新终端；失败时状态不伪装为可用 | `TerminalPaneUnmount.spec.ts:SSH uncertain input recovery / failed reconnect and late successful IO cannot clear uncertain recovery` | GREEN |
| 重复关闭、超时与关闭竞争、关闭后晚到事件有回归覆盖 | `recovery_close_bypasses_saturated_request_queue`（重复 close）<br>`recovery_disconnect_lock_wait_is_bounded_and_retires_old_generation`（disconnect 与持锁 close 的竞争）<br>`recovery_drop_client_cancels_all_old_terminal_actors`（drop 后的 actor 取消） | GREEN |
| 其他可用连接不会被故障终端阻塞 | `recovery_ssh_lock_admission_is_bounded_and_other_connection_is_independent`（核心层，断言独立连接不被持锁客户端阻塞）<br>`recovery_stalled_execution_isolated_and_independent_terminal_still_writes`（协议层，断言独立 PTY 仍可写入） | GREEN |
| 附加标签（sibling）独立隔离 | `TerminalPaneUnmount.spec.ts:SSH uncertain input recovery / a sibling label keeps input disabled until its replacement PTY and attach succeed` | GREEN |
| 主标签输出 EOF 沿用既有整条连接拆除契约 | 现有 core `data_reader` 在 `output_rx.recv()` 返回 `None` 时进入 `tokio::time::timeout(DISCONNECT_SSH_TIMEOUT, …).disconnect_ssh()` 路径（`session/service.rs:593-622`）。**无 Task 1 新增的定向回归测试。** | 既有契约路径未变化但缺 Task 1 范围内的针对性测试 |
| 未在 add-task 范围内的：Tauri IPC 字符串契约 | `terminal_recovery_has_a_structured_ipc_kind`（在 `src-tauri/src/error.rs`，由 `cargo test -p rshell --lib` 执行） | GREEN（独立审查补跑确认） |

## Risks Not Closed

1. **macOS 产品验收（out of scope here）**：实现报告自承未做真实 macOS SSH/多标签/网络故障验收；这是方案文档定义的两阶段验收，本任务范围只覆盖「开发验证完成」。
2. **Tauri 单元测试不在任务范围门禁内运行**：见 Finding 3。下一次走 `scripts/verify.sh` 必须包含 `cargo test --workspace`，否则 `terminal_recovery_required` IPC 字符串契约无自动化断言。
3. **测试夹具未覆盖外部服务器 zero-window 恢复**：见 Finding 5；需 macOS 实机补做。
4. **close-lock-pre-timeout 失败被诚实报告**：见 Finding 6；当前实现 + 文案已经承诺「不假装清理成功」，但 SshClient 写锁被并发 send 持锁的场景没有定向测试覆盖；现有测试仅在客户端锁空闲时断言快速 close。下一次可以补一个「SshClient 读锁被并发持锁时 close 在 10s 内返回 TerminalRecoveryRequired」的测试。
5. **unmount 清理与新生命周期共存**：见 Finding 7；现有 R3-05/R3-06 套件继续通过，但 `recoveryRequired`/`reconnecting`/`terminalReady` 的相互作用未来若加新路径，需补充交叉场景测试（例如「unmount 发生在 reconnect 等待 connect 期间」）。
6. **`kind_strings_are_stable` 与 `core_error_mapping_is_exhaustive` 未更新到覆盖 TerminalRecoveryRequired**：见 Findings 1、2。两条测试同时运行时不暴露问题，但测试强度不一致。
7. **核心层 `terminal_io_error` 把非 `TerminalRecoveryRequired` 的协议错误降级为 `ConnectionError(String)`**：这是合理的兜底（避免暴露协议层细节），但因此「输入错误的具体原因」会被前端吞到 `message` 文案里——和「一次输入超时」的可识别 kind 流程不冲突，因为只有 `TerminalRecoveryRequired` 走结构化分支。但任何后续要新增「输入重置 / 远端关闭 shell」之类的结构化错误时，需要更新 `terminal_io_error` 与 IPC 映射，否则会丢失区分度。

## Test-Code Spot-Check

下列断言行从工作树复制而来，逐条评估「是否真正证明实现报告所述行为」。

### `requests::tests::recovery_expired_and_abandoned_queued_input_never_executes`

```rust
// requests.rs:213-249
#[tokio::test(start_paused = true)]
async fn recovery_expired_and_abandoned_queued_input_never_executes() {
    for abandoned in [false, true] {
        let (handle, requests) = TerminalHandle::new(0, 2, Duration::from_millis(20));
        let (reply, received) = oneshot::channel();
        let received = if abandoned {
            drop(received);
            None
        } else { Some(received) };
        handle.sender.send(Request {
            operation: Operation::Send(b"old".to_vec()),
            deadline: if abandoned { Instant::now() + Duration::from_secs(1) }
                      else { Instant::now() - Duration::from_millis(1) },
            reply,
        }).await.unwrap();
        let writer = writer(false);
        let sent = writer.sent.clone();
        let task = tokio::spawn(run_writer(writer, requests, handle.clone()));
        tokio::task::yield_now().await;
        assert!(sent.lock().unwrap().is_empty(),
            "expired/cancelled input was replayed by old actor");
        handle.close();
        task.await.unwrap();
        drop(received);
    }
}
```

**评估**：是真正证明。`ControlledWriter::execute` 在任何分支都会先 `self.sent.lock().unwrap().push(data)`（包括 `block=true` 路径），所以 `sent.is_empty()` 是「`writer.execute` 一次也没被调用」的强证明。`abandoned=false` 走 `Instant::now() >= request.deadline` 早于执行；`abandoned=true` 走 `reply.is_closed()` 早于执行。两条路径都被验证。

### `requests::tests::recovery_stalled_execution_isolated_and_independent_terminal_still_writes`

```rust
// requests.rs:251-289
let first = tokio::spawn(async move { first.request(Operation::Send(b"partial".to_vec())).await });
tokio::task::yield_now().await;
let queued = tokio::spawn(
    async move { queued.request(Operation::Send(b"no replay".to_vec())).await },
);
assert!(matches!(first.await.unwrap(), Err(ProtocolError::TerminalRecoveryRequired)));
assert!(matches!(queued.await.unwrap(), Err(ProtocolError::TerminalRecoveryRequired)));
assert_eq!(old_sent.lock().unwrap().as_slice(), [b"partial".to_vec()]);
let (other, requests) = TerminalHandle::new(1, 2, Duration::from_millis(20));
let writer2 = writer(false);
let other_sent = writer2.sent.clone();
let task2 = tokio::spawn(run_writer(writer2, requests, other.clone()));
other.request(Operation::Send(b"healthy".to_vec())).await.unwrap();
assert_eq!(other_sent.lock().unwrap().as_slice(), [b"healthy".to_vec()]);
```

**评估**：是真正证明。
- `old_sent` 包含 `b"partial"`：第一个请求已到达 writer 并 `execute` 被调用 → 已部分写入，**未撒谎**地反映「输入已实际送出到 writer」。
- queued 与 first 均返回 `TerminalRecoveryRequired`：第二请求没有执行（queued 排在前一未完成请求之后），并且 `first` 在超时/取消后被清理，未在另一 channel 上泄漏。
- `other_sent` 仅含 `b"healthy"`：独立 `TerminalHandle` 不受故障 handle 影响 → sibling/独立连接隔离。

### `client::tests::recovery_drop_client_cancels_all_old_terminal_actors`

```rust
// client.rs:1912-1927
#[tokio::test]
async fn recovery_drop_client_cancels_all_old_terminal_actors() {
    let mut client = bare_client();
    let (terminal, _requests) = TerminalHandle::new(0, 1, REQUEST_BUDGET);
    let mut state = terminal.subscribe();
    let _actor_handle = terminal.clone();
    client.channel = Some(terminal.clone());
    client.terminals.insert(client.config.id, terminal);
    drop(client);
    assert!(tokio::time::timeout(std::time::Duration::from_millis(30), state.changed())
        .await.is_ok(),
        "dropping old transport must cancel actors rather than leave admitted input alive");
}
```

**评估**：是真正证明。`Drop` 同步调用 `handle.close()`，把 state 切到 `Closed`，`state.changed()` 立即可观察到。30ms 宽松上限仅用于调度，不掩盖真行为。

### `session::service::tests::recovery_ssh_lock_admission_is_bounded_and_other_connection_is_independent`

```rust
// service.rs:2494-2530
let other_result = tokio::time::timeout(
    std::time::Duration::from_millis(1),
    svc.send_data(other_id, None, b"independent"),
).await;
assert!(other_result.is_ok(), "unrelated connection must not wait on the stuck client lock");
let send    = tokio::time::timeout(std::time::Duration::from_secs(11), svc.send_data(id, None, b"input"));
let resize  = tokio::time::timeout(std::time::Duration::from_secs(11), svc.resize_terminal(id, None, 80, 24));
let close   = tokio::time::timeout(std::time::Duration::from_secs(11), svc.close_terminal(id, Uuid::new_v4()));
let (send, resize, close) = tokio::join!(send, resize, close);
assert!(send.is_ok(),   "send waits indefinitely for SSH lock");
assert!(resize.is_ok(), "resize waits indefinitely for SSH lock");
assert!(close.is_ok(),  "close waits indefinitely for SSH lock");
assert!(matches!(send.unwrap(),   Err(CoreError::TerminalRecoveryRequired)));
assert!(matches!(resize.unwrap(), Err(CoreError::TerminalRecoveryRequired)));
assert!(matches!(close.unwrap(),  Err(CoreError::TerminalRecoveryRequired)));
```

**评估**：是真正证明。
- 持锁的 `_writer = client.write().await` 占用 `SshClient` 写锁 → `id` 连接的 send/resize/close 必须等锁或超时。
- `other_result` 用 1ms 截止且断言 `is_ok()`：独立连接不阻塞（`recovery_ssh_lock_admission_is_bounded_and_other_connection_is_independent` 名副其实）。
- 三项 11s 截止 + 三项 `is_ok()` 表明客户端锁等待确实有界。
- 三项 `Err(TerminalRecoveryRequired)` 表明 `terminal_request` 超时确实把错误结构化为 `TerminalRecoveryRequired`，**未**返回虚假 `ConnectionError`。

### `TerminalPaneUnmount.spec.ts:one uncertain input stops keyboard, backspace and paste until real reconnect finishes`

```ts
// TerminalPaneUnmount.spec.ts:206-239
sendInputMock.mockRejectedValueOnce({ kind: "terminal_recovery_required", message: "input outcome uncertain" });
terminalRuntime.input("first");
await flushPromises();
expect(wrapper.find('[data-test="term-recovery"]').exists()).toBe(true);
expect(wrapper.text()).toContain("不确定");
expect(terminalRuntime.options.disableStdin).toBe(true);
terminalRuntime.input("later");
terminalRuntime.key(new KeyboardEvent("keydown", { key: "Backspace" }));
vi.stubGlobal("navigator", { clipboard: { readText: vi.fn().mockResolvedValue("clipboard") } });
window.dispatchEvent(new CustomEvent("rshell:terminal-action", { detail: { sessionId: "session-1", action: "paste" } }));
await flushPromises();
expect(sendInputMock).toHaveBeenCalledTimes(1);
await wrapper.find('[data-test="term-reconnect"]').trigger("click");
await flushPromises();
expect(disconnect).toHaveBeenCalledWith("session-1");
expect(connect).toHaveBeenCalledWith("session-1");
expect(terminalRuntime.options.disableStdin).toBe(true);
finish();
await flushPromises();
expect(wrapper.find('[data-test="term-recovery"]').exists()).toBe(false);
expect(terminalRuntime.options.disableStdin).toBe(false);
terminalRuntime.input("new input");
await flushPromises();
expect(sendInputMock).toHaveBeenCalledTimes(2);
expect(new TextDecoder().decode(sendInputMock.mock.calls[1][1])).toBe("new input");
```

**评估**：是真正证明。
- 第一次 `sendInputMock.mockRejectedValueOnce({ kind: "terminal_recovery_required", ... })` 模拟后端一次超时即返回结构化错误。
- 紧接着的 `terminalRuntime.input("later")`、`key(Backspace)`、`paste` 都必须不再到达 `sendInputMock`：测试以 `toHaveBeenCalledTimes(1)` 锁定。`input` 走 `term.onData`，由 `isConnected.value` 短路；`key(Backspace)` 走 `submitInput("\x08")`，由 `isConnected.value` 短路；`paste` 走 `onTerminalAction → pasteClipboard`，由 `isConnected.value` 短路。三条路径都被「一次不确定 → 全部禁用」覆盖。
- `toHaveBeenCalledTimes(2)` 表明重连后才允许输入恢复，没有「假可用」窗口。

### `TerminalPaneUnmount.spec.ts:failed reconnect and late successful IO cannot clear uncertain recovery`

```ts
// TerminalPaneUnmount.spec.ts:241-262
sendInputMock.mockImplementationOnce(() => new Promise<void>(resolve => { lateSuccess = resolve; }));
sendInputMock.mockRejectedValueOnce({ kind: "terminal_recovery_required", message: "input outcome uncertain" });
terminalRuntime.input("earlier pending");
terminalRuntime.input("first");
await flushPromises();
lateSuccess();
await flushPromises();
expect(wrapper.find('[data-test="term-recovery"]').exists()).toBe(true);
await wrapper.find('[data-test="term-reconnect"]').trigger("click");
await flushPromises();
expect(wrapper.text()).toContain("network unavailable");
expect(wrapper.find('[data-test="term-recovery"]').exists()).toBe(true);
expect(terminalRuntime.options.disableStdin).toBe(true);
```

**评估**：是真正证明。
- 第一条 `sendInput` 返回 pending（模拟后台早已发出但未确认），第二条直接拒绝 → `recoveryRequired=true`。
- 在 `lateSuccess()` 触发后，第一条 `sendInput` 的 `.then()` 会执行 `noteIoSuccess()`，但 `noteIoSuccess` 只重置 `ioFailureStreak`/`ioFailureNotified`，**不**清除 `recoveryRequired` → 测试断言横幅仍在，是正确证明。
- `connect` 拒绝 → `reconnect` 进入 catch 分支 → `recoveryError.value` 设为包含 `network unavailable` 的字符串 → 横幅文案与禁用输入都保留。失败重连确实保留恢复态，不伪装可用。

## 结论

- **核心规格全部命中**：5 条验收要点都有直接断言或组合断言支撑。
- **无静默成功、无伪造清理、无把超时写成「远端未执行」**：文案与错误结构均如实表达「结果不确定」。
- **次级风险**：`kind_strings_are_stable` / `core_error_mapping_is_exhaustive` 未覆盖新 kind、Tauri 测试不在任务范围命令内执行、外部服务器 zero-window 复现未做——均不属于 Task 1 范围内的可修复缺陷，应在 Task 4 整体门禁与 macOS 实机验收阶段补齐。

参考提交：`c1afefc`；审查报告不提交修改。
