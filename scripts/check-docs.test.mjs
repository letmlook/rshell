import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, cp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

test('credential docs distinguish legacy migration from current plaintext claims', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'rshell-docs-'));
  try {
    for (const file of ['README.md', 'CLAUDE.md', 'CONTRIBUTING.md', 'CHANGELOG.md', 'LICENSE', 'docs', 'scripts']) {
      await cp(new URL(`../${file}`, import.meta.url), join(fixture, file), { recursive: true });
    }
    const readme = join(fixture, 'README.md');
    const original = await readFile(readme, 'utf8');
    for (const [sentence, expected] of [
      ['旧版会话配置可能包含明文密码，首次读取时会迁移到钥匙串。', 0],
      ['会话配置可能包含明文密码。', 1],
      ['旧版会话曾保存明文密码。当前会话 TOML 仍可能包含明文认证信息。', 1],
      ['旧版会话可能包含明文密码，但当前会话仍可能包含明文密码。', 1],
      ['主密码是凭据保险库。', 1],
    ]) {
      await writeFile(readme, `${original}\n${sentence}\n`);
      const result = spawnSync(process.execPath, [join(fixture, 'scripts/check-docs.mjs')], { encoding: 'utf8' });
      assert.equal(result.status, expected, `${sentence}\n${result.stderr}`);
    }
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});
