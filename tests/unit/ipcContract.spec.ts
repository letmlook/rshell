// tests/unit/ipcContract.spec.ts - PROB-01 IPC 参数键契约对账
//
// 契约（写入 src-tauri/src/commands.rs 文件头）：后端全部 #[tauri::command]
// （含 cmd! 宏模板）声明 rename_all = "snake_case"，前端 invoke 的参数键必须
// 等于 Rust 形参名。Tauri 2 默认参数键是 camelCase（tauri-macros wrapper.rs
// 默认 ArgumentCase::Camel），键名不一致时命令在参数提取阶段即以
// missing key 被拒。
//
// 本测试双向对账，任何一侧漂移都会失败：
// 1. 解析 commands.rs 全部命令签名（手写属性 + cmd! 宏展开 + 宏模板）；
// 2. 解析 client.ts 全部 call() helper 负载，以及 src/ 下全部直接
//    invoke("命令", {...}) 调用点（当前唯一一个在 TerminalPane.vue）；
// 3. 逐点断言参数键一致；带参后端命令必须有前端调用点（push_one_mb 白名单）；
//    lib.rs generate_handler 注册表与 commands.rs 签名一一对应。
//
// 注入错误键名（如把 session_id 改成 sessionId）、去掉后端 rename_all、
// 或新增命令漏掉一侧，都会让对应断言立即失败。
//
// 解析局限：键提取按括号深度扫描，契约文件不含含花括号的字符串字面量或
// 行中注释；若未来出现此类内容，需同步升级这里的解析器。

import { describe, expect, it, vi } from "vitest";

const { readFileSync, readdirSync } = await vi.importActual<{
  readFileSync(path: string, encoding: "utf8"): string;
  readdirSync(
    path: string,
    options: { withFileTypes: true },
  ): Array<{ name: string; isDirectory(): boolean; isFile(): boolean }>;
}>("node:fs");
const { join } = await vi.importActual<{ join(...paths: string[]): string }>("node:path");

interface CallSite {
  command: string;
  keys: string[];
  file: string;
}

interface RustCommand {
  args: string[];
  argCase: "snake_case" | "camelCase";
}

/** PascalCase 变体名 → Tauri 命令名（与 client.ts 的 commandName 算法一致） */
function commandName(variant: string): string {
  return variant.replace(/([A-Z])/g, "_$1").toLowerCase().replace(/^_/, "");
}

/** 依据命令声明的 rename_all 推导 invoke 侧期望键名 */
function expectedKey(name: string, argCase: RustCommand["argCase"]): string {
  return argCase === "camelCase"
    ? name.replace(/_([a-z0-9])/g, (_, c: string) => c.toUpperCase())
    : name;
}

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

/** 去掉行注释与 doc 注释（行首 // 或 * 或 /*），避免注释里的示例被误认 */
function stripCommentLines(source: string): string {
  return source
    .split("\n")
    .filter((line) => {
      const t = line.trimStart();
      return !t.startsWith("//") && !t.startsWith("*") && !t.startsWith("/*");
    })
    .join("\n");
}

/** 按括号深度切分顶层逗号（值里的函数调用/嵌套括号不切分） */
function splitTopLevel(text: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let current = "";
  for (const ch of text) {
    if ("([{".includes(ch)) depth++;
    else if (")]}".includes(ch)) depth--;
    if (ch === "," && depth === 0) {
      parts.push(current);
      current = "";
    } else {
      current += ch;
    }
  }
  if (current.trim()) parts.push(current);
  return parts;
}

/** 解析对象字面量文本的顶层键（支持 shorthand 与 key: value） */
function parseObjectKeys(text: string): string[] {
  const keys: string[] = [];
  for (const part of splitTopLevel(text)) {
    const m = part.match(/^\s*(\w+)\s*(?=:|\s*$)/);
    if (m) keys.push(m[1]);
  }
  return keys;
}

/** Rust 形参列表 → 业务参数名（剔除薄壳注入的 state: State<'_, AppState>） */
function rustArgNames(argText: string): string[] {
  const cleaned = argText.replace(/\bstate\s*:\s*State\s*<[^<>]*>\s*,?/g, "");
  const names: string[] = [];
  for (const part of splitTopLevel(cleaned)) {
    const m = part.match(/^\s*(\w+)\s*:/);
    if (m) names.push(m[1]);
  }
  return names;
}

/**
 * 解析 commands.rs 全部命令签名。
 * - 手写：#[tauri::command(...)] + pub async fn name(args)；
 * - cmd! 宏展开：cmd!(name(args) -> ...)，模板自动追加 state 形参。
 */
function parseRustCommands(source: string): {
  commands: Map<string, RustCommand>;
  duplicates: string[];
} {
  const code = stripCommentLines(source);
  const commands = new Map<string, RustCommand>();
  const duplicates: string[] = [];
  const fnRe = /#\s*\[tauri::command(\([^)]*\))?\]\s*pub async fn (\w+)\s*\(([\s\S]*?)\) ->/g;
  for (const m of code.matchAll(fnRe)) {
    const argCase =
      m[1]?.includes('rename_all = "snake_case"') === true ? "snake_case" : "camelCase";
    if (commands.has(m[2])) duplicates.push(m[2]);
    commands.set(m[2], { args: rustArgNames(m[3]), argCase });
  }
  const macroRe = /cmd!\(\s*(\w+)\s*\(([^)]*)\)/g;
  for (const m of code.matchAll(macroRe)) {
    if (commands.has(m[1])) duplicates.push(m[1]);
    commands.set(m[1], { args: rustArgNames(m[2]), argCase: "snake_case" });
  }
  return { commands, duplicates };
}

