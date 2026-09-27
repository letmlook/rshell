#!/usr/bin/env node
// scripts/check-build-output.mjs
//
// Enforce the macOS release-readiness build-output contract:
//   - Every emitted JavaScript chunk under dist/assets/ is at most
//     MAX_CHUNK_BYTES (default 500_000) bytes when minified.
//   - When sourcemaps are disabled (the default for production), no `.map`
//     files are emitted under dist/assets/.
//
// Usage: node scripts/check-build-output.mjs [dist-dir]
//   dist-dir defaults to <repo>/dist (computed relative to this file).

import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const MAX_CHUNK_BYTES = 500_000;
const here = dirname(fileURLToPath(import.meta.url));
const distDir = resolve(process.argv[2] ?? join(here, "..", "dist"));
const assetsDir = join(distDir, "assets");

function listChunks(dir) {
  try {
    return readdirSync(dir);
  } catch (error) {
    if (error.code === "ENOENT") return [];
    throw error;
  }
}

function readSize(path) {
  return statSync(path).size;
}

const files = listChunks(assetsDir);
if (files.length === 0) {
  console.error(`No build artifacts found under ${assetsDir}; run \`npm run build\` first.`);
  process.exit(1);
}

const errors = [];
const jsChunks = files.filter((name) => name.endsWith(".js"));
const mapFiles = files.filter((name) => name.endsWith(".map"));

for (const name of jsChunks) {
  const size = readSize(join(assetsDir, name));
  if (size > MAX_CHUNK_BYTES) {
    errors.push(`chunk ${name} is ${size} bytes; limit is ${MAX_CHUNK_BYTES}`);
  }
}

// Sourcemaps must not appear in production builds. The Vite config only
// emits maps when RSHELL_SOURCEMAP=1; we still verify the on-disk result.
if (process.env.RSHELL_SOURCEMAP !== "1" && mapFiles.length > 0) {
  errors.push(
    `production build emitted ${mapFiles.length} sourcemap file(s); unset RSHELL_SOURCEMAP=1 to disable`,
  );
}

if (errors.length) {
  console.error(errors.join("\n"));
  process.exit(1);
}

// Surface vendor chunks so reviewers can confirm grouping from the log.
const vendor = jsChunks
  .filter((name) => name.startsWith("vendor-"))
  .map((name) => `${name} (${readSize(join(assetsDir, name))} bytes)`);
console.log(
  `Build output OK: ${jsChunks.length} js chunks, max ${MAX_CHUNK_BYTES} bytes; vendor groups: ${vendor.join(", ") || "(none)"}.`,
);