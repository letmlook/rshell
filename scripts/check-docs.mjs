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

const allCredentialDocs = (await Promise.all(files.map(file => readFile(resolve(root, file), 'utf8')))).join('\n');
if (!/(?:密码|凭据|口令).{0,40}(?:保存在|存放在|使用).{0,16}(?:macOS )?(?:钥匙串|Keychain)/i.test(allCredentialDocs)) {
  errors.push('credential docs: must identify macOS Keychain as the credential store');
}
if (!/迁移|migrat/i.test(allCredentialDocs) || !/旧版|legacy|明文/.test(allCredentialDocs)) {
  errors.push('credential docs: must explain migration of legacy plaintext credentials');
}
if (!/缺失|missing/i.test(allCredentialDocs) || !/重新输入|重新保存|re-enter|re-entering|resave|save again/i.test(allCredentialDocs)) {
  errors.push('credential docs: must explain how to recover a missing Keychain entry');
}
if (/会话.{0,50}(?:可能含|可能包含|仍可能包含).{0,10}明文|会话 TOML 仍可能包含明文认证信息|主密码[^。\n]*(?:尚未|未成为|并未)[^。\n]*(?:会话存储|加密保险库)|主密码(?![^。\n]*(?:不是|并非))[^。\n]*(?:凭据保险库|密码库|vault)/i.test(allCredentialDocs)) {
  errors.push('credential docs: must not claim session secrets remain plaintext or that the master password is the credential vault');
}
if (errors.length) {
  console.error(errors.join('\n'));
  process.exitCode = 1;
} else {
  console.log(`Documentation contract passed (${files.length} current documents; dated historical records excluded).`);
}
