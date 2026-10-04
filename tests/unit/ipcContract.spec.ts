// tests/unit/ipcContract.spec.ts - PROB-01 IPC 参数键契约对账 + R2-10/R2-11 三张对账表
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
// R2-10/R2-11 追加三张对账表：
// A. AppEvent 变体集：events.rs ↔ types.ts 逐变体一致（增删任一侧即失败）；
//    且每个事件要么有后端发布点（AppEvent::X）、要么有前端监听（src/ 引用），
//    0 监听事件必须在 EVENT_LISTENER_WHITELIST 白名单（0 发布点死变体无白名单）。
// B. CommandOutcome 变体 ↔ client.ts call<T> 返回标注：每个变体要么在
//    OUTCOME_RETURN_MAP 中登记（helper + 期望标注），要么在 OUTCOME_WHITELIST。
// C. client.ts helper ↔ src/ 实际调用方：零调用的 helper 必须在
//    HELPER_WHITELIST 白名单（删除某 helper 的最后一个调用点即失败）。
//
// 注入错误键名（如把 session_id 改成 sessionId）、去掉后端 rename_all、
// 在 events.rs/types.ts 增删变体、或新增命令漏掉一侧，都会让对应断言立即失败。
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

// ============================================================================
// R2-10/R2-11 三张对账表：AppEvent / CommandOutcome / client.ts helper
// ============================================================================

const eventsRsSource = readFileSync("src-tauri/crates/rshell-api/src/events.rs", "utf8");
const outcomeRsSource = readFileSync("src-tauri/crates/rshell-api/src/outcome.rs", "utf8");
const typesTsSource = readFileSync("src/ipc/types.ts", "utf8");

