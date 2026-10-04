// tests/unit/capabilityScope.spec.ts - src-tauri/capabilities/default.json 契约
//
// 为什么需要这个测试：本地目录浏览依赖 fs scope。「上次打开的目录」从
// localStorage 恢复时没有经过文件选择对话框，不会被 tauri-plugin-dialog 自动
// 加入 scope；进程重启后 scope 又回到空。此前只声明了 `fs:allow-read-dir` /
// `fs:allow-stat` 且没有任何 scope 条目，导致恢复目录被拒
// （`forbidden path: ...`）。现在按产品决定改为全局读写。
//
// 本测试锁住三件事，任一被无声改动即失败：
//   1. 存在覆盖 `**` 的 scope 条目（否则任意目录都读不了）；
//   2. 读/写命令权限仍然保留（收窄会让文件浏览或传输静默失效）；
//   3. WebView 数据目录仍在 deny 列表里（通配 allow 不应冲掉这层保护）。
import { describe, expect, it } from "vitest";
import capability from "../../src-tauri/capabilities/default.json";

const permissions = capability.permissions;

/** 形如 fs:scope 的对象型权限条目 */
const scopeEntries = permissions
  .filter((p) => typeof p !== "string")
  .filter((p) => p.identifier === "fs:scope");

/** 全部权限标识符（字符串权限与对象型 scope 条目统一取值） */
const permissionIds = permissions.map((p) => (typeof p === "string" ? p : p.identifier));

describe("capability fs scope", () => {
  it("declares a fs:scope entry covering the whole filesystem", () => {
    // 必须存在 `**` scope：否则 localStorage 恢复的目录（未经对话框）无法读取
    expect(scopeEntries.length).toBeGreaterThan(0);
    const patterns = scopeEntries.map((e) => e.allow).flat();
    expect(patterns).toContain("**");
  });

  it("keeps read and write command permissions", () => {
    expect(permissionIds).toContain("fs:read-all");
    expect(permissionIds).toContain("fs:write-all");
  });

  // deny 优先于 allow（tauri 的 is_forbidden 先判），所以通配 allow 不会
  // 覆盖这层保护；这层保护挡住 WebView 自身数据目录的读取。
  it("still denies access to the webview data directory", () => {
    expect(permissionIds).toContain("fs:deny-webview-data-windows");
    expect(permissionIds).toContain("fs:deny-webview-data-linux");
  });

  it("keeps every permission id unique", () => {
    const dupes = permissionIds.filter((id, i) => permissionIds.indexOf(id) !== i);
    expect(dupes).toEqual([]);
  });
});
