import { readFile, readdir, access } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const files = ['README.md', 'CLAUDE.md', 'CONTRIBUTING.md', 'CHANGELOG.md', 'scripts/README.md',
  ...(await readdir(resolve(root, 'docs'))).filter(name => /^0\d-.*\.md$/.test(name)).map(name => `docs/${name}`)];
const errors = [];
for (const file of files) {
  const body = await readFile(resolve(root, file), 'utf8');
  if (/\bGPUI\b|rshell-ui|cargo run\s+(?:-p|--package)/i.test(body)) {
    errors.push(`${file}: obsolete architecture or launch command`);
  }
  for (const line of body.split('\n')) {
    if (/\bRDP\b/.test(line) && !/删除|移出|不提供|拒绝/.test(line)) {
      errors.push(`${file}: RDP must only describe removed/rejected scope`);
    }
  }
  for (const match of body.matchAll(/\[[^\]]+\]\(([^)]+)\)/g)) {
    const target = match[1].split('#')[0];
    if (!target || /^[a-z]+:|^\//i.test(target)) continue;
    try { await access(resolve(root, dirname(file), target)); }
    catch { errors.push(`${file}: missing link ${target}`); }
  }
}
for (const file of ['README.md', 'docs/07-project-setup-guide.md']) {
  const body = await readFile(resolve(root, file), 'utf8');
  for (const command of ['npm run tauri:dev', 'npm run tauri:build']) {
    if (!body.includes(command)) errors.push(`${file}: missing ${command}`);
  }
}
if (errors.length) {
  console.error(errors.join('\n'));
  process.exitCode = 1;
} else {
  console.log(`Documentation contract passed (${files.length} current documents; dated historical records excluded).`);
}