/** cmd! 宏模板本体是否声明 rename_all = "snake_case" */
function macroTemplateDeclaresSnakeCase(source: string): boolean {
  const m = source.match(/macro_rules!\s*cmd\s*\{/);
  if (!m || m.index === undefined) return false;
  const bodyStart = m.index + m[0].length - 1;
  const bodyEnd = matchBrace(source, bodyStart);
  if (bodyEnd < 0) return false;
  return source.slice(bodyStart, bodyEnd).includes('rename_all = "snake_case"');
}

/** client.ts 全部 call() helper：{ Variant: {...} | null } → (命令名, 键集) */
function clientCallSites(source: string, file: string): CallSite[] {
  const sites: CallSite[] = [];
  const re = /\bcall(?:<[^<>]*>)?\s*\(\s*\{/g;
  for (const m of source.matchAll(re)) {
    if (m.index === undefined) continue;
    const outerStart = m.index + m[0].length - 1;
    const outerEnd = matchBrace(source, outerStart);
    if (outerEnd < 0) continue;
    const outer = source.slice(outerStart + 1, outerEnd);
    const vm = outer.match(/^\s*([A-Z]\w*)\s*:\s*([\s\S]*)$/);
    if (!vm) throw new Error(`${file}: 无法解析 call() 负载形状: ${outer.trim()}`);
    const value = vm[2].trim();
    let keys: string[] = [];
    if (value !== "null") {
      if (!value.startsWith("{")) {
        throw new Error(`${file}: ${vm[1]} 负载既非 null 也非对象字面量: ${value}`);
      }
      const innerEnd = matchBrace(value, 0);
      if (innerEnd < 0) throw new Error(`${file}: ${vm[1]} 负载括号不配对`);
      keys = parseObjectKeys(value.slice(1, innerEnd));
    }
    sites.push({ command: commandName(vm[1]), keys, file });
  }
  return sites;
}

/** src/ 下全部直接 invoke("命令", {...}) 调用点 */
function directInvokeSites(source: string, file: string): CallSite[] {
  const sites: CallSite[] = [];
  const re = /\binvoke(?:<[^<>]*>)?\s*\(\s*["'`](\w+)["'`]\s*,\s*\{/g;
  for (const m of source.matchAll(re)) {
    if (m.index === undefined) continue;
    const start = m.index + m[0].length - 1;
    const end = matchBrace(source, start);
    if (end < 0) continue;
    sites.push({ command: m[1], keys: parseObjectKeys(source.slice(start + 1, end)), file });
  }
  return sites;
}

function walkSources(dir: string): string[] {
  const files: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) files.push(...walkSources(full));
    else if (/\.(ts|vue)$/.test(entry.name)) files.push(full);
  }
  return files;
}

const rustSource = readFileSync("src-tauri/src/commands.rs", "utf8");
const libSource = readFileSync("src-tauri/src/lib.rs", "utf8");
const clientSource = readFileSync("src/ipc/client.ts", "utf8");

const { commands: rustCommands, duplicates } = parseRustCommands(rustSource);
const macroTemplateSnakeCase = macroTemplateDeclaresSnakeCase(rustSource);

const callSites: CallSite[] = [
  ...clientCallSites(stripCommentLines(clientSource), "src/ipc/client.ts"),
];
for (const file of walkSources("src")) {
  callSites.push(...directInvokeSites(stripCommentLines(readFileSync(file, "utf8")), file));
}

// lib.rs generate_handler 注册表（commands::name 仅出现在注册列表中）
const registered = new Set([...libSource.matchAll(/commands::(\w+)/g)].map((m) => m[1]));

// 无前端调用点的后端命令白名单
const BACKEND_ONLY = new Set(["push_one_mb"]); // 切片 0 吞吐基线工具命令，无前端入口

describe("IPC 参数键契约（PROB-01）", () => {
  it("commands.rs 全部命令属性（含 cmd! 宏模板）声明 rename_all = snake_case", () => {
    expect(duplicates).toEqual([]);
    const offenders = [...rustCommands.entries()]
      .filter(([, cmd]) => cmd.argCase !== "snake_case")
      .map(([name]) => name);
    expect(offenders).toEqual([]);
    expect(macroTemplateSnakeCase).toBe(true);
  });

  it("前端每个调用点的参数键与后端命令签名一致", () => {
    const problems: string[] = [];
    for (const site of callSites) {
      const rust = rustCommands.get(site.command);
      if (!rust) {
        problems.push(`${site.file}: 命令 ${site.command} 未在后端 commands.rs 声明`);
        continue;
      }
      const expected = rust.args.map((a) => expectedKey(a, rust.argCase)).sort();
      const actual = [...site.keys].sort();
      if (JSON.stringify(actual) !== JSON.stringify(expected)) {
        problems.push(
          `${site.file}: ${site.command} 参数键不一致 前端=${JSON.stringify(actual)} 后端期望=${JSON.stringify(expected)}`,
        );
      }
    }
    expect(problems).toEqual([]);
  });

  it("带参后端命令都有前端调用点（白名单除外）", () => {
    const siteCommands = new Set(callSites.map((s) => s.command));
    const orphans = [...rustCommands.entries()]
      .filter(
        ([name, cmd]) => cmd.args.length > 0 && !siteCommands.has(name) && !BACKEND_ONLY.has(name),
      )
      .map(([name]) => name);
    expect(orphans).toEqual([]);
  });

  it("lib.rs generate_handler 注册命令与 commands.rs 签名一一对应", () => {
    const unregistered = [...rustCommands.keys()].filter((name) => !registered.has(name));
    const unknown = [...registered].filter((name) => !rustCommands.has(name));
    expect({ unregistered, unknown }).toEqual({ unregistered: [], unknown: [] });
  });
});
