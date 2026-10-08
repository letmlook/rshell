# RShell Reliability Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans task-by-task. Observe failing regression tests before production edits.

**Goal:** Complete the accepted SSH, SFTP and credential recovery design within the existing Tauri architecture.

**Architecture:** Protocol code owns I/O, deadlines and file commit semantics; core owns lifecycle and outcomes; Vue renders real operation availability, errors and recovery. Existing IPC types evolve only where structured outcomes are required, with both Rust and TypeScript updated together.

**Tech Stack:** Tauri 2, Vue 3, TypeScript, Rust, Tokio, russh 0.62 and russh-sftp 2.4.

**Spec:** `docs/superpowers/specs/2026-10-08-reliability-improvement-design.md`.

**Execution status:** Paused by user on 2026-10-08. Task 1 implementation committed as `c1afefc` with scoped tests passing; independent task review interrupted without verdict. Tasks 2/3 not started, Task 4 pending. Resume from [saved progress](2026-10-08-reliability-recovery-progress.md), not by reimplementing Task 1.

## Global Constraints

- Retain Tauri; macOS is the acceptance platform. No egui or expanded platform scope.
- Never log passwords, key passphrases or terminal input contents.
- Input timeout means outcome uncertain; no automatic input replay. Immediately restrict affected terminal input and expose manual reconnect.
- Staged SFTP writes preserve the old target until safe commit. No delete-old-then-rename fallback; unsupported safe commit fails visibly.
- Retrying a failed transfer starts a new task from zero and rechecks conflict policy. Removing records never removes files.
- Human host-key decisions remain explicit; permanent trust must persist before accepting the handshake.
- Follow CLAUDE.md and CONTRIBUTING.md. Commits use letmlook <letmlook@aliyun.com>.
- No real credentials, user files or external service mutations in tests. No push, merge or release.
- Development verification and macOS product acceptance are separate; report unavailable target-platform acceptance honestly.

## Review Focus

- Expired queued input executes after timeout or enters a reconnected terminal.
- Closing a congested terminal blocks shared-client writers or affects an unrelated connection.
- Target appears between precheck and final commit, or an unowned file occupies a temporary path.
- Cancellation wins while final commit still overwrites a target; cleanup errors mask original failure.
- Concurrent host-key decision submission clears another request, or stale events restore an expired decision.

## Task 1: SSH deadlines and terminal recovery

**Files:** protocol `ssh/client.rs` (optionally focused request module), `rshell-protocol/src/lib.rs`, core `session/service.rs`, frontend `TerminalPane.vue`, associated IPC/error mapping and tests where required.

**Interfaces:** Consume existing send/resize/close and session connect/disconnect commands. Produce bounded request lifecycle and a distinguishable terminal recovery-required outcome consumed by Vue. Do not treat plain server output silence as failure.

- [ ] Write regressions reproducing saturated request queue, delayed request after caller timeout, blocked actor close and unaffected independent connections. Use short injectable test budgets, not 10-second sleeps.
- [ ] Run protocol regressions against unchanged production code; record failures and their causes.
- [ ] Bound queue admission and execution; skip expired unstarted requests, end or isolate stalled terminal actor, and make close/teardown cancellation independent of queued writes. Keep remote-output drain operational and do not silently change multi-label lifecycle.
- [ ] Add Vue regressions: one input timeout displays persistent uncertain-result notice, suppresses further input/paste, manual reconnect restores usable terminal only after backend success, failed reconnect retains recovery state; no replay.
- [ ] Run frontend regressions before Vue production changes; then implement the recovery state and real disconnect/connect recovery path. Preserve existing attach/unmount cleanup.
- [ ] Verify `cargo test -p rshell-protocol --lib`, `cargo test -p rshell-core --lib`, `npm test`, `npm run typecheck`. Record focused RED and final GREEN evidence; commit task changes only.

## Task 2: Staged SFTP commit and transfer retry

