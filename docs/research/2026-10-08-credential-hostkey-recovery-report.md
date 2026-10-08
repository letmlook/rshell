# Task 3 research report — credential and host-key recovery consistency

- Worktree: `C:/Users/lp/.codex/worktrees/reliability-recovery/rshell`
- Branch/base: `codex/reliability-recovery`, base `28ba8df`
- Task 3 commit: `3c309f6` (`fix: align credential and host-key recovery`)
- This report is deliberately **untracked** and is not part of the Task 3 commit.

## RED evidence captured before production implementation

The following runs were made after adding the new regressions but before the corresponding production code was complete:

1. `cargo test -p rshell-core --lib credential_ --manifest-path src-tauri/Cargo.toml`
   - Failed to compile because `CoreError::CredentialMissing`, `CredentialInaccessible`, and `CredentialSaveFailed`, plus `SessionLoadIssueKind` / typed issue fields, did not exist.
2. `npx vitest run tests/unit/SessionListSecurity.spec.ts tests/unit/ConnectionRecoveryNotice.spec.ts tests/unit/ipcContract.spec.ts`
   - `ipcContract.spec.ts` failed because the TypeScript error mirror did not contain `credential_missing` and the other new stable kinds.
   - The new recovery component test initially failed at import because `ConnectionRecoveryNotice.vue` did not exist.
3. `npx vitest run tests/unit/hostKeyStore.spec.ts`
   - The three new lifecycle cases failed because the store had no `requests` map and no keyed `cancel` operation.
4. `cargo test -p rshell-core --lib host_key_decision --manifest-path src-tauri/Cargo.toml`
   - Failed to compile because `HostKeyDecisionStateChanged`, `HostKeyDecisionState`, and `expire` did not exist.
5. `cargo test -p rshell-protocol --lib host_key_timeout --manifest-path src-tauri/Cargo.toml`
   - Failed to compile because the sink had no `expire_decision` lifecycle method and the handler had no injectable host-key timeout.

These failures were caused by the intended missing contracts, not by external services or credentials.

## GREEN evidence

All commands used the required stable toolchain, `RUSTUP_NO_UPDATE_CHECK=1`, and the shared `C:/code/github/rshell/src-tauri/target` directory.

| Check | Result |
|---|---|
| `cargo fmt --all --manifest-path src-tauri/Cargo.toml --check` | exit 0 |
| `cargo test -p rshell-protocol --lib --manifest-path src-tauri/Cargo.toml` | **70 passed, 0 failed** |
| `cargo test -p rshell-core --lib --manifest-path src-tauri/Cargo.toml` | **211 passed, 0 failed** |
| `cargo test -p rshell --lib --manifest-path src-tauri/Cargo.toml` | **15 passed, 0 failed** |
| `cargo clippy -p rshell-protocol -p rshell-core -p rshell --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings` | exit 0 |
| `npm test` | **38 files, 311 passed, 0 failed** |
| `npm run typecheck` | exit 0 |
| `git diff --check` | exit 0 (only Git line-ending warnings) |
| New-log secret scan (`git diff` additions containing log macro plus password/passphrase/secret) | no matches |

Baseline counts recorded before Task 3 were protocol 69, core 189, Tauri 15, and frontend 279 passed (3 skipped across 37 files). The final frontend run reported no skipped tests; this is a test-runner result, not a claim that platform acceptance was performed.

## CoreError and IPC category mapping

The recovery-facing core categories are:

- `CredentialMissing` — metadata declares a credential but the secure entry is absent/empty.
- `CredentialInaccessible` — the secure store cannot be read because access is unavailable/refused.
- `CredentialSaveFailed` — a credential write/delete operation was refused; rollback is also kept in this category.
- `CredentialMigrationFailed` — legacy migration did not commit; the safe cause and retryable load issue are retained.
- `AuthError` / `AuthenticationFailed` — SSH or other authentication rejection; both map to the stable IPC `auth_failed` kind.
- `ConnectionError` — transport/network/protocol failure that is not auth or host-key-specific.
- `HostKeyMismatch` — SSH host-key verification ended without an accepted decision.
- `HostKeyTrustPersistenceFailed` — permanent trust could not be persisted; the handshake remains unaccepted.

The Tauri mapping is:

