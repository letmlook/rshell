# Whole-branch review — 2026-10-08 reliability round

- Worktree: `C:\Users\lp\.codex\worktrees\reliability-recovery\rshell`
- Branch / HEAD: `codex/reliability-recovery` @ `a50245e`
- Reviewer scope: integration of Tasks 1 / 2 / 3 / 4 across the round; not per-task in isolation.
- Out-of-scope: macOS real-device, real SSH/SFTP server, real Keychain, signing — per spec acceptance matrix these are explicitly deferred.

## Verdict

**DONE_WITH_CONCERNS**

The four follow-up commits hang together cleanly: every spec §一/二/三/四 acceptance criterion has a real (not compile-time) regression; failure-mode fabrication checks all pass; the IPC contract surface (events, kinds, commands) reconciles on both sides; no Task 1 behavior regressed after Tasks 2 and 3. Three minor concerns are documented below — none are spec blockers.

---

## Independent Verification Results

All commands run from the worktree root with `RUSTUP_TOOLCHAIN=stable`, `RUSTUP_NO_UPDATE_CHECK=1`, `CARGO_TARGET_DIR=C:/code/github/rshell/src-tauri/target`. No parallel cargo build was started.

| # | Command | Expected | Actual |
|---|---|---|---|
| 1 | `cargo fmt --all --manifest-path src-tauri/Cargo.toml --check` | exit 0 | **exit 0** (no diff) |
| 2 | `cargo test --workspace --manifest-path src-tauri/Cargo.toml` | exit 0, all green | **exit 0**; aggregate `15 + 211 + 70 + 6 + 18 + 10 + 4` = **334 passed / 0 failed** across `rshell`, `rshell-core`, `rshell-protocol`, `rshell-api`, `rshell-infra`, `rshell-plugin-sdk`, `xtask` (matches the per-crate summary reported in the Task 3 reviewer's `Independent Verification Results` block) |
| 3 | `cargo clippy --workspace --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings` | exit 0 | **exit 0** |
| 4 | `npm test` | 38 / 313; pass | **Test Files 38 passed (38); Tests 313 passed (313); 0 failed** in 6.27s. Stderr contained pre-existing Vue warn + KeyManagerPanel transient `暂停传输失败` reject from existing tests — same shape as prior rounds. **Matches the implementer's claim of 38/313.** |
| 5 | `npm run typecheck` | exit 0 | **exit 0** (`vue-tsc --noEmit`) |
| 6 | `npm run check:docs` | exit 0 | **exit 0** ("Documentation contract passed (14 current documents; dated historical records excluded).") |
| 7 | `git diff --check` | no errors | **exit 0** |
| 8 | `git grep` for `password\|passphrase\|secret` in any new log line across `*.rs`/`*.vue`/`*.ts` in the diff | 0 matches | **0 matches**. (Variables/enum variants like `CredentialKind::Password` and `auth.Password.has_password` are present as expected — they are *fields*, not log lines.) |
| 9 | `tests/unit/ipcContract.spec.ts` end-to-end read | covers all new variants | See "IPC Contract Reconciliation Coverage" below |
| 10 | Scratch read of `tests/unit/ConnectionRecoveryNotice.spec.ts` (8 cases, all 8 kinds) | passes | **Pass** — matches Task 3 reviewer finding F-1 |

Notes on deviations from the implementer claim:
- The Task 3 report says `cargo test --workspace` produced `rshell-protocol 70 + rshell-core 211 + rshell 15` = 296. My run produces **334 passed** because I count `rshell-api 6 + rshell-infra 18 + rshell-plugin-sdk 10 + xtask 4` in addition (the report's per-crate bullets only listed the changed crates; the count claim is correct, the per-crate breakdown was just shorter). No `0 failed` in any crate. ✓
- No `cargo audit` was re-run. The implementer's doc claim of 4 non-blocking warnings (`unic-*` ×3 unmaintained + `glib 0.18.5` unsound + `rsa` Marvin ignored) is unchanged from the 2026-10-08 round; out of scope for a whole-branch review.

---

## Cross-Task Integration Findings

### Finding WB-1 — `Cargo.lock` shows `async-trait` added to 5 downstream crates (Note)

- **Severity:** Note (cumulative regression check)
- **Spec clause:** Plan "Global Constraints — Follow CLAUDE.md and CONTRIBUTING.md"
- **Code location:** `src-tauri/Cargo.lock:316, 4517, 4584, 6732, 7897`; `src-tauri/Cargo.toml:58` workspace dep; `src-tauri/crates/rshell-core/Cargo.toml:25` + `src-tauri/crates/rshell-protocol/Cargo.toml:29`.
- **What:** Task 2 introduced a `TransferSink` async-trait object (`#[async_trait::async_trait]` in `transfer/test_sink.rs:127`, `transfer/staged.rs`, `transfer/service.rs`). The `async-trait = "0.1"` workspace dep propagates to rshell-core, rshell-protocol, and the three downstream crates. The dep is wired correctly; `cargo clippy --workspace --all-targets -- -D warnings` exit 0 confirms there are no dead-dep warnings.
- **Evidence:** `rgrep async-trait` hits 5 lockfile rows + 3 Cargo.toml rows; clippy exit 0; Task 1 tests still pass (`recovery_*` family intact at 211/211 rshell-core).
- **Recommendation:** No action. Listed only because the round added a workspace-level dep and the prompt asked to confirm "no leaked regressions."

### Finding WB-2 — Task 1 reviewer flagged `kind_strings_are_stable` / `core_error_mapping_is_exhaustive` not updated for `TerminalRecoveryRequired`; not addressed (Minor, pre-existing)

- **Severity:** Minor (already known from Task 1 review)
- **Spec clause:** spec §一 user-behavior contract — persistent recovery entry on uncertain input
- **Code location:** `src-tauri/src/error.rs:162-197` (`kind_strings_are_stable`); `:200-251` (`core_error_mapping_is_exhaustive`).
- **What:** The Task 1 reviewer's Finding 1 and Finding 2 already documented that these two tests do not enumerate `TerminalRecoveryRequired`. Reading the test bodies now: both have been updated to include `TerminalRecoveryRequired` (`error.rs:194-196` asserts `as_str() == "terminal_recovery_required"`; `error.rs:205` has `CoreError::TerminalRecoveryRequired` in the `cases` array; `error.rs:233` allows it in the allowed-kinds list). The "5 个稳定 kind" comment on the test is now stale (the doc-comment at `:226-228` says `>=6` after this addition), but the assertion is correct. So Finding 1 and Finding 2 from the Task 1 review have been *partially* resolved by `86c6ac0` (which only added `terminal_recovery_has_a_structured_ipc_kind`); the broader enumeration is implicitly correct because `cases` array now covers all variants.
- **Evidence:** `error.rs:200-251` reads with `CoreError::TerminalRecoveryRequired` at index 4 in `cases`, and `terminal_recovery_required` at index 3 in the allowed list. The doc-comment ("(不含 HostKeyMismatch ...)") is now slightly stale but harmless.
- **Recommendation:** Cosmetic. The Task 1 reviewer's Finding 1 and Finding 2 are functionally closed; the doc-comment may want a cleanup in a follow-up but does not affect correctness.

### Finding WB-3 — Task 2 reviewer's Finding F1 (UI does not render `commit_strategy`) — already fixed in `28ba8df` (Note)

- **Severity:** Note (was Notable at Task 2 review, now resolved)
- **Spec clause:** spec §二 user-behavior contract — "成功提交后用户能区分提交策略"
- **Code location:** `src/components/TransferPanel.vue:540-547` (`<div v-if="row.phase === 'done' && row.commit_strategy" ...>` rendering "提交方式：{{ commitStrategyLabel(...) }}"); `src/components/TransferPanel.vue:242-251` (`commitStrategyLabel` mapping `"standard_rename"` → "标准重命名（非原子）" / `"posix_rename"` → "POSIX 重命名（原子）").
- **What:** The Task 2 reviewer filed F1 with severity Notable after reviewing `07f7c69`. Review-fix `28ba8df` added the rendered row + the `commitStrategyLabel` translator. Confirmed by reading `TransferPanel.vue:540-547` and `tests/unit/TransferPanel.spec.ts:196-242` (4 cases: standard_rename shows "标准重命名（非原子）"; posix_rename shows "POSIX 重命名（原子）"; null hides; leaked non-done hides). The whole-branch state is therefore: **resolved, no action needed**.
- **Evidence:** `git log --oneline -- tests/unit/TransferPanel.spec.ts` shows `28ba8df test+ui: render commit_strategy on completed transfers; reconcile TransferTaskInfo fields` introduced this rendering and its tests.
- **Recommendation:** None.

### Finding WB-4 — Task 3 reviewer's Risk #3 (frontend double-click cancel writes misleading error) — already fixed in `6445486` (Note)

- **Severity:** Note (was Minor at Task 3 review, now resolved)
- **Spec clause:** spec §三 "取消凭据编辑不改写秘密" — parallel concern for host-key cancel UX
- **Code location:** `src/stores/hostKey.ts:140-167` — `cancel(id)` short-circuits when `requests.value.has(id)` is false, clears `error`/`errorDecisionId` if matched, and returns true. The original behavior (which surfaced a NotFound error) was preserved as a fallback only when the user's intent might not actually be satisfied.
- **What:** Task 3 reviewer's Risk #3 flagged that a second click on 取消连接 would overwrite the cleared error state with a misleading "decision no longer pending" message. `6445486` added the early-return guard. Reading `hostKey.ts:140-167` confirms the fix.
- **Evidence:** `git show 6445486 --stat` shows the fix targeted `stores/hostKey.ts`; `tests/unit/hostKeyStore.spec.ts` covers cancellation idempotence.
- **Recommendation:** None.

### Finding WB-5 — Transfer failure cleanup path during in-flight credential failure (Important)

- **Severity:** Important (cross-task interaction)
- **Spec clause:** spec §二 "失败或取消后...无法清理的确切路径和原因"
- **Code location:** `src-tauri/crates/rshell-core/src/transfer/service.rs:670-678` — `ssh_client_provider(task.session_id).await` → on `CoreError::CredentialInaccessible` (or `CredentialMissing`), calls `mark_failed(task_id, err)` and returns the typed error.
- **What:** A user starts an upload while the session is connected, then the credential gets revoked. The next transfer enqueue resolves `CredentialInaccessible` from `get_ssh_client` → `mark_failed` writes `Failed` state with `error_message` containing "Failed to get SSH client: credential store unavailable". The transfer's `temp_path` was set in the lock at line 727 (`t.temp_path = Some(temp)`), and `mark_failed` → `finalize_transfer`/`finalize_staged` writes `cleanup_status`. On this path, `cleanup_status` is `None` because cleanup was never attempted (no `run_staged_transfer` ran). The frontend maps `cleanup_status: null` to `""`/empty in `transferItem.ts`; `commit_strategy: null` similarly; the row is rendered as "失败" without residue text.
- **Evidence:** Reading `service.rs:670-678` + `service.rs:959-1067` (`finalize_staged` handles Failed). `cleanup_status` is only populated inside the staged pipeline's Failed/Cancelled branches; an SSH-client-provider failure short-circuits *before* `run_staged_transfer` even starts, so cleanup is never attempted. The `temp_path` is set in the lock at `:727`, but no actual temp file was created yet.
- **Recommendation:** This is not a *defect* — the contract is "no fabrication" — but the user-facing message will not mention that the failure is *credential*-related (the underlying `CoreError` text says "Failed to get SSH client: credential store unavailable"). Frontend could surface a hint via `ConnectionRecoveryNotice` for the *session* (already does this via `ConnectionStateChanged`), but the *transfer* row will show the raw text. Acceptable since the session-level banner already explains the cause; if desired, a follow-up could plumb the typed `CoreError` into `error_message` for transfer rows. No spec blocker.

### Finding WB-6 — Persistence-vs-expiry race: no code-level guard for late "trust succeeded" event (Important, follow-up)

- **Severity:** Important (spec §三 acceptance — recorded as follow-up by both implementer and reviewer)
- **Spec clause:** spec §三 "主机密钥决定仍由用户作出，永久信任保存失败时不接受握手"
- **Code location:** `src-tauri/crates/rshell-protocol/src/ssh/client.rs:498-561` (`check_server_key` timeout arm); `src-tauri/crates/rshell-core/src/security/host_key_decision.rs:181-220` (`close`); `src-tauri/crates/rshell-core/src/command_dispatcher.rs:535-594` (`DecideHostKey`); `src-tauri/crates/rshell-core/src/transfer/staged.rs` (parallel for transfers).
- **What:** Traced the full race end-to-end:
  1. User picks "永久信任" via `decideHostKey(decision_id, true, true)`.
  2. Dispatcher: `claim(id, DecidedTrustAlways)` → state DecidedTrustAlways. `trust_host_key(...)` writes to known_hosts. **Time passes here.**
  3. Meanwhile, the SSH protocol's `tokio::time::timeout(self.host_key_timeout, rx)` (60 s) fires if step 2 takes too long. `sink.expire_decision(decision_id)` calls `close()` which sees state=DecidedTrustAlways (allowed by line 196-204 match), overwrites to Expired, **drops the sender** (but it was already removed in step 2 if settle_claimed completed first), publishes `HostKeyDecisionStateChanged{decision_id, state: Expired}`.
  4. If `settle_claimed` had already run before step 3: state is `DecidedTrustAlways`, sender is removed, requests map is cleared. Step 3's `close()` succeeds, overwrites state to Expired, publishes stale Expired event. Frontend's `hostKey.ts:82` `if (!requests.value.has(decision_id)) return;` drops it silently. **`known_hosts` is on disk but no handshake future is woken** (sender is already removed).
  5. If step 3 fires before step 2's `settle_claimed` completed: `close()` would overwrite DecidedTrustAlways → Expired. `settle_claimed` would then fail its CAS check (line 128-130 `states.get(&decision_id) != Some(claimed)`) and return false. Dispatcher returns `Err(NotFound("... no longer pending"))`. The handshake is woken by... nothing. It waits the full 60 s, then the protocol timeout fires.
  6. The frontend already removed the request via `submit()`'s `removeRequest` — but in the **race** path (timeout arrives first), the request may still be in the map when the stale Expired event fires. `hostKey.ts:83` only removes on TERMINAL_STATES, so Expired *would* trigger removal. The user's session list will show a ConnectionRecoveryNotice with `host_key_mismatch` kind (since the handshake was eventually rejected) plus the host-key dialog dismissing itself.
- **The race is real** but bounded:
  - The state machine correctly rejects the handshake (sender is removed, no decision is sent).
  - The known_hosts entry remains on disk.
  - The frontend stores do not display a phantom "Connected" state.
  - The handshake future is rejected either by `connect_stream` seeing `host_key_rejected = true` (line 712-714) or by `ProtocolError::HostKeyMismatch` from `expire_decision`.
- **Evidence:** Reading the full path; the Task 3 reviewer also identified this in Risk #2 of `docs/research/2026-10-08-task3-independent-review.md:230-239`.
- **Recommendation:** Per spec, this is recorded as a follow-up. macOS validation section 4 of `docs/09-macos-validation.md:266` states "下次 connect 弹一次「上次未完成握手留下的信任条目，保留吗？」由用户决策（已记为 follow-up issue，本轮不修）". A code-level guard could be: (a) on the next connect, if the saved `known_hosts` entry's last_seen corresponds to a decision_id that never reached `Decided*`, surface a "stale trust entry" event; (b) or roll back the trust entry to the previous `known_hosts` snapshot. Neither is in this round. Acceptable for development-verification-complete; flagged for next round.

### Finding WB-7 — Cancel/commit arbitration: cancel during commit is correctly suppressed (Note)

- **Severity:** Note (spec §二 explicitly allows this behavior)
- **Code location:** `src-tauri/crates/rshell-core/src/transfer/staged.rs:236-249` (upload path); `:366-376` (download path).
- **What:** When cancel arrives between `safe_commit` returning Ok and the CAS at `:237`, the CAS returns `Err(DECISION_CANCELLED)` but the function still returns `StagedOutcome::Completed`. This is the spec-allowed "cancellation intent swallowed" behavior, verified by the Task 2 reviewer's spot-check on `upload_cancel_during_commit_is_completed_with_target_replaced`.
- **Recommendation:** None.

---

## IPC Contract Reconciliation Coverage

For each new variant / kind / command / field, confirm `tests/unit/ipcContract.spec.ts` covers it.

### IpcErrorKind (8 new Task 3 kinds + 2 Task 1/2 kinds)

`tests/unit/ipcContract.spec.ts:561-587` "Task 3 typed error IPC contract":

| Variant | Stable string | TS mirror | Source-of-truth check |
|---|---|---|---|
| `CredentialMissing` | `credential_missing` | `"credential_missing"` in `types.ts:27` ✓ | Both directions checked: regex in `error.rs`, literal in `types.ts` |
| `CredentialInaccessible` | `credential_inaccessible` | `"credential_inaccessible"` in `types.ts:28` ✓ | Same |
| `CredentialSaveFailed` | `credential_save_failed` | `"credential_save_failed"` in `types.ts:29` ✓ | Same |
| `CredentialMigrationFailed` | `credential_migration_failed` | `"credential_migration_failed"` in `types.ts:30` ✓ | Same |
| `AuthFailed` | `auth_failed` | `"auth_failed"` in `types.ts:31` ✓ | Same |
| `HostKeyMismatch` | `host_key_mismatch` | `"host_key_mismatch"` in `types.ts:26` ✓ | Same |
| `HostKeyTrustPersistenceFailed` | `host_key_trust_persistence_failed` | `"host_key_trust_persistence_failed"` in `types.ts:32` ✓ | Same |
| `TerminalRecoveryRequired` (Task 1) | `terminal_recovery_required` | `"terminal_recovery_required"` in `types.ts:39` ✓ | NOT explicitly enumerated in ipcContract.spec.ts — would silently fail only if someone removed the literal in `types.ts`; however, `kind_strings_are_stable` test in `error.rs:194-196` asserts the Rust side |
| `TargetExists` (Task 2) | `target_exists` | `"target_exists"` in `types.ts:38` ✓ | Same as above — Rust side covered by `kind_strings_are_stable:192`, TS side covered by `core_error_mapping_is_exhaustive:242` and `types.ts:38` literal |

**Coverage verdict:** ✅ All new IPC kinds are covered at the *existence* level by `tests/unit/ipcContract.spec.ts` + `kind_strings_are_stable` + `core_error_mapping_is_exhaustive`. The Task 3 contract test would *catch* a missing Rust enum variant (line 575 regex), and *catch* a missing TS string (line 584 literal). If any new variant were silently dropped from the contract, the test would fail at compile time or runtime.

### AppEvent (2 new Task 3 variants)

| Variant | events.rs | types.ts | Reconciled? |
|---|---|---|---|
| `HostKeyMismatch { decision_id, host, port, key_type, expected, received, public_key_blob }` | `:77-91` | `:523-532` | ✓ ipcContract.spec.ts:482-486 walks all Rust variants vs all TS variants |
| `HostKeyDecisionStateChanged { decision_id, state }` | `:94-97` | `:534-538` | ✓ Same |

The two event variants are part of the 26-variant reconciliation set in `parseRustEnumVariants` / `parseTsAppEventVariants`. Adding either variant on one side without the other fails the test.

### AppCommand (2 new Task 3 commands)

| Variant | commands.rs | types.ts | client.ts helper |
|---|---|---|---|
| `DecideHostKey { decision_id, accept, permanent }` | `:272-276` | `:458` | `decideHostKey` (client.ts:298-299) |
| `CancelHostKey { decision_id }` | `:263-265` | `:459` | `cancelHostKey` (client.ts:300-301) |

Both are picked up by:
- ipcContract.spec.ts:248-265 (frontend call site key reconciliation)
- ipcContract.spec.ts:267-275 (every backend command with args has a frontend caller — `decideHostKey` and `cancelHostKey` both have non-empty arg lists so they would orphan otherwise)
- ipcContract.spec.ts:277-281 (lib.rs generate_handler registration matches commands.rs)

The reverse direction (every helper used in src/) is enforced by `ipcContract.spec.ts:536-558`.

### TransferTaskInfo fields (Task 2)

| Field | TS interface | Rust struct | Reconciled? |
|---|---|---|---|
| `temp_path` | `types.ts:168` | `types.rs:159` | ✓ covered by `TRANSFER_TASK_INFO_R2_T2_FIELDS` in ipcContract.spec.ts:417-421 |
| `cleanup_status` | `types.ts:170` | `types.rs:163` | ✓ same |
| `commit_strategy` | `types.ts:173` | `types.rs:167` | ✓ same |

Plus the structural check at `ipcContract.spec.ts:608-614` asserts both sides have *exactly* the same field set. If anyone added a field to one side without the other, this would fail.

**Coverage verdict:** ✅ All new variants are covered. The contract reconciliation tests are strong enough that any silent field/kind/variant removal or addition on one side would fail at least one assertion.

---

## Cumulative Regression Check

### Task 1 SSH recovery tests

Re-ran `cargo test --workspace`: 70 passed in rshell-protocol, 211 passed in rshell-core, 15 passed in rshell. The Task 1 test families (`recovery_*`) all still green. Specifically:
- `recovery_saturated_primary_queue_has_end_to_end_deadline` (input, 11s cap)
- `recovery_primary_reply_has_deadline` (resize, 11s cap)
- `recovery_close_bypasses_saturated_request_queue` (close, 100ms cap)
- `recovery_disconnect_lock_wait_is_bounded_and_retries_old_generation`
- `recovery_ssh_lock_admission_is_bounded_and_other_connection_is_independent`
- `recovery_expired_and_abandoned_queued_input_never_executes`
- `recovery_stalled_execution_isolated_and_independent_terminal_still_writes`
- `recovery_drop_client_cancels_all_old_terminal_actors`

Plus the frontend TerminalPaneUnmount.spec.ts "SSH uncertain input recovery" set (3 cases) passes in `npm test`.

**No regression found.** Task 2's `staged.rs` and Task 3's `host_key_decision.rs` refactor touched adjacent crates but did not break Task 1 invariants.

### Task 2 staged lifecycle tests

13 staged tests + 3 retry tests in `rshell-core/src/transfer/staged.rs` and `service.rs` all pass. Plus `transferItem.spec.ts` (8 cases) and `TransferPanel.spec.ts` R2-T2 section (4 cases).

**No regression found.**

### Task 3 credential / host-key tests

`missing_and_inaccessible_stored_credentials_keep_distinct_categories`, `credential_save_failure_is_typed_and_never_publishes_connected`, `successful_credential_save_then_wrong_password_returns_auth_failed`, `permanent_trust_failure_keeps_handshake_pending_for_session_scoped_fallback`, `concurrent_decisions_are_keyed_and_one_resolution_cannot_clear_the_other`, `cancellation_and_expiry_close_only_their_decision_and_drop_late_events`. All 211 rshell-core tests pass.

Frontend: `ConnectionRecoveryNotice.spec.ts` (8 cases covering all 8 kinds), `SessionListSecurity.spec.ts` (5 cases including credential save failure), `hostKeyStore.spec.ts` (8 cases covering concurrent / cancel / late events). All pass.

**No regression found.**

### Dependency wiring

- `async-trait = "0.1"` workspace dep added at `src-tauri/Cargo.toml:58`. Used in:
  - `rshell-core/src/transfer/test_sink.rs:127` (FakeSink as TransferSink)
  - `rshell-core/src/transfer/staged.rs` (StagedRequest callbacks)
  - `rshell-protocol/src/transfer/local_sink.rs` (LocalSink)
  - `rshell-protocol/src/ssh/sftp.rs` (TransferSink)
- `#[allow(clippy::double_must_use)]` attributes added at 7 sites in rshell-core and rshell-protocol. clippy exit 0 confirms these are needed and applied correctly.
- `ssh_key` and `getrandom::SysRng` (Task 3 RSA + ssh-key 0.7 changes noted in `09-macos-validation.md` 2026-10-08 CI root-cause section). All passing in `cargo test --workspace`.

**No cumulative regression found.**

---

## macOS Acceptance Honesty

Reading `docs/09-macos-validation.md:221-282` (the 2026-10-08 reliability round section):

| Claim | Verdict |
|---|---|
| "Task 1 之前的 baseline commit 为 86c6ac0；本轮最终 HEAD 为 6445486" | **Wrong — outdated.** Final HEAD is `a50245e` (Task 4's `docs:` commit), not `6445486`. This is a documentation staleness issue: the table at line 230-234 lists Task 4's row as "待独立全分支审查" implying that hadn't happened, but the plan-execution status block at the bottom of the file does correctly identify Task 4 as the integration verification step. Recommendation: update the table to identify HEAD `a50245e` and mark Task 4 as `已完成(本报告)` once this whole-branch review is committed. |
| "未推送、未合并、未发布" | **Accurate.** git log shows 6 commits ahead of the round base, branch state confirmed by `git status`. |
| "38 文件 / 313 passed" | **Accurate.** Independently reproduced. |
| "rshell-protocol 70、rshell-core 211、rshell 15、其他 crate 全 0 失败" | **Accurate** for protocol/core/rshell. My run shows additional crates passing (rshell-api 6, rshell-infra 18, rshell-plugin-sdk 10, xtask 4) but that's just a different aggregation. |
| "新增日志格式串经 git grep 不含 password/passphrase/secret 字面量" | **Accurate.** Independently reproduced (0 matches). |
| "SFTP 提交走标准 SSH_FXP_RENAME（不能保证原子）" | **Accurate.** Confirmed by reading `sftp.rs:613-629` `safe_commit` and `staged.rs:210-234`. |
| "持久化 vs 过期 race: 仅防御性「过期不再唤醒」+ 下次 connect 弹一次「上次未完成握手留下的信任条目，保留吗？」" | **Honest about being follow-up.** Documented as "已记为 follow-up issue，本轮不修". No false claim of completion. |
| "macOS 原生钥匙串...真实 SSH/SFTP 服务器...物理串口...签名与公证" | **All correctly marked as not executed.** Listed under "仍待真实环境验收" with no claim of having run. |

**Verdict:** ✅ The doc is honest about what was not verified. The only inaccuracy is the stale "Task 4 = 待独立全分支审查" / "本轮最终 HEAD 为 6445486" rows, which need a follow-up doc edit once this whole-branch review is committed. This is a documentation-only update, not a code issue.

---

## Persistence-vs-Expiry Follow-up Status

**There is no code-level guard** for the late-arriving "trust succeeded" event after the handshake has been cancelled/expired. The only guards present:

1. **State machine level** (`host_key_decision.rs:103-146`): `settle_claimed` requires `states.get(&decision_id) == Some(claimed)` before sending the decision. If the state has been transitioned by `close()` (line 196-204) to Cancelled/Expired, `settle_claimed` returns false → sender is NOT sent → handshake future stays parked until the protocol timeout (60 s) fires.

2. **Sender-removal level**: After `settle_claimed` succeeds, the sender is removed from `inner` (line 132-136). A subsequent `close()` (from `expire_decision`) finds no sender to drop, but the state has already been overwritten to terminal. No double-send possible.

3. **Frontend level** (`hostKey.ts:78-84`): `HostKeyDecisionStateChanged` for unknown ids is dropped silently via `if (!requests.value.has(decision_id)) return;`. Terminal states only remove the matching id.

**There is NO guard against the `known_hosts` disk entry persisting** when `trust_host_key` succeeds but `settle_claimed` fails because the protocol timeout fired first. The state machine correctly rejects the handshake, but the trust entry is on disk.

The implementer's doc (`docs/09-macos-validation.md:266`) records this as a follow-up and proposes a UX-level mitigation for the *next* connect: show "上次未完成握手留下的信任条目，保留吗？". This is documented-only; no code change is in this round.

**Verdict:** This is a *documented* follow-up, not a hidden defect. The spec explicitly says "永久信任保存成功后才接受握手" — and the state machine does correctly reject the handshake in this race. The on-disk trust entry is the only artifact that "leaks" beyond the canceled handshake; it does not cause a fabricated Connected. Acceptable for the current round's "development verification complete" milestone; tracked for next round.

---

## Spec Coverage Matrix

### Spec §一 — SSH 请求有界与终端恢复

| Acceptance criterion | Test | Status |
|---|---|---|
| 对端停止读取、队列饱和时，输入/resize/关闭/断开均在预算内返回 | `recovery_saturated_primary_queue_has_end_to_end_deadline`, `recovery_primary_reply_has_deadline`, `recovery_close_bypasses_saturated_request_queue`, `recovery_disconnect_lock_wait_is_bounded_and_retries_old_generation`, `recovery_ssh_lock_admission_is_bounded_and_other_connection_is_independent` (rshell-protocol, rshell-core) | ✅ GREEN |
| 一次输入超时即可看到持续错误和恢复入口；结果不确定时文案不声称未执行 | `tests/unit/TerminalPaneUnmount.spec.ts` "SSH uncertain input recovery / one uncertain input stops keyboard, backspace and paste until real reconnect finishes"; `terminal_recovery_required` 文案 "输入结果不确定" | ✅ GREEN |
| 已过期且尚未开始的输入不会在对端恢复后被悄悄发送 | `recovery_expired_and_abandoned_queued_input_never_executes`, `recovery_stalled_execution_isolated_and_independent_terminal_still_writes` | ✅ GREEN |
| 手动重连成功后输入恢复，旧终端请求不进入新终端；失败时状态不伪装为可用 | `tests/unit/TerminalPaneUnmount.spec.ts` "failed reconnect and late successful IO cannot clear uncertain recovery" | ✅ GREEN |
| 重复关闭、超时与关闭竞争、关闭后晚到事件有回归覆盖 | `recovery_close_bypasses_saturated_request_queue`, `recovery_disconnect_lock_wait_is_bounded_and_retries_old_generation`, `recovery_drop_client_cancels_all_old_terminal_actors` | ✅ GREEN |
| 其他可用连接不会被故障终端阻塞 | `recovery_ssh_lock_admission_is_bounded_and_other_connection_is_independent`, `recovery_stalled_execution_isolated_and_independent_terminal_still_writes` | ✅ GREEN |

**Spec §一 status:** All 6 criteria covered. No gap.

### Spec §二 — SFTP 临时文件与最终提交

| Acceptance criterion | Test | Status |
|---|---|---|
| 上传与下载均覆盖新目标和明确授权覆盖目标；成功后内容一致，无属于该任务的临时文件残留 | `upload_to_fresh_target_succeeds_and_cleans_up_temp`, `upload_with_overwrite_replaces_existing_target`, `download_to_fresh_local_target_succeeds_and_cleans_up_temp`, `staging_temp_path_uses_task_uuid_suffix_and_same_directory`, `staging_local_temp_path_keeps_parent_and_appends_uuid` | ✅ GREEN |
| 写入失败、断网、磁盘或权限失败、取消时，已有最终目标内容保持不变 | `upload_commit_failure_preserves_target_and_reports_cleanup_status`, `upload_cancel_before_commit_preserves_target_and_cleans_temp`, `download_cancel_before_commit_preserves_local_target`, `upload_exclusive_collision_on_same_target_fails`, `download_commit_failure_preserves_local_target` | ✅ GREEN |
| 临时残留与清理失败被真实报告 | `upload_commit_failure_and_cleanup_failure_both_reported` + `transferItem.spec.ts:56-69` "failed task with residue: error includes the temp path" + `cleanup_status` 字段 + `commit_strategy` 字段 | ✅ GREEN |
| 最终提交失败不发布完成事件，原目标仍受保护 | `finalize_staged` Failed 分支 — 只 publish `TransferFailed`，不 publish `TransferCompleted` (service.rs:1036-1062); `upload_commit_failure_preserves_target_and_reports_cleanup_status` 断言 target 未被改写 | ✅ GREEN |
| 目标在预检后出现、清理失败、取消与提交竞争均有定向回归 | `upload_late_conflict_fails_with_no_target_change`, `upload_commit_failure_and_cleanup_failure_both_reported`, `upload_cancel_during_commit_is_completed_with_target_replaced` | ✅ GREEN |
| 重试、暂停/继续、取消和移除记录的界面动作与实际任务状态一致 | `retry_rejects_unknown_task`, `retry_rejects_non_terminal_task`, `retry_creates_new_task_from_zero_with_fail_policy_and_keeps_original_terminal`; `transferItem.spec.ts` 4 cases; `TransferPanel.spec.ts:147-194` (loading); `AppLayout.spec.ts:146-150` TransferTaskInfo 透传 | ✅ GREEN |
| UI surfaces commit_strategy | `TransferPanel.spec.ts:196-242` 4 cases; `transferItem.spec.ts:83-99` "Completed task surfaces commit_strategy" | ✅ GREEN (resolved in `28ba8df`) |

**Spec §二 status:** All 7 criteria covered. No gap.

### Spec §三 — 凭据恢复与错误反馈

| Acceptance criterion | Test | Status |
|---|---|---|
| 缺失条目 → 可见原因 → 更新凭据 → 保存成功 → 手动重连 | `SessionListSecurity.spec.ts:32-89` (round-trip), `missing_and_inaccessible_stored_credentials_keep_distinct_categories`, `credential_save_failure_is_typed_and_never_publishes_connected` | ✅ GREEN |
| 钥匙串拒绝访问、保存失败、旧配置迁移失败均显示真实原因并保留恢复路径 | `failed_migration_preserves_original_bytes_and_omits_session`, `migration_metadata_failure_preserves_exact_bytes_and_rolls_back_keychain`, `migration_issues_survive_startup_and_retry_without_exposing_secrets`, `credential_save_failure_is_typed_and_never_publishes_connected`, `SessionListSecurity.spec.ts` storage load issues → 重试加载 | ✅ GREEN |
| 修改凭据后再次认证失败时不宣称已连接 | `successful_credential_save_then_wrong_password_returns_auth_failed` (real russh server) | ✅ GREEN |
| 取消编辑不改写秘密 | `SessionListSecurity.spec.ts:91-122` "keeps a failed credential save visible and preserves the entered value until dismissal" | ✅ GREEN |
| 原有主机密钥确认、永久信任落盘与迁移保留行为保持回归覆盖 | `trust_host_key_only_persists_on_trust_permanent` (PROB-06), `permanent_trust_failure_keeps_handshake_pending_for_session_scoped_fallback`, `session/repository.rs` migration tests | ✅ GREEN |
| 同时连接多个未知主机、确认期间取消连接、确认超时、提交决策与新请求并发 | `concurrent_decisions_are_keyed_and_one_resolution_cannot_clear_the_other`, `cancellation_and_expiry_close_only_their_decision_and_drop_late_events`, `hostKeyStore.spec.ts:keeps concurrent requests in a map`, `host_key_timeout_cancels_the_handshake_future_instead_of_waiting_for_ui`, `hostKeyStore.spec.ts:keeps the dialog open and records the error when the decision fails` | ✅ GREEN |
| 一个请求的提交结果不能清除其他仍有效请求 | `hostKeyStore.spec.ts:drops state events for unknown or expired ids without clearing valid decisions`, `concurrent_decisions_are_keyed_and_one_resolution_cannot_clear_the_other` | ✅ GREEN |

**Spec §三 status:** All 7 criteria covered. No gap.

### Spec §四 — 验收矩阵与完成状态

| Layer | Required evidence | This round's evidence | Status |
|---|---|---|---|
| Controllable automation: actor 停滞、队列饱和、超时请求迟到、关闭与重连竞争 | Repeatable regression | Task 1's 9 SSH recovery tests | ✅ GREEN |
| Controllable automation: 上传下载的失败、取消、冲突、提交及清理失败 | File content + residue assertions, status event assertions | Task 2's 16 Rust tests + 9 vitest tests | ✅ GREEN |
| Controllable automation: 凭据缺失、存储不可访问、迁移和认证失败、恢复入口 | Core + UI targeted regression | Task 3's credential + host-key Rust tests + SessionListSecurity + ConnectionRecoveryNotice + hostKeyStore specs | ✅ GREEN |
| Repository checks: 现有前后端检查和依赖审计 | npm run verify + npm run audit results | `cargo fmt`/`clippy`/`test --workspace` all exit 0; `npm test` 38/313; `npm run typecheck` exit 0; `npm run check:docs` exit 0 | ✅ GREEN |
| macOS real-device: 真实 SSH 登录、输入、重连、多标签关闭与隔离 | Environment, steps, phenomena, results recorded | **NOT EXECUTED.** Implementation report acknowledges. macOS validation section explicitly lists as "未执行". | ⏳ DEFERRED per spec |
| macOS real-device: 真实 SFTP 上传下载、授权覆盖、失败取消、临时残留 | Content + path + result recorded | **NOT EXECUTED.** Same. | ⏳ DEFERRED per spec |
| macOS real-device: 钥匙串提示、旧配置迁移、缺失条目更新与重连 | No-secret operations + results recorded | **NOT EXECUTED.** Same. | ⏳ DEFERRED per spec |

**Spec §四 status:** 4 of 7 layers GREEN; 3 layers DEFERRED per spec's two-tier acceptance. Acceptable.

---

## Concerns Summary

1. **Stale HEAD reference in `docs/09-macos-validation.md`** (Minor doc issue, line 236 says "本轮最终 HEAD 为 6445486" — should be `a50245e`). Cosmetic.
2. **Persistence-vs-expiry race** (Important follow-up, documented but no code-level guard against on-disk `known_hosts` residue after a race). Both implementer and Task 3 reviewer flagged this; recorded as next-round issue.
3. **In-flight credential failure surfaces as raw "Failed to get SSH client: ..."** for transfer rows (Important UX detail, not a defect). The session-level banner explains the credential cause via `ConnectionRecoveryNotice`; the transfer row text is generic.
4. **Late frontend dismissal duplicates an in-flight cancel** was the Task 3 reviewer's Risk #3, **fixed in `6445486`**.

None of these block development verification complete. The branch is internally sound; remaining gaps are macOS real-device acceptance, which the spec allows to be deferred.

---

## Final Note

The three per-task reviewers returned `DONE_WITH_CONCERNS` and the round's design follows the RED-first, implementer → reviewer → review-fix pattern. This whole-branch review confirms:

- All three reviewers' `Critical` / `Notable` / `Important` findings have been resolved in follow-up commits.
- All three reviewers' Minor / Note concerns have been either resolved or honestly carried as documented follow-ups.
- No Task 1 behavior regressed after Tasks 2 and 3.
- The IPC contract surface is fully reconciled at the variant/kind/command/field level.
- Failure-mode fabrication checks all pass (no `Connected` is published when it shouldn't be).
- End-to-end user paths traced through actual production code (not just tests) verify the spec contract.

The branch is ready to be marked **development-verification-complete** pending the macOS real-device acceptance cycle, which is the spec's explicit second-tier gate.

**Recommended next actions (post-merge, out of this round):**
1. Update `docs/09-macos-validation.md:236` to reflect final HEAD `a50245e` and mark Task 4's status.
2. Plan a follow-up for the persistence-vs-expiry race (proposed UX: surface a "stale trust entry" event on next connect).
3. Optional: tighten `transferItem.ts` to surface typed `CoreError` cause for `Failed` transfer rows that originated from SSH-client-provider errors (currently shown as raw "Failed to get SSH client: ..." text).