/** 解析 Rust 枚举顶层变体名（变体位于 4 空格缩进；字段/属性在更深缩进） */
function parseRustEnumVariants(enumSource: string, enumName: string): string[] {
  const code = stripCommentLines(enumSource);
  const declIndex = code.indexOf(`pub enum ${enumName}`);
  if (declIndex < 0) throw new Error(`enum ${enumName} not found`);
  const bodyStart = code.indexOf("{", declIndex);
  const bodyEnd = matchBrace(code, bodyStart);
  if (bodyEnd < 0) throw new Error(`enum ${enumName} 括号不配对`);
  const names: string[] = [];
  for (const line of code.slice(bodyStart + 1, bodyEnd).split("\n")) {
    const m = line.match(/^ {4}([A-Z][A-Za-z0-9]*)\s*(?:,|\{|\(|$)/);
    if (m) names.push(m[1]);
  }
  return names;
}

/** 解析 types.ts 的 AppEvent union 变体名（字符串变体 + 单键对象变体） */
function parseTsAppEventVariants(typesSource: string): string[] {
  const start = typesSource.indexOf("export type AppEvent");
  if (start < 0) throw new Error("AppEvent not found in types.ts");
  // 类型别名终止于顶层（括号深度 0）的第一个 ";" —— 多行 payload 内部的
  // "};"/";" 不能提前截断
  let depth = 0;
  let end = -1;
  for (let i = start; i < typesSource.length; i++) {
    const ch = typesSource[i];
    if (ch === "{" || ch === "(") depth++;
    else if (ch === "}" || ch === ")") depth--;
    else if (ch === ";" && depth === 0) {
      end = i;
      break;
    }
  }
  if (end < 0) throw new Error("AppEvent union 未闭合");
  const code = stripCommentLines(typesSource.slice(start, end)).replace(/\n/g, " ");
  const names = new Set<string>();
  for (const m of code.matchAll(/\|\s*"(\w+)"/g)) names.add(m[1]);
  for (const m of code.matchAll(/\|\s*\{\s*(\w+):/g)) names.add(m[1]);
  return [...names];
}

/** 递归收集目录下（跳过 target/）全部 .rs 文件源码 */
function walkRustSources(dir: string): string[] {
  const files: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === "target") continue;
    const full = join(dir, entry.name);
    if (entry.isDirectory()) files.push(...walkRustSources(full));
    else if (entry.name.endsWith(".rs")) files.push(readFileSync(full, "utf8"));
  }
  return files;
}

const rustAppEventVariants = parseRustEnumVariants(eventsRsSource, "AppEvent");
const tsAppEventVariants = parseTsAppEventVariants(typesTsSource);
const rustOutcomeVariants = parseRustEnumVariants(outcomeRsSource, "CommandOutcome");

/** 后端全部发布点（所有 .rs 源码中出现 AppEvent::<Variant> 的次数） */
const publishCounts = new Map<string, number>(
  rustAppEventVariants.map((name) => {
    const re = new RegExp(`\\bAppEvent::${name}\\b`, "g");
    let count = 0;
    for (const source of walkRustSources("src-tauri")) {
      count += (source.match(re) ?? []).length;
    }
    return [name, count];
  }),
);

/** 前端监听计数：src/（除 types.ts 镜像）中出现该变体名的文件数 */
const listenerFileCounts = new Map<string, number>(
  rustAppEventVariants.map((name) => {
    const re = new RegExp(`\\b${name}\\b`);
    let count = 0;
    for (const file of walkSources("src")) {
      if (file.endsWith("src/ipc/types.ts")) continue;
      if (re.test(readFileSync(file, "utf8"))) count++;
    }
    return [name, count];
  }),
);

/**
 * 0 监听事件白名单：后端已发布、当前无 UI 需求响应的反馈/信息性事件。
 * 给事件接线后必须从此处移除；新增 0 监听事件必须登记并说明理由。
 */
const EVENT_LISTENER_WHITELIST = new Set([
  "SessionUpdated", // 单会话元数据更新提示，前端经 SessionListChanged 全量刷新已覆盖
  "ScriptFinished", // 脚本结果随命令返回值展示，无需事件分支
  "SyncInputSessionsChanged", // 同步输入集合变化，UI 尚未消费（helper toggleSyncInput 同为未接线）
  "SshKeyListChanged", // 密钥面板刷新由自身操作触发，无跨面板消费需求
  "SshKeyGenerated", // 同上
  "PublicKeyExported", // exportPublicKey helper 未接线（同 helper 白名单）
  "MasterPasswordChanged", // 主密码状态由启动校验命令返回值决定
  "MasterPasswordVerified", // 同上
  "ColorSchemeListChanged", // 配色方案列表由 listThemes 返回值驱动
  "PluginListUpdated", // 插件面板刷新由自身操作触发
  "PluginStateChanged", // 同上
  "PluginLoadFailed", // 加载失败经命令错误路径展示
]);

/** CommandOutcome 变体 → client.ts 返回标注（helper + 期望的 call<T> 文本） */
const OUTCOME_RETURN_MAP: Record<string, { helper: string; typeText: string }> = {
  Sessions: { helper: "listSessions", typeText: "SessionConfig[]" },
  SessionLoadIssues: { helper: "listSessionLoadIssues", typeText: "SessionLoadIssue[]" },
  SessionId: { helper: "createSession", typeText: "Uuid" },
  Triggers: { helper: "listTriggers", typeText: "Trigger[]" },
  QuickCommands: { helper: "listQuickCommands", typeText: "QuickCommand[]" },
  Keys: { helper: "listKeys", typeText: "unknown[]" },
  Tunnels: { helper: "listTunnels", typeText: "ActiveTunnelInfo[]" },
  Plugins: { helper: "listPlugins", typeText: "PluginInfo[]" },
  Themes: { helper: "listThemes", typeText: "ThemeInfo" },
  PendingTunnels: { helper: "listPendingTunnels", typeText: "PendingTunnelInfo" },
  RemoteDir: {
    helper: "browseRemoteDir",
    typeText: "{ path: string; entries: RemoteFileEntry[] }",
  },
  Transfers: { helper: "listTransfers", typeText: "TransferTaskInfo[]" },
  Verified: { helper: "verifyMasterPassword", typeText: "boolean" },
};

/** 无 call<T> 返回标注的 CommandOutcome 变体白名单 */
const OUTCOME_WHITELIST = new Set([
  "None", // 写命令统一返回 None；前端 call() 不带 <T>
  "PublicKey", // export_public_key 后端存在，前端 helper 未标注返回且无调用方（R2-10 同源死面）
]);

/**
 * client.ts 零调用 helper 白名单：后端命令可用但 src/ 暂无调用方的薄壳。
 * 任一 helper 接线后必须从此处移除。
 */
const HELPER_WHITELIST = new Set([
  "restoreTunnel", // 隧道磁盘恢复尚未接入 UI
  "suspendTunnel", // 隧道挂起/恢复尚未接入 UI
  "resumeTunnel",
  "sendComposeText", // 撰写窗格尚未接入后端命令
  "executeScript", // Rhai 宿主执行尚未接入 UI（docs/08：没有完整脚本编辑器）
  "toggleSyncInput", // 同步输入开关尚未接后端
  "exportPublicKey", // 后端命令存在，无 UI 入口（R2-10 CommandOutcome::PublicKey 同源）
  "setupMasterPassword", // 主密码设置流程尚未接入 UI
  "verifyMasterPassword", // 主密码验证流程尚未接入 UI（后端启动校验在用）
  "changeMasterPassword", // 主密码修改流程尚未接入 UI
  "trustHostKey", // 永久信任由后端 decide_host_key 持久化路径完成
  "deleteHostKey", // 已保存主机密钥的删除入口尚未接入 UI
  "importColorScheme", // 配色方案导入尚未接入 UI
]);

describe("IPC 事件契约（R2-10/R2-11）", () => {
  it("events.rs 与 types.ts 的 AppEvent 变体集逐一对齐", () => {
    const rustOnly = rustAppEventVariants.filter((n) => !tsAppEventVariants.includes(n));
    const tsOnly = tsAppEventVariants.filter((n) => !rustAppEventVariants.includes(n));
    expect({ rustOnly, tsOnly }).toEqual({ rustOnly: [], tsOnly: [] });
  });

  it("每个事件都有后端发布点（0 发布点死变体不允许）", () => {
    const dead = rustAppEventVariants.filter((n) => (publishCounts.get(n) ?? 0) === 0);
    expect(dead).toEqual([]);
  });

  it("每个事件都有前端监听（0 监听必须白名单）", () => {
    const unlistened = rustAppEventVariants.filter(
      (n) => (listenerFileCounts.get(n) ?? 0) === 0 && !EVENT_LISTENER_WHITELIST.has(n),
    );
    expect(unlistened).toEqual([]);
  });

  it("事件监听白名单只包含真实存在的事件（防止白名单腐化）", () => {
    const unknown = [...EVENT_LISTENER_WHITELIST].filter(
      (n) => !rustAppEventVariants.includes(n),
    );
    expect(unknown).toEqual([]);
  });
});

describe("IPC 返回值契约（R2-11）", () => {
  it("CommandOutcome 变体全部登记返回标注或白名单", () => {
    const mapped = new Set([...Object.keys(OUTCOME_RETURN_MAP), ...OUTCOME_WHITELIST]);
    const unregistered = rustOutcomeVariants.filter((n) => !mapped.has(n));
    const phantom = [...mapped].filter((n) => !rustOutcomeVariants.includes(n));
    expect({ unregistered, phantom }).toEqual({ unregistered: [], phantom: [] });
  });

  it("已登记变体的 client.ts helper 返回标注与契约一致", () => {
    const problems: string[] = [];
    for (const [variant, { helper, typeText }] of Object.entries(OUTCOME_RETURN_MAP)) {
      if (!clientSource.includes(`call<${typeText}>({`)) {
        problems.push(`${variant}: client.ts 缺少期望标注 call<${typeText}>({（helper: ${helper}）`);
      }
    }
    expect(problems).toEqual([]);
  });
});

describe("client.ts helper 调用方契约（R2-11）", () => {
  const helperNames = [...clientSource.matchAll(/^export const (\w+)/gm)].map((m) => m[1]);

  it("client.ts 确实导出了 IPC helper（解析器自检，防止正则失效空转）", () => {
    expect(helperNames.length).toBeGreaterThan(30);
    expect(helperNames).toContain("listSessions");
    expect(helperNames).toContain("exportPublicKey");
  });

  it("每个 helper 在 src/ 都有调用方（零调用必须白名单）", () => {
    const problems: string[] = [];
    for (const name of helperNames) {
      const re = new RegExp(`\\b${name}\\b`);
      let callers = 0;
      for (const file of walkSources("src")) {
        if (file.endsWith("src/ipc/client.ts")) continue;
        if (re.test(readFileSync(file, "utf8"))) {
          callers++;
          break;
        }
      }
      if (callers === 0 && !HELPER_WHITELIST.has(name)) {
        problems.push(`${name}: src/ 无调用方且不在 HELPER_WHITELIST`);
      }
    }
    expect(problems).toEqual([]);
  });

  it("helper 白名单只包含真实存在的 helper（防止白名单腐化）", () => {
    const unknown = [...HELPER_WHITELIST].filter((n) => !helperNames.includes(n));
    expect(unknown).toEqual([]);
  });
});