| Core category | `IpcErrorKind` / wire kind |
|---|---|
| `CredentialMissing` | `CredentialMissing` / `credential_missing` |
| `CredentialInaccessible` | `CredentialInaccessible` / `credential_inaccessible` |
| `CredentialSaveFailed` | `CredentialSaveFailed` / `credential_save_failed` |
| `CredentialMigrationFailed` | `CredentialMigrationFailed` / `credential_migration_failed` |
| `AuthError`, `AuthenticationFailed` | `AuthFailed` / `auth_failed` |
| `ConnectionError` | `Connection` / `connection` |
| `HostKeyMismatch` | `HostKeyMismatch` / `host_key_mismatch` |
| `HostKeyTrustPersistenceFailed` | `HostKeyTrustPersistenceFailed` / `host_key_trust_persistence_failed` |

The repository discards raw adapter/TOML diagnostics at the credential boundary. Session load issues now carry `kind`, a safe message, and `retryable`; connect retries preserve the migration category instead of turning it into a generic not-found error.

## Host-key request lifecycle

The registry has a UUID-keyed sender/request map and a state CAS boundary:

```text
Pending
  ├─ Trust once  ───────────────> DecidedTrustOnce
  ├─ Reject ────────────────────> DecidedReject
  ├─ Trust always claim
  │    ├─ trusted-store success > DecidedTrustAlways (then wake one handshake)
  │    └─ trusted-store failure > Pending (no wake; fallback remains available)
  ├─ explicit cancel ───────────> Cancelled
  ├─ bounded protocol timeout ─> Expired
  └─ stale/invalid local close ─> Dismissed
```

`DecidedTrustAlways` is a CAS claim while persistence is in flight. Only a successful `settle_claimed` sends an accepting oneshot. A failed write releases the claim to `Pending`; it does not send a fabricated rejection, allowing the user to choose trust-once or cancel. Cancellation/expiry removes the sender so the protocol wait future is actually dropped, and publishes a keyed `HostKeyDecisionStateChanged` event. A late decision or request event for an unknown/settled id is ignored. The frontend keeps all pending requests in a map and renders each independently; one decision cannot clear another.

The explicit frontend dismissal path calls `CancelHostKey(decision_id)`, never `DecideHostKey(accept=true)`. The permanent-trust failure path returns the underlying safe reason, keeps the request visible, and does not publish `Connected`.

## Credential recovery behavior

- The existing session-list **更新凭据** entry remains the missing-credential recovery action.
- Credential save failure surfaces the backend message, preserves the entered value, and does not close or fabricate success.
- Cancelling the editor clears only the in-memory input and performs no write.
- Saving credentials does not automatically reconnect; a later explicit connect can return `AuthFailed` and is never converted into a successful state.
- Migration failures preserve the original legacy bytes and expose an explicit retry-load path. No plaintext fallback is used.
- Frontend per-session recovery notices use distinct kind-specific guidance/actions; they do not parse error messages.

## Platform caveats

- **macOS (acceptance target):** the production adapter uses the native Keychain. Real Keychain prompts, denied-access recovery, old-file migration, and missing-entry recovery were not exercised in this Windows environment; those remain Task 4/product acceptance items.
- **Windows:** the `keyring` adapter maps to the platform credential service (normally Credential Manager/DPAPI semantics). This run used only in-memory credential doubles and a loopback SSH server; no user vault or DPAPI prompt was touched.
- **Linux:** the adapter depends on the available Secret Service/keyring backend. A headless Linux session may report an unavailable store; the UI directs the user to repair access rather than falling back to plaintext. No desktop Secret Service or real migration was available here.
- The host-key trusted store is an application known-hosts file. The failing-write test uses a blocked temporary path, not a user file.

## Unresolved concerns / limits

1. macOS Keychain, Windows DPAPI, and Linux Secret Service behavior still need real platform acceptance.
2. The protocol timeout is bounded at 60 seconds and tested with a short injected duration; no real user/network timing study was performed.
3. A persistence write that completes after an expiry/cancel race is prevented from waking/accepting the handshake by the sender/CAS check. The trusted-store write itself cannot be rolled back generically without risking deletion of a pre-existing key; this is recorded as a narrow lifecycle concern for platform review.
4. Frontend tests run under jsdom and existing Element Plus stubbing warnings remain; they do not replace a real WebView/macOS run.
5. No real credentials, user files, external SSH service, push, merge, release, or tag were used.

## Commit scope

Task 3 code and `docs/08-incomplete-features.md` are intended for one atomic commit with author `letmlook <letmlook@aliyun.com>`. This report and the pre-existing review reports remain untracked.
