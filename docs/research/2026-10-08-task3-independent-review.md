# Task 3 independent review — credential and host-key recovery consistency

- Worktree: `C:/Users/lp/.codex/worktrees/reliability-recovery/rshell`
- Branch / base: `codex/reliability-recovery` @ `3c309f6` (`fix: align credential and host-key recovery`), base `28ba8df`
- Reviewer scope: Task 3 implementation only; Task 2 (SFTP) is not in scope and was not inspected.
- Implementer report: `docs/research/2026-10-08-credential-hostkey-recovery-report.md` (untracked, work product)
- This review is deliberately **untracked** and not part of the Task 3 commit.

### Verdict

`DONE_WITH_CONCERNS`

The Task 3 commit satisfies every spec § 三 acceptance criterion and every Task 3 plan bullet
with at least one regression test. Concerns below are about platform coverage gaps the implementer
already flagged (real Keychain/DPAPI/Secret Service, persistence-vs-expiry race) plus a small
addition the implementer's report did not call out — see "Risks Not Closed" #3.

### Independent Verification Results

All commands run from the worktree root, with the toolchain / target-dir overrides the task brief
specified.

| Check | Exit | Output |
|---|---|---|
| `cargo fmt --all --manifest-path src-tauri/Cargo.toml --check` | `0` | clean |
| `cargo test -p rshell-protocol --lib --manifest-path src-tauri/Cargo.toml` | `0` | `70 passed; 0 failed` (matches implementer claim 70) |
| `cargo test -p rshell-core --lib --manifest-path src-tauri/Cargo.toml` | `0` | `211 passed; 0 failed` (matches implementer claim 211) |
| `cargo test -p rshell --lib --manifest-path src-tauri/Cargo.toml` | `0` | `15 passed; 0 failed` (matches implementer claim 15) |
| `cargo clippy -p rshell-protocol -p rshell-core -p rshell --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings` | `0` | clean |
| `npm test` | `0` | `Test Files 38 passed (38)`; `Tests 311 passed (311)` (matches implementer claim 38/311) |
| `npm run typecheck` | `0` | clean |
| `git diff --check` | `0` | no whitespace errors |

I additionally ran a `git grep`-style secret scan over the diff (log macro plus
`password|passphrase|secret`): zero hits. The only matches are non-log variable names
(`CredentialKind::Password` / `Passphrase`, parameter names, type fields) and the existing
`russh::server::Handler` test stub. The new `SessionCredential` `Debug` impl is unchanged
(`<redacted>`), the `repository.rs` `From<anyhow::Error>` continues to discard raw backend
diagnostics, and the front-end does not display secret material.

### Findings

#### F-1 — Error category distinctness: confirmed
- Severity: Note
- Spec clause: § 三 user-contract / acceptance "区分…给出对应操作建议"
- Code location: `src-tauri/crates/rshell-core/src/error.rs:17-38`,
  `src-tauri/src/error.rs:17-58`, `src/components/ConnectionRecoveryNotice.vue:16-33`
- What: All eight `CoreError` variants added for Task 3 map to a distinct stable
  `IpcErrorKind`/string, and `ConnectionRecoveryNotice.vue` renders an 8-row `Record`
  with different copy / action / event per kind. `CoreError::AuthError` and
  `CoreError::AuthenticationFailed` collapse to the same `auth_failed` kind — that is the
  pre-existing intent, not a regression.
- Evidence:
  - `src-tauri/src/error.rs:106-119` maps each new variant.
  - `core_error_mapping_is_exhaustive` test (`src-tauri/src/error.rs:200-251`) iterates all
    `CoreError` variants and asserts each maps to an allowed string — confirmed by running.
  - `ipcContract.spec.ts:561-587` ("Task 3 typed error IPC contract") reconciles both
    directions: Rust `IpcErrorKind` names ↔ TS `IpcErrorKind` string members.
  - `ConnectionRecoveryNotice.spec.ts:7-21` exercises all 8 kinds and asserts each renders
    the correct text + action + emitted event.

