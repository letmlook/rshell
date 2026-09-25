# macOS Existing-Feature Hardening Implementation Plan

> Historical execution plan (2026-09-25). The original steps below are preserved for traceability, not as the current support matrix. For implementation status and observed verification as of 2026-09-26, see [project status](../../02-project-plan.md) and [macOS validation](../../09-macos-validation.md).

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Deliver a macOS-verifiable RShell whose visible features are real, whose unsupported RDP and Remote Forward capabilities are absent, and whose documentation describes the Tauri/Vue product.

**Architecture:** Vue calls typed Tauri IPC only; Tauri adapts calls only; core and protocol crates own behavior. Remove unsupported public contracts before completing every visible control’s end-to-end path. Automated results plus a dated macOS manual checklist determine documentation claims.

**Tech Stack:** Rust 1.90, Tokio, Tauri 2, Vue 3, TypeScript, Vitest, xterm.js, russh/russh-sftp, serialport, Rhai and wasmtime.

**Spec:** docs/superpowers/specs/2026-09-25-macos-hardening-design.md

## Global Constraints

- Keep existing SSH/SFTP, Telnet, Serial, terminal, security, automation, Local/Dynamic tunnel, theme and WASM-plugin capability only.
- Remove RDP from runtime code, public types, direct dependencies, tests and product claims.
- Do not modify Windows ConPTY or claim Windows runtime verification; macOS is the sole GUI acceptance platform.
- A visible action invokes typed IPC or is removed. No silent no-op and no mock-on-error fallback.
- Do not advertise Remote Forward; legacy persisted Remote rules are preserved but skipped with a clear error.
- Run Rust commands from src-tauri and commit with letmlook <letmlook@aliyun.com>.

## Review Focus

1. Persisted RDP sessions fail validation instead of silently becoming SSH (Task 2).
2. Persisted Remote Forward rules cannot bind a plain TCP listener or be erased (Task 3).
3. Empty/root/directory/unselected transfer targets cannot enqueue or delete files (Task 4).
4. Trigger actions after disconnect report failure rather than retaining a stale sender (Task 5).
5. Reopened Vue components do not duplicate Tauri listeners (Task 5).

## File Structure

- Rust contracts: src-tauri/crates/rshell-api/src/{types.rs,commands.rs,outcome.rs}.
- Protocol support: src-tauri/crates/rshell-protocol/{src/lib.rs,Cargo.toml}.
- Core behavior: src-tauri/crates/rshell-core/src/{command_dispatcher.rs,session/service.rs,security/tunnel_manager.rs,script/engine.rs,theme/mod.rs}.
- Tauri/client/UI: src-tauri/src/commands.rs, src/ipc/, src/components/transfer/, src/App.vue, src/stores/theme.ts.
- Evidence: README.md, CLAUDE.md, CONTRIBUTING.md, CHANGELOG.md, docs/.

### Task 1: Create an isolated, measured baseline

**Files:**
- Modify: none unless locked installation requires its lock file.
- Test: existing frontend and Rust suites.

**Interfaces:**
- Consumes: clean main, package-lock.json and src-tauri/Cargo.lock.
- Produces: branch codex/macos-hardening and recorded baseline outcomes.

- [ ] **Step 1: Detect current isolation**

~~~
git_dir=$(cd "$(git rev-parse --git-dir)" && pwd -P)
git_common=$(cd "$(git rev-parse --git-common-dir)" && pwd -P)
git branch --show-current
git rev-parse --show-superproject-working-tree
~~~

Expected: normal checkout on main and not a submodule.

- [ ] **Step 2: Create the approved worktree**

~~~
git check-ignore -q .worktrees
git worktree add .worktrees/macos-hardening -b codex/macos-hardening
~~~

Expected: ignored worktree on codex/macos-hardening.

- [ ] **Step 3: Establish frontend baseline**

~~~
npm ci
npm run typecheck
npm test
~~~

Expected: record exact counts. Stop to diagnose a baseline failure before product changes.

- [ ] **Step 4: Establish Rust baseline**

~~~
cd src-tauri
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
~~~

Expected: record exact counts. Do not attribute a baseline failure to a later task.

### Task 2: Remove RDP from the public and runtime contract

**Files:**
- Delete: src-tauri/crates/rshell-protocol/src/rdp/mod.rs.
- Modify: src-tauri/crates/rshell-protocol/src/lib.rs, src-tauri/crates/rshell-protocol/Cargo.toml, src-tauri/Cargo.lock.
- Modify: src-tauri/crates/rshell-api/src/types.rs and src/ipc/types.ts.
- Test: an rshell-api test module for protocol serialization.

