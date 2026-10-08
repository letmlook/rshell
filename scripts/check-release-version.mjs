#!/usr/bin/env node
// RShell release version guard.
//
// Usage: node scripts/check-release-version.mjs [tag]
//
// Without an argument the script only asserts that every version source in the
// repository agrees with the others. With a tag (`v0.2.0`, `0.2.0`) the shared
// version must additionally equal the tag, so a mis-tagged release fails before
// any installer is built.
//
// The bundlers read `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and
// `package.json` separately, and nothing in the build enforces that they agree.
// A tag release whose tag says v0.2.0 while the bundles report 0.1.0 would
// publish installers that lie about their own version, so CI runs this check
// before the release matrix.
//
// The script never prints credentials and reports every mismatch at once
// instead of failing on the first one.

import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

/** Strip a leading `v` so `v0.2.0` and `0.2.0` compare equal. */
function stripTagPrefix(tag) {
  const trimmed = tag.trim();
  return trimmed.replace(/^[vV]/, '');
}

function readCargoWorkspaceVersion(body) {
  // Only `[workspace.package]` counts: `[workspace.dependencies]` and the
  // package manifests carry their own, unrelated `version` entries, so the
  // version is read from that section alone.
  let inSection = false;
  for (const rawLine of body.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (line.startsWith('[')) {
      inSection = line === '[workspace.package]';
      continue;
    }
    if (!inSection) continue;
    const match = /^version\s*=\s*"([^"]+)"/.exec(line);
    if (match) return match[1];
  }
  return null;
}

const pkg = JSON.parse(await readFile(resolve(root, 'package.json'), 'utf8'));
const tauriConf = JSON.parse(await readFile(resolve(root, 'src-tauri/tauri.conf.json'), 'utf8'));
const cargoToml = await readFile(resolve(root, 'src-tauri/Cargo.toml'), 'utf8');

const sources = [
  ['package.json', pkg.version],
  ['src-tauri/tauri.conf.json', tauriConf.version],
  ['src-tauri/Cargo.toml [workspace.package]', readCargoWorkspaceVersion(cargoToml)],
];

const errors = [];
for (const [name, value] of sources) {
  if (typeof value !== 'string' || value.length === 0) {
    errors.push(`${name}: no version field found`);
  } else if (!SEMVER.test(value)) {
    errors.push(`${name}: "${value}" is not a semantic version`);
  }
}

const tag = process.argv[2];
let expected = sources[0][1];
if (tag) {
  const normalized = stripTagPrefix(tag);
  if (!SEMVER.test(normalized)) {
    errors.push(`tag "${tag}": expected a semantic version such as v0.2.0`);
  }
  expected = normalized;
}

// Compare every source against the same expectation instead of against each
// other, so the output always names one release version to converge on.
const distinct = [...new Set(sources.map(([, value]) => value))];
if (distinct.length > 1) {
  errors.push(`version mismatch: ${sources.map(([name, value]) => `${name}=${value}`).join(', ')}`);
}
for (const [name, value] of sources) {
  if (typeof value === 'string' && value !== expected) {
    errors.push(`${name}: "${value}" must be "${expected}"`);
  }
}

if (errors.length > 0) {
  console.error('Release version check failed:');
  for (const message of errors) console.error(`  - ${message}`);
  process.exitCode = 1;
} else {
  const scope = tag ? `tag ${tag}` : 'repository';
  console.log(`Release version check passed (${scope}, version ${expected}).`);
}