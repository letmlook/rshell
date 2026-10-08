# Task 1 report: SSH deadlines and terminal recovery

Status: DONE_WITH_CONCERNS (development slice verified; macOS product acceptance and final workspace gate remain separate).

## Changes

- New private SSH `requests.rs` owns each PTY's request deadline, cancellation state, and writer lifecycle. Queue capacity remains 32. A 10-second deadline covers admission plus execution/reply. Requests whose caller is gone or whose deadline has passed are never started. A dropped caller future, execution failure, or timeout isolates the old PTY and drops queued requests before best-effort wire close; no replay occurs.
- russh 0.62 split read/write halves replace the stale 0.48 actor design. Receive draining stays independent of blocked writes. Cancellation closes the output route immediately; wire close is bounded to 5 seconds in the background. SshClient Drop closes all old PTYs and clears routes.
- Protocol/core/Tauri distinguish `TerminalRecoveryRequired` / `terminal_recovery_required`. Existing TypeScript IPC errors already expose a string `kind`, consumed directly by TerminalPane. No terminal text or authentication materials were added to logs.
- Core send/resize budget is **10 seconds total**, not 10 seconds for a lock plus another 10 seconds for protocol I/O. The outer timeout encloses client lock acquisition and the protocol future. If it drops an admitted protocol request, that request's cancellation guard isolates the old PTY. Closing a label has a 10-second client-lock budget and then sends out-of-band cancellation rather than queueing Close. Disconnect lock admission plus execution is bounded to 15 seconds. PTY channel opening is also bounded to 10 seconds.
- TerminalPane shows a persistent uncertain-result notice on the first recovery error, disables keyboard/backspace/paste/resize, and offers a real disconnect → connect → ensure PTY → attach path. Failed recovery keeps the restriction and reason; old asynchronous I/O completions cannot clear a later generation. `terminalReady` gates stdin until a replacement PTY and attach succeed, including sibling labels. Existing async unmount cleanup remains covered.
- Current-feature documentation describes budgets, manual recovery, no replay, and actual fault scope.

## Fault scope and failure paths

Extra-PTY failure isolates that label while other labels can remain usable. Primary output EOF retains the existing core contract: retire the whole connection, close sibling PTYs, and emit the real Disconnected event. Sibling panes disable input on this backend transition and wait for their own PTY/attach before enabling it after reconnection. Manual recovery replaces the entire SSH connection. Independent connections have separate request queues, cancellation state, and client locks.

If label close times out *before acquiring the client lock*, it returns structured failure and does not claim cleanup succeeded. The old backend terminal remains until later connection teardown/drop; the existing releasePty fallback logs that remote reclamation path. Once actual protocol close runs, cancellation and route removal happen immediately, independent of queued/stalled writes. No new cleanup fallback deletes unrelated labels.

Timeout outcomes are deliberately conservative: UI does not claim that remote input was unexecuted, including admission timeouts. Confirmed ordinary rejections retain their existing error path. Recovery does not undo already transmitted bytes or restore the remote process.

## RED evidence (before relevant production behavior)

All Rust commands used PowerShell environment variables:

```powershell
$env:RUSTUP_TOOLCHAIN='stable'
$env:RUSTUP_NO_UPDATE_CHECK='1'
$env:CARGO_TARGET_DIR='C:/code/github/rshell/src-tauri/target'
```

Working directory: `C:/Users/lp/.codex/worktrees/reliability-recovery/rshell`.

