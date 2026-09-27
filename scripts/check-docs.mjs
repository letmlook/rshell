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
// Evaluate separate claims so a truthful legacy migration sentence cannot
// hide a current-state claim later in the same paragraph.
const plaintextClaim = /会话.{0,50}(?:可能含|可能包含|仍可能包含).{0,10}明文/i;
const currentPlaintextClaim = allCredentialDocs.split(/[。；;\n]|(?:，|,)?(?:但是|但|然而)/).some(claim =>
  plaintextClaim.test(claim) && !/^\s*(?:[-*]\s*)?(?:读取)?旧版/.test(claim),
);
if (currentPlaintextClaim || /主密码[^。\n]*(?:尚未|未成为|并未)[^。\n]*(?:会话存储|加密保险库)|主密码(?![^。\n]*(?:不是|并非))[^。\n]*(?:凭据保险库|密码库|vault)/i.test(allCredentialDocs)) {
  errors.push('credential docs: must not claim session secrets remain plaintext or that the master password is the credential vault');
}

// Release-readiness docs must point operators at the shared script entry
// points (verify, audit, preflight, verify-app) instead of duplicating
// internal commands. README, CONTRIBUTING, docs/07 and docs/08 each have to
// surface at least one of these references.
const releaseReadinessDocs = ['README.md', 'CONTRIBUTING.md', 'docs/07-project-setup-guide.md', 'docs/08-incomplete-features.md'];
const releaseBody = (await Promise.all(releaseReadinessDocs.map(file => readFile(resolve(root, file), 'utf8')))).join('\n');
if (!/scripts\/verify\.sh/.test(releaseBody) || !/scripts\/audit\.sh/.test(releaseBody)) {
  errors.push('release-readiness docs: must reference scripts/verify.sh and scripts/audit.sh');
}
if (!/scripts\/macos-release-preflight\.sh/.test(releaseBody) || !/scripts\/macos-verify-app\.sh/.test(releaseBody)) {
  errors.push('release-readiness docs: must reference scripts/macos-release-preflight.sh and scripts/macos-verify-app.sh');
}
if (!/com\.letmlook\.rshell/.test(releaseBody)) {
  errors.push('release-readiness docs: must state the exact Bundle ID com.letmlook.rshell');
}
if (!/APPLE_SIGNING_IDENTITY/.test(releaseBody) || !/APPLE_NOTARY_PROFILE/.test(releaseBody)) {
  errors.push('release-readiness docs: must name APPLE_SIGNING_IDENTITY and APPLE_NOTARY_PROFILE explicitly');
}

// Dated historical records may still mention the old RShell identifier,
// but no current doc may pretend com.rshell.app is still shipping.
for (const file of files) {
  const body = await readFile(resolve(root, file), 'utf8');
  if (/com\.rshell\.app/.test(body)) {
    errors.push(`${file}: must not mention the old com.rshell.app Bundle ID`);
  }
}

// The release-readiness docs must continue to admit that signing,
// notarization, real SSH/SFTP, tunnels, gestures, plugins, and physical
// serial ports remain unverified without external evidence.
const honestLimits = ['docs/08-incomplete-features.md', 'docs/09-macos-validation.md'];
const limitsBody = (await Promise.all(honestLimits.map(file => readFile(resolve(root, file), 'utf8')))).join('\n');
const requiredLimits = ['签名', '公证', '物理串口|Serial', 'SSH', 'SFTP'];
for (const phrase of requiredLimits) {
  const regex = new RegExp(phrase);
  if (!regex.test(limitsBody)) {
    errors.push(`release-readiness docs: must keep "${phrase}" listed as unverified external validation`);
  }
}

if (errors.length) {
  console.error(errors.join('\n'));
  process.exitCode = 1;
} else {
  console.log(`Documentation contract passed (${files.length} current documents; dated historical records excluded).`);
}
