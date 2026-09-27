import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, rmSync, mkdirSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const root = fileURLToPath(new URL('../', import.meta.url));
const verify = join(root, 'scripts/verify.sh');
const audit = join(root, 'scripts/audit.sh');

function makeFakeBin(name, log) {
  const tc = '${RUSTUP_TOOLCHAIN:-unset}';
  const nu = '${RUSTUP_NO_UPDATE_CHECK:-unset}';
  const fakeExit = '${FAKE_EXIT:-0}';
  return [
    '#!/bin/sh',
    `printf '%s TC=%s NU=%s ' "${name}" "${tc}" "${nu}" >> "${log}"`,
    `printf '%s\\n' "$*" >> "${log}"`,
    `exit "${fakeExit}"`,
    '',
  ].join('\n');
}

function makeFailingFakeBin(name, log, code = '17') {
  const tc = '${RUSTUP_TOOLCHAIN:-unset}';
  const nu = '${RUSTUP_NO_UPDATE_CHECK:-unset}';
  return [
    '#!/bin/sh',
    `printf '%s TC=%s NU=%s ' "${name}" "${tc}" "${nu}" >> "${log}"`,
    `printf '%s\\n' "$*" >> "${log}"`,
    `exit "${code}"`,
    '',
  ].join('\n');
}

function setupFixture({ failingArgv = '', includeCargoAudit = true } = {}) {
  const fixture = mkdtempSync(join(tmpdir(), 'rshell-automation-'));
  const logPath = join(fixture, 'invocations.log');
  const wrapper = (name) => {
    const tc = '${RUSTUP_TOOLCHAIN:-unset}';
    const nu = '${RUSTUP_NO_UPDATE_CHECK:-unset}';
    return [
      '#!/bin/sh',
      `printf '%s TC=%s NU=%s ' "${name}" "${tc}" "${nu}" >> "${logPath}"`,
      `printf '%s\\n' "$*" >> "${logPath}"`,
      `if [ -n "${failingArgv}" ] && [ "x$*" = "x${failingArgv}" ]; then exit 17; fi`,
      'exit 0',
      '',
    ].join('\n');
  };
  writeFileSync(join(fixture, 'npm'), wrapper('npm'), { mode: 0o700 });
  writeFileSync(join(fixture, 'cargo'), wrapper('cargo'), { mode: 0o700 });
  if (includeCargoAudit) {
    writeFileSync(join(fixture, 'cargo-audit'), wrapper('cargo-audit'), { mode: 0o700 });
  }
  mkdirSync(join(fixture, 'workspace with space'), { recursive: true });
  return { fixture, logPath };
}

function cleanup(fixture) {
  rmSync(fixture, { recursive: true, force: true });
}

function readLog(logPath) {
  return readFileSync(logPath, 'utf8');
}

function runVerify(args, { cwd, fixture, logPath, extra = {} } = {}) {
  const env = {
    ...process.env,
    PATH: `${fixture}:${process.env.PATH}`,
    ...extra,
  };
  delete env.RUSTUP_TOOLCHAIN;
  delete env.RUSTUP_NO_UPDATE_CHECK;
  return spawnSync('bash', [verify, ...args], {
    cwd: cwd ?? fixture,
    env,
    encoding: 'utf8',
    timeout: 30000,
  });
}

function runAudit(args, { cwd, fixture, extra = {} } = {}) {
  const env = {
    ...process.env,
    PATH: `${fixture}:${process.env.PATH}`,
    ...extra,
  };
  delete env.RUSTUP_TOOLCHAIN;
  delete env.RUSTUP_NO_UPDATE_CHECK;
  return spawnSync('bash', [audit, ...args], {
    cwd: cwd ?? fixture,
    env,
    encoding: 'utf8',
    timeout: 30000,
  });
}