**Files:** protocol `ssh/sftp.rs` and focused staging module if needed, core `transfer/service.rs`, shared transfer outcome/types if needed, `TransferWorkspace.vue`, `TransferPanel.vue`, `transferItem.ts`, associated tests.

**Interfaces:** Preserve pause/resume semantics within an active task; expose actual commit/cleanup outcome and temporary residual path through consistent Rust/TypeScript contract. Add explicit failed/cancelled-task retry using existing enqueue path and conflict confirmation, or a small command if existing contract cannot carry it safely.

- [ ] Write failing upload/download regressions exercising production transfer staging: old/new target preservation on failure/cancel, late conflict, exclusive temp creation, safe commit, cleanup failure and cancel/commit arbitration. Tests must exercise production state machine, not an isolated mock of the whole transfer.
- [ ] Run regressions before changes; record observed failure.
- [ ] Stage local/remote files exclusively next to target. Check actual locked dependency rename/replacement semantics from local sources; implement supported safe commit. Fail explicitly where the server cannot safely commit. Preserve original failure plus cleanup result. Mark complete only after commit and close/flush succeed.
- [ ] Ensure cancellation and commit are mutually adjudicated in the task lifecycle, including task removal/pruning while cleanup is pending; no completed file paired with cancelled result.
- [ ] Write/run failing UI regressions for residual information, from-zero retry and conflict reconfirmation; implement through real queue commands and existing confirmation flow. No cross-connection resumption.
- [ ] Verify protocol/core tests, `npm test`, `npm run typecheck`; document supported commit capability and residual handling without claiming universal atomicity. Commit task changes only.

## Task 3: Credential and host-key recovery consistency

**Files:** infra `credentials.rs`, core session repository/service/error as needed; API/events/Tauri event bridge where required; frontend `SessionList.vue`, `SessionCredentialDialog.vue`, sessions/hostKey stores and `HostKeyMismatchDialog.vue`; associated tests.

**Interfaces:** Distinguish missing stored credentials from inaccessible storage and auth/network failures without exposing secrets. Carry host-key request cancellation/expiry keyed by decision id via existing event architecture.

- [ ] Write/run failing regressions for missing/inaccessible credentials and correct UI recovery actions, failed credential save, rejected auth after save and cancelled editing.
- [ ] Implement error categories and direct recovery actions using existing credential editor, retry-load and explicit connection actions. Do not introduce automatic reconnect or secret display.
- [ ] Write/run failing regressions for concurrent host-key requests, expired/cancelled requests, submitted decision racing another request, and failed permanent trust persistence.
- [ ] Preserve requests independently, sync backend cancellation/expiry, permit dismissal of invalid decisions without letting them accept a handshake; valid decisions retain explicit trust/reject behavior. Avoid secret-bearing diagnostic logs.
- [ ] Verify impacted Rust tests, IPC contract tests, full `npm test` and typecheck; commit task changes only.

## Task 4: Integration verification and evidence

**Files:** affected current documentation, `docs/09-macos-validation.md`, spec status and this plan.

- [ ] Review cross-task type compatibility and every accepted spec requirement; fix material omissions with failing regressions first.
- [ ] Run shared verification/audit scripts when usable on Windows. If shell environment prevents execution, run the contained checks in their documented order and report environment-specific script failures instead of claiming the shared entrypoint passed.
- [ ] Run fresh whole-branch review with a separate reviewer. Resolve significant issues and verify fixes.
- [ ] Document actual test counts/results, safe commit capabilities and remaining macOS real-device/server/keychain acceptance. No simulated macOS acceptance.
- [ ] Leave a reviewable isolated branch with committed code and documentation; do not merge/push without authorization.

## Execution choices

The user confirmed the complete design and requested continuation. Proceed with sequential implementers and task reviews without another plan-approval pause. Work in the managed `reliability-recovery` worktree. Windows-compatible ledger/brief/review-package creation substitutes for the skill's POSIX helper scripts. The local existing Cargo target directory may be reused serially to avoid cold recompilation; never run concurrent Rust builds against it.