**Interfaces:**
- Consumes: serialized Protocol and ProtocolType.
- Produces: SSH, Telnet and Serial only; RDP deserialization is rejected.

- [ ] **Step 1: Write failing protocol rejection tests**

Add tests equivalent to:

~~~rust
assert!(serde_json::from_str::<Protocol>("\"RDP\"").is_err());
assert!(serde_json::from_str::<ProtocolType>("\"RDP\"").is_err());
assert!(serde_json::from_str::<Protocol>("\"SSH\"").is_ok());
~~~

- [ ] **Step 2: Confirm current behavior fails the new requirement**

~~~
cd src-tauri
cargo test -p rshell-api rdp -- --nocapture
~~~

Expected: FAIL because both enums still expose RDP.

- [ ] **Step 3: Delete RDP runtime code and direct dependencies**

Delete rdp/mod.rs, its lib export, RdpConfig and Rust/TypeScript union members. Remove IronRDP, Rustls and graphics dependencies used solely by RDP and update Cargo.lock.

- [ ] **Step 4: Verify removal**

~~~
cd src-tauri
cargo test -p rshell-api rdp -- --nocapture
cargo check --workspace
cd ..
rg -n -i '\brdp\b' src-tauri/src src-tauri/crates src --glob '!target/**'
~~~

Expected: test/check PASS, with no runtime references.

- [ ] **Step 5: Commit**

~~~
git add src-tauri/crates/rshell-protocol src-tauri/crates/rshell-api src/ipc/types.ts src-tauri/Cargo.lock
git commit -m "refactor: remove unsupported RDP protocol"
~~~

### Task 3: Restrict tunnels to Local and Dynamic, preserving legacy files

**Files:**
- Modify: src-tauri/crates/rshell-api/src/types.rs, src-tauri/crates/rshell-core/src/security/tunnel_manager.rs, src-tauri/crates/rshell-core/src/command_dispatcher.rs.
- Modify: src-tauri/src/commands.rs, src/ipc/{types.ts,client.ts}, src/components/TunnelPanel.vue.
- Test: tunnel-manager Rust tests and TunnelPanel Vitest.

**Interfaces:**
- Consumes: persisted TOML PortForwardRule and IPC createTunnel(session_id, rule).
- Produces: public TunnelType Local/Dynamic, plus UnsupportedTunnelRule { rule_id, reason } for legacy Remote entries.

- [ ] **Step 1: Write failing migration tests**

Load TOML containing Local, Dynamic and legacy Remote rules. Assert Local/Dynamic are unchanged, Remote is not started, and source TOML bytes are unchanged. Assert new Remote input produces CoreError::UnsupportedFeature("remote forwarding").

- [ ] **Step 2: Run focused tests**

~~~
cd src-tauri
cargo test -p rshell-core tunnel_manager -- --nocapture
~~~

Expected: FAIL until compatibility loading is present.

- [ ] **Step 3: Implement storage-only compatibility conversion**

Use a private legacy DTO which decodes Remote, then returns a supported rule or UnsupportedTunnelRule. Never pass unsupported entries to create_tunnel. New Rust/TS types and TunnelPanel offer Local/Dynamic only.

- [ ] **Step 4: Verify and commit**

~~~
cd src-tauri && cargo test -p rshell-core tunnel_manager -- --nocapture
cd .. && npm run typecheck && npm test -- TunnelPanel
git add src-tauri/crates/rshell-api src-tauri/crates/rshell-core src-tauri/src/commands.rs src/ipc src/components/TunnelPanel.vue
git commit -m "fix: limit tunnels to supported forwarding modes"
~~~

Expected: PASS and no Remote UI option.

### Task 4: Replace typed-but-incomplete SFTP paths and mock UI

**Files:**
- Modify: src-tauri/crates/rshell-api/src/{commands.rs,outcome.rs,types.rs}, src-tauri/crates/rshell-core/src/{command_dispatcher.rs,session/service.rs,transfer/service.rs}, src-tauri/src/commands.rs.
- Modify: src/ipc/{types.ts,client.ts}, src/App.vue, src/components/WorkspaceToolbar.vue, src/components/transfer/{TransferWorkspace.vue,FileBrowserPane.vue}, src/components/{SidePanel.vue,FileBrowserPanel.vue}.
- Test: command-dispatcher/transfer Rust tests and TransferWorkspace/FileBrowserPane/App/SidePanel Vitest tests.

**Interfaces:**
- Consumes: BrowseRemoteDir, EnqueueUpload and EnqueueDownload.
- Produces: RemoteDir { path, entries }, CreateRemoteDirectory and DeleteRemoteEntry commands; an active TransferWorkspace action context.