1. `cargo test -p rshell-protocol --lib recovery_ --manifest-path src-tauri/Cargo.toml` against unchanged request/actor logic: **0 passed, 3 failed**, exit 1. Failures: `recovery_saturated_primary_queue_has_end_to_end_deadline` (unbounded queue admission), `recovery_primary_reply_has_deadline` (main resize waits forever), `recovery_close_bypasses_saturated_request_queue` (Close queues behind writes). Tokio paused-time tests advance virtual deadlines instead of sleeping 10 seconds.
2. `npm test -- tests/unit/TerminalPaneUnmount.spec.ts` before Vue changes: **3 passed, 2 failed**, exit 1. Missing persistent recovery notice and reconnect action after one uncertain input error.
3. `cargo test -p rshell-protocol --lib recovery_ --manifest-path src-tauri/Cargo.toml` on the incomplete lifecycle implementation without pre-execution expiry guard: **4 passed, 1 failed**, exit 1. `recovery_expired_and_abandoned_queued_input_never_executes` observed an actual recorded send side effect from expired queued bytes. Restored the explicit abandoned-reply/deadline guard.
4. `cargo test -p rshell-core --lib recovery_ --manifest-path src-tauri/Cargo.toml` before core lock deadline changes: **0 passed, 1 failed**, exit 1. `recovery_ssh_lock_admission_is_bounded_and_other_connection_is_independent` failed at indefinite send lock admission. Feature-union rebuild took 3m25s; the test itself took virtual time only.
5. `cargo test -p rshell-protocol --lib recovery_drop --manifest-path src-tauri/Cargo.toml` before SshClient Drop cancellation: **0 passed, 1 failed**, exit 1. Corrected fixture retains the actor handle and proves old PTYs are otherwise left active after client drop.
6. `npm test -- tests/unit/TerminalPaneUnmount.spec.ts` before the terminalReady gate: **5 passed, 1 failed**, exit 1. Sibling replacement PTY was still pending when stdin became enabled on Connected.

## GREEN evidence

- Initial protocol focused regressions: **3 passed**, exit 0.
- Core focused lock regression after fix: **1 passed**, exit 0.
- Frontend focused recovery/unmount tests after first fix: **5 passed**, exit 0.
- `cargo test -p rshell-protocol --lib --manifest-path src-tauri/Cargo.toml`: **69 passed**, exit 0. Includes queued expiration/cancellation, stalled execution/queued input isolation, independent writer, repeated close, drop cancellation, existing real russh loopback, and keepalive/silence regressions.
- `cargo test -p rshell-core --lib --manifest-path src-tauri/Cargo.toml`: **189 passed**, exit 0. Includes bounded send/resize/close client-lock admission, independent client lock, and disconnect retirement despite a blocked client lock.
- `npm test`, latest run after terminalReady: **37 files passed, 279 tests passed, 3 skipped**, exit 0.
- `npm run typecheck`, latest run after terminalReady: exit 0.
- `cargo clippy -p rshell-protocol -p rshell-core --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings`: exit 0.
- `cargo fmt --all --manifest-path src-tauri/Cargo.toml` completed. `git diff --check` completed without whitespace errors.

The first full frontend run was **277 passed, 1 failed, 3 skipped**. Existing `TerminalPalette` Backspace-success fixture had mounted a disconnected terminal but expected a backend send; changed its fixture to Connected, preserving actual Backspace behavior and separate recovery suppression coverage. Full rerun then passed; a later readiness regression justified the final 279-test run above.

One first full protocol compile exposed old tests moving authentication fields out of SshClient after adding Drop (E0509); changed those assertions to borrow authentication values. Final full suite passes. A test-double return-type literal inference error was fixed before final typecheck.

## Concerns / unverified acceptance

- No macOS real SSH/network/multi-label acceptance was possible in this Windows environment. No real credentials or external service mutations were used.
- Tauri's new structured-mapping unit test is added in `src-tauri/src/error.rs`; task-scoped protocol/core suites do not execute shell tests. Root's final workspace suite must execute that test and broader repository gates.
- Core tests emit an informational Windows linker warning about creating `.lib`/`.exp` files; tests pass and package clippy with `-D warnings` passes.
- Close that cannot acquire a client lock reports failure and depends on later connection teardown for final remote cleanup, as described above. No successful cleanup is fabricated.
- Tests prove lifecycle behavior with controllable writer boundaries and existing real loopback SSH; they do not reproduce an external server's full zero-window recovery on macOS.