test('verify.sh switches to repo root regardless of caller cwd', () => {
  const { fixture, logPath } = setupFixture();
  try {
    const result = runVerify([], { cwd: fixture, fixture, logPath });
    assert.equal(result.status, 0, result.stderr);
    const invocations = readLog(logPath).trim().split('\n');
    // First invocation must be `npm ci` from the repo root (env-suffix ignored).
    assert.match(invocations[0], /^npm\s+TC=\S+\s+NU=\S+\s+ci$/);
    for (const line of invocations) {
      assert.ok(!line.includes(fixture), `verify.sh did not switch away from caller cwd: ${line}`);
    }
  } finally {
    cleanup(fixture);
  }
});

test('verify.sh runs each expected step in the documented order', () => {
  const { fixture, logPath } = setupFixture();
  try {
    const result = runVerify(['--skip-install'], { cwd: fixture, fixture, logPath });
    assert.equal(result.status, 0, result.stderr);
    const expectedArgv = [
      'run typecheck',
      'test',
      'run build',
      'run check:docs',
      'run check:bundle',
      'run test:scripts',
      'fmt --all --check',
      'clippy --workspace --all-targets -- -D warnings',
      'test --workspace',
    ];
    const lines = readLog(logPath).trim().split('\n');
    let idx = 0;
    for (const argv of expectedArgv) {
      const found = lines.findIndex((line, i) => i >= idx && line.endsWith(` ${argv}`));
      assert.ok(found >= 0, `verify.sh did not run ${argv}: ${lines.join(' | ')}`);
      idx = found + 1;
    }
  } finally {
    cleanup(fixture);
  }
});

test('verify.sh sets RUSTUP_TOOLCHAIN=stable and RUSTUP_NO_UPDATE_CHECK=1 for cargo steps', () => {
  const { fixture, logPath } = setupFixture();
  try {
    const result = runVerify(['--skip-install'], { cwd: fixture, fixture, logPath });
    assert.equal(result.status, 0, result.stderr);
    const body = readLog(logPath);
    assert.match(body, /TC=stable/, 'cargo calls must run under stable toolchain');
    assert.match(body, /NU=1/, 'cargo calls must set RUSTUP_NO_UPDATE_CHECK=1');
  } finally {
    cleanup(fixture);
  }
});

test('verify.sh --skip-install omits npm ci but keeps the remaining steps', () => {
  const { fixture, logPath } = setupFixture();
  try {
    const result = runVerify(['--skip-install'], { cwd: fixture, fixture, logPath });
    assert.equal(result.status, 0, result.stderr);
    const body = readLog(logPath);
    assert.ok(!/\sci\s/.test(body), 'npm ci should not run when --skip-install is passed');
    assert.match(body, /\srun typecheck$/m);
    assert.match(body, /\stest --workspace$/m);
  } finally {
    cleanup(fixture);
  }
});

test('verify.sh supports a working directory whose path contains spaces', () => {
  const { fixture, logPath } = setupFixture();
  try {
    const spaced = join(fixture, 'workspace with space');
    const result = runVerify(['--skip-install'], { cwd: spaced, fixture, logPath });
    assert.equal(result.status, 0, result.stderr);
    const body = readLog(logPath);
    assert.match(body, /\srun typecheck$/m);
  } finally {
    cleanup(fixture);
  }
});

test('verify.sh propagates child exit codes unchanged', () => {
  const { fixture, logPath } = setupFixture({ failingArgv: 'test' });
  try {
    const result = runVerify(['--skip-install'], { cwd: fixture, fixture, logPath });
    assert.equal(result.status, 17, `expected child exit 17 to propagate, got ${result.status}: ${result.stderr}`);
  } finally {
    cleanup(fixture);
  }
});

test('audit.sh pins npm audit to the official registry', () => {
  const { fixture, logPath } = setupFixture();
  try {
    const result = runAudit([], { cwd: fixture, fixture, logPath });
    assert.equal(result.status, 0, result.stderr);
    const body = readLog(logPath);
    assert.ok(/ audit[^\n]*--registry=https:\/\/registry\.npmjs\.org/.test(body), `npm audit registry not pinned: ${body}`);
    assert.match(body, /\s--file src-tauri\/Cargo\.lock$/m);
  } finally {
    cleanup(fixture);
  }
});

