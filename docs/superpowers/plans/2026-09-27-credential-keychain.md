# Credential Keychain Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove SSH secrets from session files, long-lived session state, and list IPC while migrating legacy plaintext sessions into the native system keychain.

**Architecture:** `rshell-api` owns safe public session types and the credential-store port, `rshell-infra` implements that port with `keyring` 4.2, and `rshell-core` coordinates metadata/credential transactions. The protocol layer receives a short-lived resolved credential only when a connection starts.

**Tech Stack:** Rust 1.90+, serde/TOML, keyring 4.2 `v1`, Tauri 2, Vue 3/TypeScript, Vitest.

**Spec:** `docs/superpowers/specs/2026-09-27-release-readiness-hardening-design.md`

## Global Constraints

- Keychain service is exactly `com.letmlook.rshell.credentials`; accounts are `session:<uuid>:password` and `session:<uuid>:passphrase`.
- Session TOML, `ListSessions`, logs, and errors must never contain password or passphrase values.
- Missing/unavailable keychain entries fail closed; no plaintext fallback.
- Legacy migration writes the keychain first and atomically replaces the TOML only after success.
- Keep the existing master-password feature as an application lock; do not present it as the credential vault.

## Review Focus

- Empty SSH passwords create no keychain entry but still produce a valid password-auth session.
- A keychain entry missing after restart produces a visible connection error and no `Connected` event.
- Updating metadata without supplying a new secret preserves the existing keychain secret.
- A failed legacy migration preserves the original bytes and does not add the session to runtime state.
- Deleting a session whose keychain entry is already absent remains idempotent.

---

### Task 1: Make public session types structurally secret-free

**Files:**
- Modify: `src-tauri/crates/rshell-api/src/types.rs`
- Modify: `src-tauri/crates/rshell-api/src/commands.rs`
- Modify: `src-tauri/crates/rshell-api/src/outcome.rs`
- Modify: `src/ipc/types.ts`
- Modify: `src/ipc/client.ts`
- Modify: `src/components/SessionCreateDialog.vue`
- Test: `src-tauri/crates/rshell-core/tests/private_key_security.rs`
- Create: `tests/unit/SessionCreateDialog.spec.ts`

**Interfaces:**
- Produces: secret-free `AuthMethod`; `SessionCredential { secret: String }`; `CredentialUpdate::{Keep, Set(SessionCredential), Clear}`; `CreateSession { config, credential: Option<SessionCredential> }`; `UpdateSession { id, config, credential: CredentialUpdate }`.
- Produces: TypeScript mirrors and `createSession(config, credential)` / `updateSession(id, config, credentialUpdate)`.

- [ ] **Step 1: Write failing serialization and dialog tests** asserting `SessionConfig` JSON has no `password`, `passphrase`, or sample secret, while create submits the secret only in the command credential field.
- [ ] **Step 2: Run `cargo test -p rshell-core --test private_key_security && npm test -- tests/unit/SessionCreateDialog.spec.ts`** and verify failures show the old secret-bearing schema.
- [ ] **Step 3: Replace `AuthMethod` secret fields with descriptors** (`Password { username }`, `PublicKey { username, key_path, has_passphrase }`, `KeyboardInteractive { username, has_password }`), add the credential input/update types, and update Rust/TypeScript commands and the create dialog.
- [ ] **Step 4: Run the two focused test commands** and verify they pass.
- [ ] **Step 5: Commit** with `git commit -m "refactor: separate session credentials from public config"`.

### Task 2: Resolve credentials only at the protocol boundary

**Files:**
- Modify: `src-tauri/crates/rshell-protocol/src/ssh/client.rs`
- Modify: `src-tauri/crates/rshell-protocol/src/ssh/mod.rs`
- Modify: `src-tauri/crates/rshell-core/src/session/service.rs`
- Test: `src-tauri/crates/rshell-protocol/src/ssh/client.rs`

**Interfaces:**
- Consumes: secret-free `SessionConfig` from Task 1.
- Produces: `ResolvedAuthMethod` in `rshell-protocol::ssh` and `SshClient::new(config: SessionConfig, auth: ResolvedAuthMethod)`.

- [ ] **Step 1: Add failing protocol tests** proving password/public-key authentication consumes `ResolvedAuthMethod` and that `SessionConfig` alone cannot initiate SSH authentication.
- [ ] **Step 2: Run `cargo test -p rshell-protocol ssh::client::tests`** and verify failure is caused by the missing resolved-auth API.
- [ ] **Step 3: Introduce `ResolvedAuthMethod` and update `SshClient`** so secrets live only in the connection attempt object; adjust existing loopback tests.
- [ ] **Step 4: Run `cargo test -p rshell-protocol`** and verify all protocol tests pass.
- [ ] **Step 5: Commit** with `git commit -m "refactor: isolate ssh credentials to connection attempts"`.

### Task 3: Add the credential-store port and native adapter

**Files:**
- Create: `src-tauri/crates/rshell-api/src/credentials.rs`
- Modify: `src-tauri/crates/rshell-api/src/lib.rs`
- Create: `src-tauri/crates/rshell-infra/src/credentials.rs`
- Modify: `src-tauri/crates/rshell-infra/src/lib.rs`
- Modify: `src-tauri/crates/rshell-infra/Cargo.toml`
- Modify: `src-tauri/Cargo.toml`
- Test: `src-tauri/crates/rshell-infra/src/credentials.rs`

