import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, rmSync, readFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const root = fileURLToPath(new URL('../', import.meta.url));
test('every configured desktop bundle icon exists', () => {
  const config = JSON.parse(readFileSync(join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
  for (const icon of config.bundle.icon) assert.ok(existsSync(join(root, 'src-tauri', icon)), `Missing bundle icon: ${icon}`);
});

test('macOS build wrapper calls Tauri from the repo and preserves target/errors', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'rshell-build-test-'));
  try {
    writeFileSync(join(fixture, 'npm'), '#!/bin/sh\npwd\nprintf "%s\\n" "$@"\nexit "${BUILD_TEST_EXIT:-0}"\n', { mode: 0o700 });
    const env = { ...process.env, PATH: `${fixture}:${process.env.PATH}` };
    const run = (args, extra = {}) => spawnSync('bash', [join(root, 'scripts/build.sh'), ...args], {
      cwd: fixture, env: { ...env, ...extra }, encoding: 'utf8', timeout: 5000,
    });
    const normal = run([]);
    assert.equal(normal.status, 0, normal.stderr);
    assert.deepEqual(normal.stdout.trim().split('\n'), [root.replace(/\/$/, ''), 'run', 'tauri:build', '--']);
    const target = run(['aarch64-apple-darwin']);
    assert.equal(target.status, 0, target.stderr);
    assert.deepEqual(target.stdout.trim().split('\n').slice(1), ['run', 'tauri:build', '--', '--target', 'aarch64-apple-darwin']);
    assert.equal(run([], { BUILD_TEST_EXIT: '17' }).status, 17);
    assert.equal(run(['one', 'two']).status, 2);
  } finally {
    rmSync(fixture, { recursive: true, force: true });
  }
});