test('audit.sh runs cargo audit against src-tauri/Cargo.lock', () => {
  const { fixture, logPath } = setupFixture();
  try {
    const result = runAudit([], { cwd: fixture, fixture, logPath });
    assert.equal(result.status, 0, result.stderr);
    const body = readLog(logPath);
    assert.match(body, /src-tauri\/Cargo\.lock/, `cargo audit did not target src-tauri/Cargo.lock: ${body}`);
  } finally {
    cleanup(fixture);
  }
});

test('audit.sh fails with a clear error when cargo-audit is not installed', () => {
  const { fixture } = setupFixture({ includeCargoAudit: false });
  try {
    const result = runAudit([], { cwd: fixture, fixture });
    assert.notEqual(result.status, 0, 'audit.sh must exit nonzero when cargo-audit is missing');
    assert.match(result.stderr || result.stdout, /cargo-audit/, 'error must mention cargo-audit');
  } finally {
    cleanup(fixture);
  }
});

test('audit.sh sets RUSTUP_TOOLCHAIN=stable and RUSTUP_NO_UPDATE_CHECK=1 for cargo audit', () => {
  const { fixture, logPath } = setupFixture();
  try {
    const result = runAudit([], { cwd: fixture, fixture, logPath });
    assert.equal(result.status, 0, result.stderr);
    const body = readLog(logPath);
    assert.match(body, /TC=stable/);
    assert.match(body, /NU=1/);
  } finally {
    cleanup(fixture);
  }
});

test('audit.sh propagates child exit codes unchanged', () => {
  const { fixture } = setupFixture({ failingArgv: 'audit --file src-tauri/Cargo.lock' });
  try {
    const result = runAudit([], { cwd: fixture, fixture });
    assert.equal(result.status, 17, `expected cargo audit failure to propagate, got ${result.status}: ${result.stderr}`);
  } finally {
    cleanup(fixture);
  }
});

// --- Build-output contract ----------------------------------------------------

function makeDistWith(thresholdBytes, { withSourcemap = false } = {}) {
  const fixture = mkdtempSync(join(tmpdir(), 'rshell-build-output-'));
  const assets = join(fixture, 'assets');
  mkdirSync(assets, { recursive: true });
  const small = Buffer.alloc(thresholdBytes - 1, 0);
  writeFileSync(join(assets, 'vendor-vue.js'), small);
  writeFileSync(join(assets, 'vendor-element-plus.js'), small);
  writeFileSync(join(assets, 'vendor-xterm.js'), small);
  writeFileSync(join(assets, 'vendor-dockview.js'), small);
  writeFileSync(join(assets, 'vendor-pinia.js'), small);
  writeFileSync(join(assets, 'app.js'), small);
  if (withSourcemap) writeFileSync(join(assets, 'app.js.map'), small);
  return fixture;
}

function runCheckBuildOutput(distDir, extra = {}) {
  return spawnSync('node', [join(root, 'scripts/check-build-output.mjs'), distDir], {
    cwd: root,
    env: { ...process.env, ...extra },
    encoding: 'utf8',
    timeout: 10000,
  });
}

test('check-build-output.mjs accepts a dist whose JS chunks stay under the limit', () => {
  const fixture = makeDistWith(500_000);
  try {
    const result = runCheckBuildOutput(fixture);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /Build output OK/);
  } finally {
    cleanup(fixture);
  }
});

test('check-build-output.mjs fails when any JS chunk exceeds the 500 KiB limit', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'rshell-build-output-'));
  const assets = join(fixture, 'assets');
  mkdirSync(assets, { recursive: true });
  writeFileSync(join(assets, 'small.js'), Buffer.alloc(10_000, 0));
  writeFileSync(join(assets, 'huge.js'), Buffer.alloc(600_000, 0));
  try {
    const result = runCheckBuildOutput(fixture);
    assert.notEqual(result.status, 0, 'oversized chunk must fail');
    assert.match(result.stderr, /huge\.js is 600000 bytes/);
  } finally {
    cleanup(fixture);
  }
});

