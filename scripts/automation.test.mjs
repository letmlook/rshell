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