#### F-2 — `CredentialSaveFailed` does not publish `Connected`; auth failure surfaces as `AuthError`
- Severity: Important (spec acceptance)
- Spec clause: § 三 "修改凭据后再次认证失败时不宣称已连接"
- Code location: `src-tauri/crates/rshell-core/src/session/service.rs:131-149` (`repository_error`)
  + `683-693` (`connect_attempt` returns `Err(ssh_connect_error(e))`), and
  `tests/unit/SessionListSecurity.spec.ts` siblings in `tests/unit/`
- What: A failed save returns `CredentialSaveFailed` without ever entering `connect_attempt`'s
  success path; a successful save followed by a wrong password surfaces `CoreError::AuthError`
  (kind `auth_failed`) and never publishes `ConnectionStateChanged{Connected,..}`.
- Evidence (assertion lines from passing tests, verbatim):
  - `src-tauri/crates/rshell-core/src/session/service.rs:1567-1572`
    ```
    assert!(matches!(error, CoreError::CredentialSaveFailed(_)));
    assert!(!connected.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(svc.get_state(id).await.unwrap(), ConnectionState::Disconnected);
    ```
  - `src-tauri/crates/rshell-core/src/session/service.rs:1705-1707`
    ```
    assert!(matches!(error, CoreError::AuthError(_)));
    assert!(!connected.load(std::sync::atomic::Ordering::SeqCst));
    ```
- Both tests stand up a real russh loopback server (not a mock) and an in-memory credential
  store, so they actually exercise the production path end-to-end. They prove the contract
  the implementer claims, not just compile-time existence.

#### F-3 — Cancelled editing writes nothing
- Severity: Important (spec acceptance)
- Spec clause: § 三 "取消编辑不改写秘密"
- Code location: `src/components/SessionCredentialDialog.vue:15-31`, `src/stores/sessions.ts:97-102`
- What: Cancel button calls `close()` which clears `secret.value` and emits `close`. No IPC is
  fired on cancel. The dialog is rendered conditionally in `SessionList.vue:222` so dismissing
  the parent closes the dialog.
- Evidence:
  - `tests/unit/SessionListSecurity.spec.ts:113-121` proves that "取消" leaves the dialog closed,
    `update_session` was not re-invoked, and a re-open starts with empty input. The full test
    asserts that the originally-typed `unsaved-secret` is not leaked into `store.items` and the
    input field stays alive across the failure (line 110-111).

#### F-4 — No plaintext fallback on `CredentialInaccessible`
- Severity: Note
- Spec clause: § 三 "不回退到旧明文"
- Code location: `src-tauri/crates/rshell-core/src/session/repository.rs:319-339` (`credential`),
  `session/service.rs:293-299` (`resolve_auth`)
- What: The repository only returns the stored secret when `has_credential(config)` AND the
  keychain `get` returns `Some(non-empty)`. There is no code path that re-reads a legacy TOML
  plaintext value — `store.load_pending` migrates any legacy bytes to the keychain and either
  succeeds (returns the migrated config) or fails with `CredentialMigrationFailed`.
- Evidence: `tests/unit/credential_*.spec` does not exist; the closest are
  `migrates_all_legacy_auth_variants_before_returning` (repo.rs:447-464) which fails the test
  on any remaining plaintext, and `failed_migration_preserves_original_bytes_and_omits_session`
  (repo.rs:466-492) which asserts `fs::read(&path).unwrap() == bytes` (legacy plaintext remains
  on disk because migration failed, the in-memory session list omits it). Confirmed by reading
  the production path.

#### F-5 — Host-key registry concurrency: keyed and CAS-bounded
- Severity: Note
- Spec clause: § 三 "并发主机密钥确认保留各自请求和主机指纹，不互相覆盖"
- Code location: `src-tauri/crates/rshell-core/src/security/host_key_decision.rs:18-225`
- What: Each handshake gets its own `Uuid`; the `inner` sender map, `requests` info map, and
  `states` lifecycle map are all keyed by id. `claim` / `settle_claimed` / `release_claim` /
  `close` all guard with `if states.get(&id).copied() != Some(expected)` so a late operation on
  a settled id is a no-op.