**Interfaces:**
- Produces: object-safe `CredentialStore: Send + Sync` with `get(&CredentialKey) -> Result<Option<String>, CredentialError>`, `set(&CredentialKey, &str) -> Result<(), CredentialError>`, and `delete(&CredentialKey) -> Result<(), CredentialError>`; `CredentialKey { session_id, kind }`; `CredentialKind::{Password, Passphrase}`; typed `CredentialError`.
- Produces: `SystemCredentialStore::new(service: &'static str)` backed by `keyring::Entry`.

- [ ] **Step 1: Write failing contract tests** using an in-memory test store for set/get/delete, missing-entry idempotent delete, empty-secret handling, and injected backend failure without secret text in errors.
- [ ] **Step 2: Run `cargo test -p rshell-infra credentials`** and verify the module/API is missing.
- [ ] **Step 3: Add `keyring = 4.2.0` with the `v1` API** and implement the port plus system adapter using the exact service/account identifiers from Global Constraints.
- [ ] **Step 4: Run `cargo test -p rshell-infra credentials`** and verify tests pass without touching the user's keychain.
- [ ] **Step 5: Commit** with `git commit -m "feat: add native session credential store"`.

### Task 4: Make session metadata atomic and migrate legacy plaintext

**Files:**
- Modify: `src-tauri/crates/rshell-infra/src/storage/session_store.rs`
- Modify: `src-tauri/crates/rshell-core/src/session/repository.rs`
- Test: `src-tauri/crates/rshell-infra/src/storage/session_store.rs`
- Test: `src-tauri/crates/rshell-core/src/session/repository.rs`

**Interfaces:**
- Consumes: `Arc<dyn CredentialStore>`, `CredentialKey`, and safe `SessionConfig` from earlier tasks.
- Produces: `SessionRepository::new(path: PathBuf, credentials: Arc<dyn CredentialStore>)`; transactional `save(&SessionConfig, CredentialUpdate)`, `delete(Uuid)`, `load(Uuid)`, and `list_all()`; internal legacy parser never exposed through IPC.

- [ ] **Step 1: Add failing tests** for secret-free TOML, atomic replacement, successful password/passphrase migration, migration failure preserving exact original bytes, update rollback, metadata-only update preserving a secret, and idempotent delete with a missing entry.
- [ ] **Step 2: Run `cargo test -p rshell-infra session_store && cargo test -p rshell-core session::repository`** and verify the expected security/transaction failures.
- [ ] **Step 3: Implement atomic metadata writes and repository coordination**; parse legacy auth in a private compatibility DTO, migrate before returning a session, and redact all diagnostic values.
- [ ] **Step 4: Run the focused tests** and scan temp TOML fixtures to confirm sample secrets are absent after successful migration.
- [ ] **Step 5: Commit** with `git commit -m "feat: migrate session secrets to keychain"`.

### Task 5: Wire just-in-time credentials through session lifecycle and Tauri

**Files:**
- Modify: `src-tauri/crates/rshell-core/src/session/service.rs`
- Modify: `src-tauri/crates/rshell-core/src/command_dispatcher.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/commands.rs`
- Test: `src-tauri/crates/rshell-core/src/session/service.rs`
- Test: `src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: repository transaction API and `ResolvedAuthMethod`.
- Produces: create/update/delete persistence with credentials; connect-time credential lookup; secret-free `list_sessions()`.

- [ ] **Step 1: Add failing lifecycle tests** for create/list secrecy, connect-time lookup, missing credential failure without `Connected`, metadata-only update, rollback on save failure, and delete cleanup.
- [ ] **Step 2: Run `cargo test -p rshell-core session::service && cargo test -p rshell commands`** and verify failures reflect the unwired repository.
- [ ] **Step 3: Inject `SystemCredentialStore` during Tauri setup** and update dispatcher/service/commands to pass credential updates and resolve secrets only inside `connect_attempt`.
- [ ] **Step 4: Run `cargo test --workspace`** and verify every Rust test passes.
- [ ] **Step 5: Run `npm run typecheck && npm test`** and verify the frontend mirrors remain compatible.
- [ ] **Step 6: Commit** with `git commit -m "feat: secure persisted session credentials"`.

### Task 6: Document migration and verify the security boundary

**Files:**
- Modify: `docs/08-incomplete-features.md`
- Modify: `docs/09-macos-validation.md`
- Modify: `README.md`
- Modify: `scripts/check-docs.mjs`

**Interfaces:**
- Consumes: completed secret storage behavior.
- Produces: current user-facing migration/recovery instructions and documentation contract checks.

- [ ] **Step 1: Add failing documentation-contract assertions** that current docs identify Keychain as the credential store, explain missing-entry recovery, and do not claim the master password is the vault.
- [ ] **Step 2: Run `npm run check:docs`** and verify it fails on the old plaintext limitation text.
- [ ] **Step 3: Update current docs** with migration behavior, Keychain prompts, recovery, and the remaining external-validation boundary.
- [ ] **Step 4: Run `npm run check:docs && cargo test --workspace && npm test`** and verify all pass.
- [ ] **Step 5: Commit** with `git commit -m "docs: document keychain credential storage"`.
