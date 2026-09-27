# Release Automation and Build Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Centralize repeatable verification and release checks, fix macOS identity/build warnings, and run the same checks in CI.

**Architecture:** Small fail-fast shell entrypoints own verification, auditing, release preflight, and app validation; Node tests exercise them with fake executables. CI config only prepares the environment and invokes these scripts.

**Tech Stack:** Bash, Node test runner, npm/Vite, Cargo, Tauri 2, GitHub Actions, macOS codesign/spctl.

**Spec:** `docs/superpowers/specs/2026-09-27-release-readiness-hardening-design.md`

## Global Constraints

- Bundle ID is exactly `com.letmlook.rshell`.
- npm audit always uses `https://registry.npmjs.org`.
- CI pins checkout/setup-node/cache to `11d5960a326750d5838078e36cf38b85af677262`, `49933ea5288caeca8642d1e84afbd3f7d6820020`, and `0057852bfaa89a56745cba8c7296529d2fc39830`; cargo-audit is `0.22.2`.
- Scripts never print signing/notary credentials and fail nonzero when a required check is unavailable.
- CI and docs call scripts instead of duplicating their command bodies.
- Signing, notarization, and real-device validation remain unpassed without external evidence.

## Review Focus

- Every script works when invoked outside the repository root.
- A failing child command is returned unchanged to callers.
- Paths containing spaces are handled safely.
- Missing `cargo-audit`, app bundle, signing identity, or notary profile gives a specific non-secret error.
- Unsigned/ad-hoc apps fail strict release verification but can pass explicit no-secret preflight.

---

### Task 1: Centralize verification and dependency audits

**Files:**
- Create: `scripts/verify.sh`
- Create: `scripts/audit.sh`
- Create: `scripts/automation.test.mjs`
- Modify: `package.json`
- Modify: `scripts/README.md`

**Interfaces:**
- Produces: `scripts/verify.sh` with optional `--skip-install`; `scripts/audit.sh`; `npm run verify`; expanded `npm run test:scripts`.

- [ ] **Step 1: Add failing Node tests** for repo-root switching, exact verification order, official npm registry, Rust working directory/toolchain variables, missing cargo-audit failure, `--skip-install`, paths with spaces, and child exit propagation.
- [ ] **Step 2: Run `node --test scripts/build.test.mjs scripts/automation.test.mjs`** and verify failures are caused by missing scripts.
- [ ] **Step 3: Implement the two fail-fast scripts**; `verify.sh` runs install/typecheck/tests/build/docs/script tests/fmt/clippy/Rust tests, while `audit.sh` runs npm audit then cargo audit against `src-tauri/Cargo.lock`.
- [ ] **Step 4: Update package scripts and script documentation** to expose only the shared entrypoints.
- [ ] **Step 5: Re-run the Node tests** and verify they pass.
- [ ] **Step 6: Commit** with `git commit -m "build: centralize verification and audits"`.

### Task 2: Fix macOS identity and split the frontend bundle

**Files:**
- Modify: `src-tauri/tauri.conf.json`
- Modify: `vite.config.ts`
- Create: `scripts/check-build-output.mjs`
- Modify: `scripts/automation.test.mjs`
- Modify: `package.json`

**Interfaces:**
- Produces: `npm run check:bundle`; production sourcemap controlled by `RSHELL_SOURCEMAP=1`; deterministic vendor chunk groups.

- [ ] **Step 1: Add failing tests** asserting the exact Bundle ID, sourcemaps off by default/on by environment, named Vue/Element Plus/xterm/dockview chunks, no `.map` files by default, and every emitted JavaScript chunk at or below 500,000 bytes minified.
- [ ] **Step 2: Run `node --test scripts/automation.test.mjs && npm run build`** and verify the old ID, sourcemap, and monolithic bundle fail expectations.
- [ ] **Step 3: Implement `manualChunks` and build-output checking**; split Element Plus by stable package sub-area if its vendor chunk alone exceeds the threshold.
- [ ] **Step 4: Run `npm run build && npm run check:bundle`** and verify no chunk-size warning and no production source maps.
- [ ] **Step 5: Run `RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 npm run tauri:build -- --debug --bundles app`** and verify the `.app` builds without the identifier warning.
- [ ] **Step 6: Commit** with `git commit -m "build: fix macos identity and split frontend bundle"`.

### Task 3: Add macOS release preflight and app verification scripts