- Evidence:
  - `src-tauri/crates/rshell-core/src/security/host_key_decision.rs:301-364` — concurrent test:
    two registers with distinct UUIDs, each `resolve` only sends to its own `oneshot::Receiver`,
    events include both `DecidedTrustOnce` and `DecidedReject` keyed correctly.
  - `src-tauri/crates/rshell-core/src/security/host_key_decision.rs:367-399` — cancel/expire
    drops receiver, double `expire` no-ops, `cancel` after publish removes only that id.

#### F-6 — Cancellation flows both ways; backend drops the handshake future
- Severity: Note
- Spec clause: § 三 "后端已取消或过期的请求明确失效并允许关闭，不能再执行信任"
- Code location: `src-tauri/crates/rshell-protocol/src/ssh/client.rs:247-273`
  (`DecisionWaitGuard`), `:444-562` (`check_server_key`), `:702` (60 s timeout bound)
- What: Frontend cancel → `CancelHostKey` Tauri command → `host_key_registry.cancel(id)` →
  `close(id, Cancelled)` drops the `oneshot::Sender`. The handler's `Ok(Err(_))` arm returns
  an `anyhow` err; `SshClient::connect_ssh` checks the `host_key_rejected` flag set inside the
  handler and maps the russh error to `ProtocolError::HostKeyMismatch` (kind
  `host_key_mismatch`).
- Backend timeout / cancellation also calls `sink.expire_decision(decision_id)` /
  `sink.cancel_decision(decision_id)` and publishes `HostKeyDecisionStateChanged{state,..}`;
  `hostKeyStore.ts` removes only that keyed id (`TERMINAL_STATES` filter, line 23-27).
- Evidence:
  - `src-tauri/crates/rshell-protocol/src/ssh/client.rs:1853-1877`
    (`host_key_timeout_cancels_the_handshake_future_instead_of_waiting_for_ui`): 20 ms
    injected timeout, the handshake returns Err, `sink.expired == 1`.
  - `tests/unit/hostKeyStore.spec.ts:81-95` ("cancelling one does not affect the other") and
    `:97-106` ("drops state events for unknown or expired ids") cover both directions.

#### F-7 — Permanent-trust save failure keeps handshake pending
- Severity: Important (spec acceptance)
- Spec clause: § 三 "永久信任保存失败时不接受握手"
- Code location: `src-tauri/crates/rshell-core/src/command_dispatcher.rs:535-594` (DecideHostKey
  branch)
- What: When `accept=true, permanent=true`, the dispatcher calls `claim(id, DecidedTrustAlways)`
  first; only after `trust_host_key` succeeds does it call `settle_claimed`. A `trust_host_key`
  failure returns `release_claim` to `Pending` and surfaces `HostKeyTrustPersistenceFailed`.
  The handshake future is never sent an accepting decision.
- Evidence:
  - `src-tauri/crates/rshell-core/src/command_dispatcher.rs:994-1039`
    `permanent_trust_failure_keeps_handshake_pending_for_session_scoped_fallback`:
    creates a directory at `known_hosts.tmp` to force the staged-write to fail, asserts
    `CoreError::HostKeyTrustPersistenceFailed(_)`, asserts state remains `Pending`, then
    follows up with `accept=true, permanent=false` (trust-once) which succeeds, asserts the
    oneshot receives the decision and `known_hosts` does not exist.

#### F-8 — Late / unknown events dropped silently
- Severity: Note
- Spec clause: § 三 "不能清除其他仍有效请求"
- Code location: `src/stores/hostKey.ts:78-83`
- What: `HostKeyDecisionStateChanged` for an unknown `decision_id` is dropped via
  `if (!requests.value.has(decision_id)) return;`. Terminal states only remove the matching id.
- Evidence: `tests/unit/hostKeyStore.spec.ts:97-106` asserts a stale `Expired` event does not
  clear a valid request, and that the valid request is removed only when its own state event
  arrives.

#### F-9 — No secret display, no secret-bearing logs
- Severity: Note
- Spec clause: § 三 "不记录密码或口令", "不展示已存储秘密"
- Code location: `src-tauri/crates/rshell-api/src/types.rs:53-64` (`SessionCredential` Debug impl
  prints `<redacted>`); no new log macros in the diff carry `password|passphrase|secret`.