test('check-build-output.mjs fails when production builds emit sourcemaps', () => {
  const fixture = makeDistWith(500_000, { withSourcemap: true });
  try {
    const result = runCheckBuildOutput(fixture);
    assert.notEqual(result.status, 0, 'production sourcemap must fail');
    assert.match(result.stderr, /sourcemap/);
  } finally {
    cleanup(fixture);
  }
});

test('check-build-output.mjs tolerates sourcemaps when RSHELL_SOURCEMAP=1', () => {
  const fixture = makeDistWith(500_000, { withSourcemap: true });
  try {
    const result = runCheckBuildOutput(fixture, { RSHELL_SOURCEMAP: '1' });
    assert.equal(result.status, 0, result.stderr);
  } finally {
    cleanup(fixture);
  }
});

test('check-build-output.mjs requires a populated dist directory', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'rshell-build-output-'));
  try {
    const result = runCheckBuildOutput(fixture);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /No build artifacts found/);
  } finally {
    cleanup(fixture);
  }
});

// --- macOS preflight / verify-app -------------------------------------------

function makeAppBundle(parent, bundleId = 'com.letmlook.rshell') {
  const dir = join(parent, 'RShell.app');
  mkdirSync(join(dir, 'Contents'), { recursive: true });
  writeFileSync(
    join(dir, 'Contents', 'Info.plist'),
    `<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict><key>CFBundleIdentifier</key><string>${bundleId}</string></dict>
</plist>
`,
  );
  return dir;
}

function fakePlistBuddy(home) {
  writeFileSync(
    join(home, 'PlistBuddy'),
    [
      '#!/bin/sh',
      'exec /usr/libexec/PlistBuddy "$@"',
      '',
    ].join('\n'),
    { mode: 0o700 },
  );
}

function runPreflight(args, { cwd, env = {}, unsetSecrets = true } = {}) {
  const script = join(root, 'scripts/macos-release-preflight.sh');
  const finalEnv = { ...process.env, ...env };
  if (unsetSecrets) {
    delete finalEnv.APPLE_SIGNING_IDENTITY;
    delete finalEnv.APPLE_NOTARY_PROFILE;
  }
  return spawnSync('bash', [script, ...args], {
    cwd: cwd ?? root,
    env: finalEnv,
    encoding: 'utf8',
    timeout: 15000,
  });
}

function runVerifyApp(args, { cwd, env = {} } = {}) {
  const script = join(root, 'scripts/macos-verify-app.sh');
  return spawnSync('bash', [script, ...args], {
    cwd: cwd ?? root,
    env: { ...process.env, ...env },
    encoding: 'utf8',
    timeout: 15000,
  });
}

test('macos-release-preflight.sh --unsigned succeeds without secrets when the bundle is correct', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'rshell-preflight-'));
  try {
    fakePlistBuddy(fixture);
    const app = makeAppBundle(fixture);
    const env = { PATH: `${fixture}:${process.env.PATH}` };
    const result = runPreflight(['--unsigned', app], { env });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stderr, /Preflight \(unsigned\)/);
  } finally {
    cleanup(fixture);
  }
});

test('macos-release-preflight.sh refuses --unsigned when signing/notary secrets are set', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'rshell-preflight-'));
  try {
    fakePlistBuddy(fixture);
    const app = makeAppBundle(fixture);
    const env = {
      PATH: `${fixture}:${process.env.PATH}`,
      APPLE_SIGNING_IDENTITY: 'Developer ID Application: Example (XXXXXXXXXX)',
      APPLE_NOTARY_PROFILE: 'rshell-notary',
    };
    const result = runPreflight(['--unsigned', app], { env, unsetSecrets: false });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Refusing to mix --unsigned with signing\/notary credentials/);
  } finally {
    cleanup(fixture);
  }
});

test('macos-release-preflight.sh signed mode fails when APPLE_SIGNING_IDENTITY is missing', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'rshell-preflight-'));
  try {
    fakePlistBuddy(fixture);
    const app = makeAppBundle(fixture);
    const env = {
      PATH: `${fixture}:${process.env.PATH}`,
      APPLE_NOTARY_PROFILE: 'rshell-notary',
    };
    const result = runPreflight([app], { env });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /APPLE_SIGNING_IDENTITY and APPLE_NOTARY_PROFILE must be set/);
  } finally {
    cleanup(fixture);
  }
});

