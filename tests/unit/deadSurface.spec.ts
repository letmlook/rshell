// tests/unit/deadSurface.spec.ts - 「后端已实现、界面无入口」的死面门禁
//
// AGENTS.md 把「后端已实现但界面无入口」的问题分成几类，其中两类此前
// **完全靠人工审计**：
//
//   1. 「两侧零引用的类型：Rust 与 TS 同时没有使用方的类型，是在声明
//       不存在的能力。」
//   2. 「字段声明了但恒为空。」
//
// 本文件把第 1 类变成门禁：任何一侧出现「声明了但全仓库零使用方」的
// 类型，测试立刻失败。
//
// 判据（与「有人用」的严格定义一致）：
//
//   · 一个类型算「有人用」，当且仅当它在**类型定义文件之外**被真实引用。
//   · **import 与 re-export 不算使用** —— 把名字从 A 搬到 B 不是有人用它。
//     （Rust 的 `pub use types::*;` 同理：globs 是搬运，不是消费。）
//   · 引用按**可达性**传递：A 被 B 引用、B 被生产代码引用 → A 也算有人用。
//     于是「只有死类型在引用它」的类型同样会暴露出来。
//
// 为什么不能只看「名字出现过」：rshell-api 的 lib.rs 里有 `pub use types::*;`，
// client.ts 里有 `export type { ProtocolType };` —— 这两处会让每一个类型
// 都「有引用」，门禁就静默失效了（与本仓库已有的「假绿」教训同类）。
//
// 新增一个类型却忘了接线时，本测试会指名道姓地报出来。

import { describe, expect, it, vi } from "vitest";

const { readFileSync, readdirSync } = await vi.importActual<{
  readFileSync(path: string, encoding: "utf8"): string;
  readdirSync(
    path: string,
    options: { withFileTypes: true },
  ): Array<{ name: string; isDirectory(): boolean; isFile(): boolean }>;
}>("node:fs");
const { join } = await vi.importActual<{ join(...paths: string[]): string }>("node:path");

// ============================================================================
// 通用文本工具
// ============================================================================

/** 返回与 source[start]（应为 '{'）配对的 '}' 下标；不配对返回 -1 */
function matchBrace(source: string, start: number): number {
  let depth = 0;
  for (let i = start; i < source.length; i++) {
    const ch = source[i];
    if (ch === "{") depth++;
    else if (ch === "}") {
      depth--;
      if (depth === 0) return i;
    }
  }
  return -1;
}

/** 去掉行注释与 doc 注释，避免注释里的示例被当成引用 */
function stripCommentLines(source: string): string {
  return source
    .split("\n")
    .filter((line) => {
      const t = line.trimStart();
      return !t.startsWith("//") && !t.startsWith("*") && !t.startsWith("/*");
    })
    .join("\n");
}

/** 去掉 import 语句（TS）与 use/import 语句（Rust）——搬运不算使用 */
function stripImports(source: string, lang: "ts" | "rust"): string {
  let out = source;
  if (lang === "ts") {
    // import type { A, B } from "x"; / import { A } from "x";
    out = out.replace(/import\s+(?:type\s+)?[\s\S]*?\s+from\s*["'][^"']*["']\s*;?/g, "");
    // export type { A, B }; / export { A } from "x";
    out = out.replace(/export\s+(?:type\s+)?\{[^}]*\}\s*(?:from\s*["'][^"']*["'])?\s*;?/g, "");
    // export * from "x";
    out = out.replace(/export\s+\*\s+from\s*["'][^"']*["']\s*;?/g, "");
  } else {
    // use foo::{A, B}; / use foo::*; / pub use foo::*;
    out = out.replace(/(?:pub\s+)?use\s+[\s\S]*?;/g, "");
  }
  return out;
}