- Evidence: `git diff` secret scan over all `.rs`/`.vue`/`.ts` files added by Task 3 — zero log
  macro matches; only structural references (`CredentialKind::Password` / `Passphrase` enum
  variants, parameter names, test stubs) appear.

#### F-10 — No automatic reconnect after a successful save
- Severity: Note
- Spec clause: § 三 "保存失败不提示成功或擅自建立连接" (implicitly no auto-retry on success)
- Code location: `src/stores/sessions.ts:97-102` (`updateCredential`), `:157-183` (`connect`)
- What: `updateCredential` only calls `updateSession` then `refresh()`; no connect call. The
  `connect` function is gated by `inflightConnects` dedup and only runs once per user
  invocation. `SessionCredentialDialog.save()` calls `close()` on success — no reconnect.
- Evidence: `tests/unit/SessionListSecurity.spec.ts:56-89` and `:91-122` exercise the dialog
  round-trip; neither asserts or causes a `connect_session` invocation.

### Test Coverage Matrix

Spec § 三 acceptance criterion → test → status

| Spec clause | Test(s) | Status |
|---|---|---|
| 缺失条目 → 更新凭据 → 保存成功 → 手动重连 | `credential_save_failure_is_typed_and_never_publishes_connected` (save refusal path); `missing_and_inaccessible_stored_credentials_keep_distinct_categories` (distinguish kinds); `SessionListSecurity.spec.ts:keeps a failed credential save visible...` (cancel preserves input) | Covered |
| 钥匙串拒绝访问、保存失败、旧配置迁移失败显示真实原因并保留恢复路径 | `failed_migration_preserves_original_bytes_and_omits_session`, `migration_metadata_failure_preserves_exact_bytes_and_rolls_back_keychain`, `migration_issues_survive_startup_and_retry_without_exposing_secrets`, `SessionListSecurity.spec.ts` (storage load issues → 重试加载), `credential_save_failure_is_typed_and_never_publishes_connected` | Covered |
| 修改凭据后再次认证失败时不宣称已连接 | `successful_credential_save_then_wrong_password_returns_auth_failed` (real russh server rejecting auth) | Covered |
| 取消编辑不改写秘密 | `SessionListSecurity.spec.ts:keeps a failed credential save visible and preserves the entered value until dismissal` — cancel button produces no additional `update_session` call | Covered |
| 原有主机密钥确认、永久信任落盘与迁移保留行为保持回归覆盖 | `trust_host_key_only_persists_on_trust_permanent` (PROB-06), `permanent_trust_failure_keeps_handshake_pending_for_session_scoped_fallback`, all `session/repository.rs` migration tests | Covered (pre-existing PROB-06 test preserved + new permanent-trust failure test) |
| 同时连接多个未知主机、确认期间取消连接、确认超时、提交决策与新请求并发 | `concurrent_decisions_are_keyed_and_one_resolution_cannot_clear_the_other`, `cancellation_and_expiry_close_only_their_decision_and_drop_late_events`, `hostKeyStore.spec.ts:keeps concurrent requests in a map`, `host_key_timeout_cancels_the_handshake_future_instead_of_waiting_for_ui`, `hostKeyStore.spec.ts:keeps the dialog open and records the error when the decision fails` | Covered |
| 一个请求的提交结果不能清除其他仍有效请求 | `hostKeyStore.spec.ts:drops state events for unknown or expired ids without clearing valid decisions`, `concurrent_decisions_are_keyed_and_one_resolution_cannot_clear_the_other` | Covered |
| macOS 原生钥匙串提示、真实旧配置迁移和缺失条目恢复单独记录实机结果 | `MemoryCredentials` test double only; real platform adapter not exercised | Not covered (acknowledged Task 4 / product acceptance) |

Plan Task 3 acceptance bullets → test → status