**Files:**
- Create: `scripts/macos-release-preflight.sh`
- Create: `scripts/macos-verify-app.sh`
- Modify: `scripts/automation.test.mjs`
- Modify: `scripts/README.md`

**Interfaces:**
- Produces: preflight `--unsigned` mode and strict signed mode; `macos-verify-app.sh <app-path>` validating plist ID, `codesign --verify --deep --strict`, and `spctl --assess`.

- [ ] **Step 1: Add failing fake-tool tests** for usage errors, exact Bundle ID, missing app, unsigned mode, missing identity/profile, paths with spaces, codesign/spctl failures, and secret redaction.
- [ ] **Step 2: Run `node --test scripts/automation.test.mjs`** and verify the scripts are missing.
- [ ] **Step 3: Implement both scripts** with explicit environment names `APPLE_SIGNING_IDENTITY` and `APPLE_NOTARY_PROFILE`; never echo their values.
- [ ] **Step 4: Run the Node tests** and then run preflight in `--unsigned` mode against the debug bundle.
- [ ] **Step 5: Commit** with `git commit -m "build: add macos release verification scripts"`.

### Task 4: Run shared checks in pinned macOS CI

**Files:**
- Create: `.github/workflows/ci.yml`
- Modify: `scripts/automation.test.mjs`
- Modify: `.gitignore`

**Interfaces:**
- Consumes: `scripts/verify.sh --skip-install` and `scripts/audit.sh`.
- Produces: macOS CI for pushes and pull requests; `.omo/` ignored.

- [ ] **Step 1: Add failing repository-policy tests** asserting `.omo/` is ignored, workflow actions are commit-SHA pinned, CI calls shared scripts, and it does not duplicate their internal npm/cargo check commands.
- [ ] **Step 2: Run `node --test scripts/automation.test.mjs`** and verify failures identify the missing workflow/ignore rule.
- [ ] **Step 3: Add the workflow** using the exact SHA pins in Global Constraints; install stable Rust and cargo-audit 0.22.2, run `npm ci`, shared verification with `--skip-install`, shared audit, debug app build, and unsigned preflight.
- [ ] **Step 4: Add `.omo/` to `.gitignore`** and run `git status --short` to verify existing `.omo/` state disappears without deleting it.
- [ ] **Step 5: Run the repository-policy tests** and verify they pass.
- [ ] **Step 6: Commit** with `git commit -m "ci: add macos verification workflow"`.

### Task 5: Align current release and validation documentation

**Files:**
- Modify: `README.md`
- Modify: `CONTRIBUTING.md`
- Modify: `docs/07-project-setup-guide.md`
- Modify: `docs/08-incomplete-features.md`
- Modify: `docs/09-macos-validation.md`
- Modify: `scripts/check-docs.mjs`

**Interfaces:**
- Consumes: all shared script entrypoints and current CI behavior.
- Produces: one non-duplicated operator path for verification, audit, release preflight, and app verification.

- [ ] **Step 1: Add failing doc-contract assertions** that current docs reference shared scripts, exact Bundle ID and credential variable names, and retain unchecked external signing/server/serial items.
- [ ] **Step 2: Run `npm run check:docs`** and verify it fails on old copied command lists or missing script references.
- [ ] **Step 3: Update current docs** to link to `scripts/README.md`, describe evidence capture, and remove duplicate command bodies while preserving honest unchecked validation items.
- [ ] **Step 4: Run `npm run check:docs && node --test scripts/build.test.mjs scripts/automation.test.mjs`** and verify all documentation/script contracts pass.
- [ ] **Step 5: Commit** with `git commit -m "docs: document repeatable release verification"`.

### Task 6: Final integrated verification

**Files:**
- Modify only if verification reveals a defect in files already owned by Tasks 1-5.

**Interfaces:**
- Consumes: all release automation deliverables.
- Produces: fresh evidence for the final report.

- [ ] **Step 1: Run `bash scripts/verify.sh --skip-install`** and require zero failures.
- [ ] **Step 2: Run `bash scripts/audit.sh`** after installing cargo-audit in the environment and require zero advisories, or report exact advisories without suppressing them.
- [ ] **Step 3: Run the debug macOS app build and `bash scripts/macos-release-preflight.sh --unsigned`** and require zero configuration errors.
- [ ] **Step 4: Run `git diff --check && git status --short --branch`** and confirm only intentional tracked changes remain; `.omo/` is absent from status.
- [ ] **Step 5: Record remaining external signing, notarization, SSH/SFTP, tunnel, gesture, plugin, and serial checks as unverified, not passed.**