/** 递归收集目录下指定后缀的文件（跳过 target/node_modules/dist） */
function walkFiles(dir: string, exts: string[]): string[] {
  const files: string[] = [];
  let entries: Array<{ name: string; isDirectory(): boolean; isFile(): boolean }>;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return files;
  }
  for (const entry of entries) {
    if (entry.name === "target" || entry.name === "node_modules" || entry.name === "dist") continue;
    const full = join(dir, entry.name);
    if (entry.isDirectory()) files.push(...walkFiles(full, exts));
    else if (exts.some((ext) => entry.name.endsWith(ext))) files.push(full);
  }
  return files;
}

/** 从文本里挑出属于 known 的标识符 */
function referencedNames(text: string, known: Set<string>): Set<string> {
  const found = new Set<string>();
  for (const m of text.matchAll(/\b[A-Za-z_][A-Za-z0-9_]*\b/g)) {
    if (known.has(m[0])) found.add(m[0]);
  }
  return found;
}

/**
 * 从根集合出发做可达性闭包。
 *
 * roots：真实被外部引用的类型名
 * edges：类型 → 它引用的其它类型
 */
function reachableFrom(roots: Set<string>, edges: Map<string, Set<string>>): Set<string> {
  const seen = new Set<string>();
  const stack = [...roots];
  while (stack.length > 0) {
    const name = stack.pop() as string;
    if (seen.has(name)) continue;
    seen.add(name);
    for (const next of edges.get(name) ?? []) {
      if (!seen.has(next)) stack.push(next);
    }
  }
  return seen;
}

// ============================================================================
// TypeScript 侧
// ============================================================================

const typesTsPath = "src/ipc/types.ts";
const typesTsSource = readFileSync(typesTsPath, "utf8");

interface TsDecl {
  name: string;
  body: string;
}

/**
 * 解析 types.ts 里 export 的 type / interface 声明及其正文。
 *
 * interface 的正文在 {} 里；type 别名的正文到「深度 0 的第一个分号」为止
 * （对象字面量别名内部的分号不会提前截断）。
 */
function parseTsDecls(source: string): TsDecl[] {
  const decls: TsDecl[] = [];
  const re = /^export (type|interface) (\w+)/gm;
  let m: RegExpExecArray | null;
  while ((m = re.exec(source)) !== null) {
    const kind = m[1];
    const name = m[2];
    const afterName = m.index + m[0].length;
    if (kind === "interface") {
      const braceStart = source.indexOf("{", afterName);
      const braceEnd = braceStart < 0 ? -1 : matchBrace(source, braceStart);
      if (braceEnd < 0) throw new Error(`interface ${name} 的大括号不配对`);
      decls.push({ name, body: source.slice(braceStart + 1, braceEnd) });
    } else {
      let depth = 0;
      let end = -1;
      for (let i = afterName; i < source.length; i++) {
        const ch = source[i];
        if (ch === "{" || ch === "(" || ch === "[") depth++;
        else if (ch === "}" || ch === ")" || ch === "]") depth--;
        else if (ch === ";" && depth === 0) {
          end = i;
          break;
        }
      }
      if (end < 0) throw new Error(`type ${name} 未以分号结束`);
      decls.push({ name, body: source.slice(afterName, end) });
    }
  }
  return decls;
}

const tsDecls = parseTsDecls(typesTsSource);
const tsNames = new Set(tsDecls.map((d) => d.name));

/** TS 类型的引用边 */
const tsEdges = new Map<string, Set<string>>(
  tsDecls.map((d) => [d.name, referencedNames(d.body, tsNames)]),
);

/**
 * TS 根集合：types.ts 之外的真实使用点。
 *
 * src/ 与 tests/ 都算使用点 —— 只有测试引用的类型至少还有契约在守着；
 * import / re-export 不算（见文件头）。
 */
function collectTsRoots(): Set<string> {
  const roots = new Set<string>();
  const files = [
    ...walkFiles("src", [".ts", ".vue"]),
    ...walkFiles("tests", [".ts", ".vue"]),
  ];
  for (const file of files) {
    if (file === typesTsPath) continue;
    const cleaned = stripImports(stripCommentLines(readFileSync(file, "utf8")), "ts");
    for (const name of referencedNames(cleaned, tsNames)) roots.add(name);
  }
  return roots;
}