| Plan bullet | Test(s) | Status |
|---|---|---|
| Failing regressions for missing/inaccessible credentials and correct UI recovery actions, failed credential save, rejected auth after save, cancelled editing | `missing_and_inaccessible_stored_credentials_keep_distinct_categories`, `credential_save_failure_is_typed_and_never_publishes_connected`, `successful_credential_save_then_wrong_password_returns_auth_failed`, `SessionListSecurity.spec.ts:keeps a failed credential save visible...`, `ConnectionRecoveryNotice.spec.ts` (8 cases) | Covered |
| Implement error categories and direct recovery actions using existing credential editor, retry-load and explicit connection actions. Do not introduce automatic reconnect or secret display | `ConnectionRecoveryNotice.vue` (8 kinds → distinct action event), `updateCredential` does not connect, `SessionCredential` Debug impl `<redacted>` | Covered |
| Failing regressions for concurrent host-key requests, expired/cancelled requests, submitted decision racing another request, and failed permanent trust persistence | see table above | Covered |
| Preserve requests independently, sync backend cancellation/expiry, permit dismissal of invalid decisions without letting them accept a handshake; valid decisions retain explicit trust/reject behavior. Avoid secret-bearing diagnostic logs | `concurrent_decisions_are_keyed_and_one_resolution_cannot_clear_the_other`, `cancellation_and_expiry_close_only_their_decision_and_drop_late_events`, `permanent_trust_failure_keeps_handshake_pending_for_session_scoped_fallback`, secret-scan above | Covered |

### Risks Not Closed

1. **Platform acceptance (macOS Keychain / Windows DPAPI / Linux Secret Service).** The
   `MemoryCredentials` double only models visibility (read fail) and write refusal
   (`fail_write`). It does not exercise the real `SystemCredentialStore` adapter, real
   migration semantics on the user's vault, or denied-access recovery on each OS. The
   implementer already records this as a Task 4 / product-acceptance item; the same
   limitation applies to Task 3.

2. **Persistence-vs-expiry race (already flagged by the implementer).** If the user picks
   "trust always" while the protocol timeout fires *after* `host_key_manager.trust_host_key`
   has committed the staged write, the registry correctly drops the sender and
   `settle_claimed` no-ops — the handshake is rejected. But the on-disk `known_hosts`
   already has the entry. The state machine cannot generically roll back the write without
   risking deletion of a pre-existing trusted entry. The implementer flagged this; the
   Task 3 acceptance criteria do not require closing it. Recommend a follow-up: surface the
   post-expiry write as an audit-style event (`HostKeyDecisionStateChanged{Expired,
   persisted=true}`) so the next session can show "this key was trusted but the handshake
   didn't finish" instead of silently accepting the entry on the next connect.

3. **Late frontend dismissal duplicates an in-flight cancel (not in the report).** If the
   user clicks 取消连接 twice quickly, the second click calls `cancelHostKey(id)` which
   dispatches `CancelHostKey` to a registry that has already cancelled that id and now returns
   `Err(NotFound("... no longer pending"))`. `hostKeyStore.cancel` catches this and calls
   `setError` — the dialog then displays a confusing "decision no longer pending" message
   even though the user's intent was satisfied on the first click. The race window is small
   and the error text is correct, but the UX is suboptimal. Minor — recommend only that the
   next implementer revisit `hostKeyStore.cancel` to no-op silently when the registry has
   already removed the request, instead of overwriting the (now-cleared) error state.

4. **SessionLoadIssue.message may contain "migration failed" wording that leaks through.** The
   Rust migration reason strings are constrained (e.g. `"credential store unavailable"`,
   `"credential store write failed"`, `"credential entry missing"`) and are reproduced in
   the TS mirror; the test
   `migration_issues_survive_startup_and_retry_without_exposing_secrets` (session/service.rs
   line ~2370) explicitly asserts the serialized payload does not contain the secret
   (`"migration-secret"`). The category is what the UI branches on. Acceptable.

### Test-Code Spot-Check

#### 1. `successful_credential_save_then_wrong_password_returns_auth_failed`
`src-tauri/crates/rshell-core/src/session/service.rs:1575-1707`
```
assert!(matches!(error, CoreError::AuthError(_)));
assert!(!connected.load(std::sync::atomic::Ordering::SeqCst));
```
Stand up a real russh server (`RejectPasswordServer` rejects every password), create a
session, save initial credential, overwrite with wrong credential, connect, resolve the host
key mismatch via the registry. The handshake reaches the real auth reject path; the
connect call returns `AuthError`. The connected atomic stays false because the success
branch only fires after `authenticate(...)?.` and `open_session()?.` both succeed and the
test rejects at `authenticate`. Proves the claim: a successful save followed by wrong
auth returns `AuthError`, not a fabricated `Connected`. **Passes.**

