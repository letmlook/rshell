# Transfer Pause and Resume Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose the existing transfer pause/resume backend operations through honest, failure-aware UI controls.

**Architecture:** `TransferPanel` renders state-derived actions and emits task IDs; `App.vue` owns IPC calls, pending state, refresh, and user-visible errors. Backend task events remain the source of truth.

**Tech Stack:** Vue 3, TypeScript, Element Plus, Tauri IPC, Vitest/Test Utils.

**Spec:** `docs/superpowers/specs/2026-09-27-release-readiness-hardening-design.md`

## Global Constraints

- Show pause only for `active` and resume only for `paused`.
- Never optimistically mutate a task phase; refresh from backend after IPC.
- Disable an action while that task ID is pending and surface IPC failure.

## Review Focus

- Repeated clicks while an IPC call is pending produce one command.
- A failed pause/resume leaves the displayed backend state unchanged.
- Terminal states never render pause/resume actions.
- Refresh failure after a successful command remains visible.
- Multiple tasks track pending state independently.

---

### Task 1: Add state-derived transfer actions

**Files:**
- Modify: `src/components/TransferPanel.vue`
- Test: `tests/unit/TransferPanel.spec.ts`

**Interfaces:**
- Produces: `pendingTaskIds: ReadonlySet<string>` prop and `pause(taskId: string)` / `resume(taskId: string)` emits.

- [ ] **Step 1: Add failing component tests** for active/paused/terminal visibility, exact emitted IDs, and per-task disabled state.
- [ ] **Step 2: Run `npm test -- tests/unit/TransferPanel.spec.ts`** and verify failures are caused by missing action controls.
- [ ] **Step 3: Implement the action column and emits** with accessible labels “暂停传输” and “继续传输”; stop row/header click propagation.
- [ ] **Step 4: Run the focused test** and verify it passes.
- [ ] **Step 5: Commit** with `git commit -m "feat: add transfer pause and resume controls"`.

### Task 2: Wire IPC, pending state, refresh, and errors

**Files:**
- Modify: `src/App.vue`
- Modify: `tests/unit/AppLayout.spec.ts`

**Interfaces:**
- Consumes: panel emits and existing `pauseTransfer(taskId)` / `resumeTransfer(taskId)` clients.
- Produces: `runTransferAction(taskId, action)` with per-task pending state, backend refresh, and `ElNotification.error` on failure.

- [ ] **Step 1: Add failing App tests** asserting one IPC call during repeated clicks, post-success refresh, pause/resume failure notification, unchanged task phase, and independent pending IDs.
- [ ] **Step 2: Run `npm test -- tests/unit/AppLayout.spec.ts`** and verify the handlers are missing.
- [ ] **Step 3: Implement the shared action handler** using `Set<Uuid>`, `try/catch/finally`, and `refreshTransfers()`; bind it to panel events.
- [ ] **Step 4: Run `npm test -- tests/unit/AppLayout.spec.ts tests/unit/TransferPanel.spec.ts`** and verify both pass.
- [ ] **Step 5: Run `npm run typecheck && npm test`** and verify the full frontend suite passes.
- [ ] **Step 6: Commit** with `git commit -m "feat: wire transfer pause and resume actions"`.