// ============================================================================
// Rust 侧
// ============================================================================

const rustTypesPath = "src-tauri/crates/rshell-api/src/types.rs";
const rustTypesSource = readFileSync(rustTypesPath, "utf8");

interface RustDecl {
  name: string;
  body: string;
}

/** 解析 rshell-api/types.rs 里的 pub struct / pub enum 及其正文 */
function parseRustDecls(source: string): RustDecl[] {
  const decls: RustDecl[] = [];
  const re = /^pub (?:struct|enum) (\w+)/gm;
  let m: RegExpExecArray | null;
  while ((m = re.exec(source)) !== null) {
    const name = m[1];
    const braceStart = source.indexOf("{", m.index);
    const braceEnd = braceStart < 0 ? -1 : matchBrace(source, braceStart);
    if (braceEnd < 0) throw new Error(`${name} 的大括号不配对`);
    decls.push({ name, body: source.slice(braceStart + 1, braceEnd) });
  }
  return decls;
}

const rustDecls = parseRustDecls(rustTypesSource);
const rustNames = new Set(rustDecls.map((d) => d.name));

const rustEdges = new Map<string, Set<string>>(
  rustDecls.map((d) => [d.name, referencedNames(d.body, rustNames)]),
);

/**
 * Rust 根集合：types.rs 之外的真实使用点。
 *
 * `use` 语句被剥掉 —— 但 Rust 里未使用的 import 会被 clippy -D warnings
 * 拦下，所以能编译过�� import 一定是真的在用。
 */
function collectRustRoots(): Set<string> {
  const roots = new Set<string>();
  for (const file of walkFiles("src-tauri", [".rs"])) {
    if (file === rustTypesPath) continue;
    const cleaned = stripImports(stripCommentLines(readFileSync(file, "utf8")), "rust");
    for (const name of referencedNames(cleaned, rustNames)) roots.add(name);
  }
  return roots;
}

// ============================================================================
// 白名单
// ============================================================================

/**
 * 零引用类型的白名单。
 *
 * 每条都要写清楚「为什么它现在没有使用方」——白名单不是垃圾桶，
 * 是「已知的、已决定不接线的声明」。
 */
const DEAD_TYPE_WHITELIST: Record<string, string> = {};

// ============================================================================
// 断言
// ============================================================================

describe("死面门禁：两侧零引用的类型", () => {
  it("解析器没有失效（两侧都解析出了类型）", () => {
    // 解析器若因格式变化而返回空数组，下面的断言会平凡通过 ——
    // 这里给个下界，确保门禁真的在比对。
    expect(tsDecls.length).toBeGreaterThan(30);
    expect(rustDecls.length).toBeGreaterThan(30);
  });

  it("TypeScript 侧没有零引用的导出类型", () => {
    const roots = collectTsRoots();
    const alive = reachableFrom(roots, tsEdges);
    const dead = [...tsNames]
      .filter((name) => !alive.has(name))
      .filter((name) => !(name in DEAD_TYPE_WHITELIST));
    expect(dead).toEqual([]);
  });

  it("Rust 侧没有零引用的导出类型", () => {
    const roots = collectRustRoots();
    const alive = reachableFrom(roots, rustEdges);
    const dead = [...rustNames]
      .filter((name) => !alive.has(name))
      .filter((name) => !(name in DEAD_TYPE_WHITELIST));
    expect(dead).toEqual([]);
  });

  it("白名单只包含真实存在的类型（防止白名单腐化）", () => {
    const known = new Set([...tsNames, ...rustNames]);
    const unknown = Object.keys(DEAD_TYPE_WHITELIST).filter((name) => !known.has(name));
    expect(unknown).toEqual([]);
  });

  it("白名单每条都写明了不接线的原因", () => {
    const unreasoned = Object.entries(DEAD_TYPE_WHITELIST)
      .filter(([, reason]) => reason.trim().length < 4)
      .map(([name]) => name);
    expect(unreasoned).toEqual([]);
  });
});