- [ ] **Step 1: Write failing core and Tauri boundary tests**

Assert browse_remote_dir returns CommandOutcome::RemoteDir as { path, entries }, not (). Assert empty, slash-root, traversal and directory-delete paths return errors before protocol I/O; a regular file request reaches mocked SFTP.

- [ ] **Step 2: Run focused Rust tests**

~~~
cd src-tauri
cargo test -p rshell-core 'browse_remote_dir|enqueue_|delete_' -- --nocapture
cargo test -p rshell browse_remote_dir -- --nocapture
~~~

Expected: FAIL because commands.rs currently discards RemoteDir and guards are incomplete.

- [ ] **Step 3: Implement narrow, guarded commands**

Unwrap RemoteDir in the Tauri command. Add remote directory creation and file deletion through API/core/Tauri, normalize remote paths and reject empty/root/traversal values. Require a non-directory selected file for deletion. Do not use shell commands for local operations.

- [ ] **Step 4: Write failing UI tests**

Mount TransferWorkspace with no session, no selected row and root remote path; assert upload/download/create/delete disabled. With a selected file, assert enqueueUpload(localFile, remoteFile, sessionId) occurs once. Assert delete opens confirmation and cannot call API before confirmation.

- [ ] **Step 5: Remove mock and no-op behavior**

Delete localMock, remoteMock, Windows sample paths, remote error fallback and App empty callbacks. Use the fs plugin only after a chosen local root and typed remote output for remote lists. Delegate WorkspaceToolbar actions to the active workspace, refresh after mutation, show operation errors. SidePanel either reuses a read-only browser for a selected SSH session or shows the explicit connected-session empty state.

- [ ] **Step 6: Verify and commit**

~~~
cd src-tauri && cargo test -p rshell-core 'browse_remote_dir|enqueue_|delete_' -- --nocapture
cd .. && npm test -- TransferWorkspace FileBrowserPane App SidePanel
npm run typecheck
npm run build
git add src-tauri/crates/rshell-api src-tauri/crates/rshell-core src-tauri/src/commands.rs src/ipc src/App.vue src/components
git commit -m "fix: wire transfer workspace to real file operations"
~~~

Expected: all tests, typecheck and build PASS.

### Task 5: Complete automation, theme event lifecycle and quality evidence

**Files:**
- Modify: src-tauri/crates/rshell-core/src/{session/service.rs,command_dispatcher.rs,script/engine.rs,script/trigger_engine.rs,theme/mod.rs}.
- Modify: src-tauri/crates/rshell-api/src/{events.rs,outcome.rs,types.rs}, src-tauri/src/commands.rs, src/ipc/{client.ts,events.ts}, src/stores/theme.ts, src/App.vue, src/components/ThemePanel.vue.
- Test: core trigger/script/theme tests and frontend theme/App tests.

**Interfaces:**
- Consumes: terminal output, TriggerAction, SessionService send/disconnect, ListThemes and ThemeChanged.
- Produces: TriggerActionSink::dispatch(session_id, action), ScriptHost service trait, ThemeInfo containing colors and retained Tauri unlisten callbacks.

- [ ] **Step 1: Write failing trigger and script tests**

With fake writer/event bus, assert SendText("clear\n") writes exact bytes, notification emits TriggerFired, disconnect executes once and stale writer emits typed failure. Assert invalid UTF-8 does not match. Assert ScriptHost send/list_sessions/execute_quick_command calls core services and converts errors to Rhai runtime errors.

- [ ] **Step 2: Write failing theme/listener tests**

Assert ListThemes includes complete active colors; changing theme emits those colors and the store maps all CSS variables. Mock Tauri listen, mount/unmount App twice and assert every returned unlisten function runs exactly once.

- [ ] **Step 3: Run focused tests**

~~~
cd src-tauri && cargo test -p rshell-core 'trigger|script|theme' -- --nocapture
cd .. && npm test -- theme App ThemePanel
~~~

Expected: FAIL for unwired SendText, missing colors and unreleased listeners.

- [ ] **Step 4: Implement bounded core behavior**

Give the receive task a cloneable synchronized sender registry or TriggerActionSink, never a borrowed self. Route every TriggerAction to explicit send/notify/disconnect/log/script/quick-command behavior and emit typed failure events. Inject ScriptHost into ScriptEngine without Tauri/Vue references. Return ThemeInfo colors; centralize CSS updates in theme.ts and release all listeners in component unmount/store disposal.

- [ ] **Step 5: Verify and commit**