test('macos-release-preflight.sh signed mode fails when APPLE_NOTARY_PROFILE is missing', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'rshell-preflight-'));
  try {
    fakePlistBuddy(fixture);
    const app = makeAppBundle(fixture);
    const env = {
      PATH: `${fixture}:${process.env.PATH}`,
      APPLE_SIGNING_IDENTITY: 'Developer ID Application: Example (XXXXXXXXXX)',
    };
    const result = runPreflight([app], { env });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /APPLE_SIGNING_IDENTITY and APPLE_NOTARY_PROFILE must be set/);
  } finally {
    cleanup(fixture);
  }
});

test('macos-release-preflight.sh reports a missing app bundle with a clear error', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'rshell-preflight-'));
  try {
    const missing = join(fixture, 'Missing.app');
    const result = runPreflight(['--unsigned', missing]);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /App bundle not found/);
  } finally {
    cleanup(fixture);
  }
});

test('macos-release-preflight.sh rejects unexpected Bundle IDs', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'rshell-preflight-'));
  try {
    fakePlistBuddy(fixture);
    const app = makeAppBundle(fixture, 'com.example.rshell');
    const env = { PATH: `${fixture}:${process.env.PATH}` };
    const result = runPreflight(['--unsigned', app], { env });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Unexpected Bundle ID/);
  } finally {
    cleanup(fixture);
  }
});

test('macos-release-preflight.sh supports app paths containing spaces', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'rshell-preflight-'));
  try {
    fakePlistBuddy(fixture);
    const parentWithSpace = join(fixture, 'path with space');
    mkdirSync(parentWithSpace, { recursive: true });
    const app = makeAppBundle(parentWithSpace);
    const env = { PATH: `${fixture}:${process.env.PATH}` };
    const result = runPreflight(['--unsigned', app], { env });
    assert.equal(result.status, 0, result.stderr);
  } finally {
    cleanup(fixture);
  }
});

test('macos-release-preflight.sh rejects unknown flags with exit code 2', () => {
  const result = runPreflight(['--bogus']);
  assert.equal(result.status, 2);
  assert.match(result.stderr, /Unknown flag/);
});

test('macos-release-preflight.sh never echoes signing or notary secrets', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'rshell-preflight-'));
  try {
    fakePlistBuddy(fixture);
    const app = makeAppBundle(fixture);
    const identity = 'SECRET-SIGNING-IDENTITY-XYZ';
    const profile = 'SECRET-NOTARY-PROFILE-XYZ';
    const env = {
      PATH: `${fixture}:${process.env.PATH}`,
      APPLE_SIGNING_IDENTITY: identity,
      APPLE_NOTARY_PROFILE: profile,
    };
    const result = runPreflight(['--unsigned', app], { env, unsetSecrets: false });
    assert.match(result.stdout + result.stderr, /Refusing to mix --unsigned with signing\/notary credentials/);
    assert.ok(!result.stdout.includes(identity), 'identity leaked to stdout');
    assert.ok(!result.stderr.includes(identity), 'identity leaked to stderr');
    assert.ok(!result.stdout.includes(profile), 'notary profile leaked to stdout');
    assert.ok(!result.stderr.includes(profile), 'notary profile leaked to stderr');
  } finally {
    cleanup(fixture);
  }
});

test('macos-verify-app.sh rejects usage errors', () => {
  assert.equal(runVerifyApp([]).status, 2);
  assert.equal(runVerifyApp(['one', 'two']).status, 2);
});

test('macos-verify-app.sh reports a missing bundle', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'rshell-verify-'));
  try {
    const result = runVerifyApp([join(fixture, 'Missing.app')]);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /App bundle not found/);
  } finally {
    cleanup(fixture);
  }
});