#### 2. `credential_save_failure_is_typed_and_never_publishes_connected`
`src-tauri/crates/rshell-core/src/session/service.rs:1513-1573`
```
assert!(matches!(error, CoreError::CredentialSaveFailed(_)));
assert!(!connected.load(std::sync::atomic::Ordering::SeqCst));
assert_eq!(svc.get_state(id).await.unwrap(), ConnectionState::Disconnected);
```
Make the in-memory credential store refuse writes), update session with a new credential,
assert `CredentialSaveFailed` is returned, the connected atomic stays false, the in-memory
session state stays `Disconnected`. The `update_session_with_credential` path bails out at
`repo.save(...).map_err(repository_error)?` — never reaches `connect_attempt`. Proves the
claim. **Passes.**

#### 3. `permanent_trust_failure_keeps_handshake_pending_for_session_scoped_fallback`
`src-tauri/crates/rshell-core/src/command_dispatcher.rs:993-1039`
```
let failure = dispatcher.dispatch(AppCommand::DecideHostKey {...,accept:true,permanent:true})
    .await.unwrap_err();
assert!(matches!(failure, CoreError::HostKeyTrustPersistenceFailed(_)));
assert_eq!(registry.state(decision_id), Some(HostKeyDecisionState::Pending));
// then trust-once:
dispatcher.dispatch(AppCommand::DecideHostKey {...,accept:true,permanent:false}).await.unwrap();
assert!(receiver.await.unwrap().accept);
assert!(!known_hosts.exists());
```
The dispatcher blocks the tmp write by creating a directory at `known_hosts.tmp`; the
`trust_host_key` call fails, `release_claim` returns the state to Pending, and the
oneshot is never sent — even though the failure path runs the `?` operator and the
authority of the claim was already taken. A follow-up trust-once succeeds and the
oneshot receives `accept=true`. `known_hosts` does not exist because the trust-once path
never wrote to disk. Proves the claim. **Passes.**

#### 4. `concurrent_decisions_are_keyed_and_one_resolution_cannot_clear_the_other`
`src-tauri/crates/rshell-core/src/security/host_key_decision.rs:301-364`
```
assert!(reg.resolve(first_id, ...{accept:true,permanent:false}));
assert!(reg.request_info(second_id).is_some());
assert!(reg.resolve(second_id, ...{accept:false,permanent:false}));
assert!(first_rx.await.unwrap().accept);
assert!(!second_rx.await.unwrap().accept);
```
Two distinct UUIDs; resolving first does not remove the second's request info; resolving
second delivers only to second_rx. Proves the claim. **Passes.**

#### 5. `cancellation_and_expiry_close_only_their_decision_and_drop_late_events`
`src-tauri/crates/rshell-core/src/security/host_key_decision.rs:366-399`
```
reg.cancel(cancelled_id);
assert!(cancelled_rx.await.is_err());
assert!(!reg.resolve(cancelled_id, ...{accept:true,...}));
assert!(reg.expire(expired_id));
assert!(expired_rx.await.is_err());
assert!(!reg.expire(expired_id));
let (next_id, next_rx) = reg.register();
reg.publish_request(...);
assert!(reg.cancel(next_id));
assert!(next_rx.await.is_err());
```
Cancel drops sender (Err on recv), resolve on a cancelled id is a no-op (returns false),
expire works once and is idempotent on a second call, and a freshly-registered id after
expiry still cancels cleanly. Proves the claim. **Passes.**

All five spot-checked tests actually prove the behavior claimed — none of them are mere
compile-time existence assertions.

### Independent review summary

The implementation is correct against every Task 3 spec clause and every Task 3 plan bullet
I could exercise without real platform credentials. The risk concerns above are about (a)
real-platform coverage the implementer already documented, and (b) one minor UX bug the
implementer did not flag. None of them block Task 3's "development verification complete"
milestone; (a) belongs in Task 4 / product acceptance, and (b) is small enough to ride
along with the next fix in the same area.