~~~
cd src-tauri
cargo test -p rshell-core 'trigger|script|session|theme' -- --nocapture
cargo clippy -p rshell-core --all-targets -- -D warnings
cd ..
npm test -- theme App ThemePanel
npm run typecheck
git add src-tauri/crates/rshell-api src-tauri/crates/rshell-core src-tauri/src/commands.rs src/ipc src/stores src/App.vue src/components/ThemePanel.vue
git commit -m "fix: complete automation and theme event behavior"
~~~

Expected: PASS.

### Task 6: Verify macOS and rewrite all product documentation

**Files:**
- Modify: README.md, CLAUDE.md, CONTRIBUTING.md, CHANGELOG.md and docs/01-xshell-xftp-feature-research.md through docs/08-incomplete-features.md.
- Create: docs/09-macos-validation.md.
- Test: all automated gates and manual Tauri checklist.

**Interfaces:**
- Consumes: proved Task 2–5 support matrix.
- Produces: accurate Tauri/Vue documentation and dated macOS evidence.

- [ ] **Step 1: Add a failing documentation contract check**

The check fails if README or setup docs claim GPUI, list RDP as supported, or prescribe cargo run --package rshell-ui. Accepted launch commands are npm run tauri:dev and npm run tauri:build.

- [ ] **Step 2: Run the check before rewrites**

~~~
rg -n 'GPUI|RDP|rshell-ui' README.md CLAUDE.md CONTRIBUTING.md docs
~~~

Expected: documented stale claims are found.

- [ ] **Step 3: Rewrite from verified behavior**

Document Tauri 2 + Vue 3 + xterm.js; SSH/SFTP, Telnet and Serial only; Local/Dynamic tunnels only; and actual macOS boundaries. Replace historical incomplete status with a support/verification matrix and non-goals. Research history may mention RDP solely as removed scope.

- [ ] **Step 4: Execute and record manual macOS verification**

Create a dated checklist for Tauri launch, SSH, terminal input/resize/copy, host key, SFTP browse/upload/download/create/delete, Local/Dynamic tunnel, theme, trigger, script and plugin loading. Run npm run tauri:dev; mark only observed checks complete and record exact blockers.

- [ ] **Step 5: Run final gates**

~~~
npm run typecheck
npm test
npm run build
cd src-tauri && cargo fmt --check
cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings
cd src-tauri && cargo test --workspace
cd .. && rg -n -i '\brdp\b|\bgpui\b' README.md CLAUDE.md CONTRIBUTING.md docs src src-tauri --glob '!src-tauri/target/**'
~~~

Expected: all quality gates PASS. Any history-only match is explicitly marked removed scope and is not a product claim or runtime reference.

- [ ] **Step 6: Commit documentation and evidence**

~~~
git add README.md CLAUDE.md CONTRIBUTING.md CHANGELOG.md docs package.json src-tauri
git commit -m "docs: align product documentation with macOS support"
~~~

### Task 7: Independent review and final handoff

**Files:**
- Modify: only files required to fix reproduced review findings.
- Test: final suite in Task 6.

**Interfaces:**
- Consumes: complete branch, spec, plan and macOS evidence.
- Produces: clean reviewable branch and evidence-backed completion report.

- [ ] **Step 1: Audit for scope regressions**

~~~
git diff main...HEAD --check
git diff --stat main...HEAD
rg -n '=> \{\}|not yet wired|placeholder|mock|TO''DO\(' src src-tauri --glob '!target/**'
~~~

Expected: no silent visible action, RDP runtime reference or mock fallback; test fixtures are clearly test-only.

- [ ] **Step 2: Request independent code review**

Give the reviewer this plan, the design spec, branch diff and final outputs. Require file/line citations; fix only reproduced findings.

- [ ] **Step 3: Re-run final gates and report**

Repeat Task 6 Step 5 after any review fix, then run:

~~~
git status --short
git log --oneline main..HEAD
~~~

Expected: clean worktree, tested commands, observed macOS checks and any deliberately unverified hardware path are included in handoff.

## Plan Self-Review

- **Coverage:** Tasks 2–5 respectively remove RDP, constrain tunnels, complete SFTP/UI and complete automation/theme events. Task 6 updates every named document and records macOS evidence. Task 7 audits and reviews the finished branch.
- **Type consistency:** RemoteDir, UnsupportedTunnelRule, TriggerActionSink, ScriptHost and extended ThemeInfo are defined before their consumers.
- **Review focus:** Every listed failure mode is pinned by a concrete Task 2–5 test.
- **Scope:** RDP and Remote Forward are removed rather than replaced; Windows and new protocols stay excluded.