test('macos-verify-app.sh rejects bundles without Contents/Info.plist', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'rshell-verify-'));
  try {
    const dir = join(fixture, 'RShell.app');
    mkdirSync(dir, { recursive: true });
    const result = runVerifyApp([dir]);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Missing Contents\/Info\.plist/);
  } finally {
    cleanup(fixture);
  }
});

test('macos-verify-app.sh rejects bundles with the wrong Bundle ID', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'rshell-verify-'));
  try {
    fakePlistBuddy(fixture);
    const app = makeAppBundle(fixture, 'com.example.rshell');
    const env = { PATH: `${fixture}:${process.env.PATH}` };
    const result = runVerifyApp([app], { env });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Unexpected Bundle ID/);
  } finally {
    cleanup(fixture);
  }
});

// --- Repository CI / ignore policy ------------------------------------------

const ciWorkflowPath = join(root, '.github/workflows/ci.yml');

function readCiWorkflow() {
  return readFileSync(ciWorkflowPath, 'utf8');
}

test('.omo/ is ignored so local Agent state does not pollute the workspace', () => {
  const ignore = readFileSync(join(root, '.gitignore'), 'utf8');
  assert.match(ignore, /^\.omo\//m, '.gitignore must list .omo/ to keep Agent state out of the tree');
});

test('macOS CI workflow pins every third-party action to a full commit SHA', () => {
  const workflow = readCiWorkflow();
  // Each `uses:` must reference a 40-character SHA, never a moving tag.
  for (const match of workflow.matchAll(/uses:\s*([^@\s]+)@([0-9a-f]+)/g)) {
    const action = match[1];
    const sha = match[2];
    assert.ok(/^[0-9a-f]{40}$/.test(sha), `${action} must be pinned to a full 40-char SHA, got "${sha}"`);
  }
  // Spot-check the SHAs recorded in the release-readiness plan so a future
  // bump requires an explicit decision instead of a silent tag update.
  assert.match(workflow, /actions\/checkout@11d5960a326750d5838078e36cf38b85af677262/);
  assert.match(workflow, /actions\/setup-node@49933ea5288caeca8642d1e84afbd3f7d6820020/);
  assert.match(workflow, /actions\/cache@0057852bfaa89a56745cba8c7296529d2fc39830/);
});

test('macOS CI workflow delegates to the shared scripts and does not duplicate their commands', () => {
  const workflow = readCiWorkflow();
  assert.match(workflow, /bash scripts\/verify\.sh --skip-install/);
  assert.match(workflow, /bash scripts\/audit\.sh/);
  assert.match(workflow, /bash scripts\/macos-release-preflight\.sh --unsigned/);
  // The workflow must not re-implement the verification or audit chain.
  for (const duplicate of [
    /npm run typecheck/,
    /npm run check:docs/,
    /npm run test:scripts/,
    /cargo fmt/,
    /cargo clippy/,
    /cargo test --workspace/,
    /npm audit/,
    /cargo audit/,
  ]) {
    assert.ok(
      !duplicate.test(workflow),
      `CI workflow duplicates shared-script command ${duplicate}; call the script instead`,
    );
  }
});

test('macOS CI workflow installs cargo-audit 0.22.2 with --locked', () => {
  const workflow = readCiWorkflow();
  assert.match(workflow, /cargo install cargo-audit --version 0\.22\.2 --locked/);
});

test('macOS CI workflow pins Rust toolchain via RUSTUP_* env, not a fixed toolchain file', () => {
  const workflow = readCiWorkflow();
  assert.match(workflow, /RUSTUP_TOOLCHAIN:\s*stable/);
  assert.match(workflow, /RUSTUP_NO_UPDATE_CHECK:\s*1/);
  // No "rust-toolchain:" file pinning is allowed in CI; the project ships
  // rust-toolchain.toml for local use only.
  const toolchainFile = readFileSync(join(root, 'rust-toolchain.toml'), 'utf8');
  assert.ok(!workflow.includes('rust-toolchain:'), 'CI must not depend on rust-toolchain.toml');
  // Sanity: the file itself still exists.
  assert.match(toolchainFile, /\[toolchain\]/);
});