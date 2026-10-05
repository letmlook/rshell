import { describe, expect, it } from "vitest";
import { isWithinRoot, normalizePathSegments } from "../../src/utils/rootBoundary";

// R3-10：路径栏的「已授权根目录」检查曾经是两处各写一份的前缀字符串比较，
// `/home/user/../etc` 能通过，Windows 根 `C:\data` 又被 navigateTo 误挡。
// 本组用例锁住共享判据的语义：词法消解 `.` / `..` 后逐段比较，且两个入口
// （PathBar.commitEdit 与 FileBrowserPane.navigateTo）用的是同一份实现。
describe("normalizePathSegments", () => {
  it("消解 `.` 与 `..`，不访问文件系统", () => {
    expect(normalizePathSegments("/home/user/../etc")).toEqual({
      prefix: "\\",
      segments: ["home", "etc"],
      windowsDrive: false,
    });
    expect(normalizePathSegments("/home/user/./docs")).toEqual({
      prefix: "\\",
      segments: ["home", "user", "docs"],
      windowsDrive: false,
    });
    // 越过根的 `..` 钳制在根：多退几层也不会凭空拼出更浅的合法子路径
    expect(normalizePathSegments("/home/user/../../../etc").segments).toEqual(["etc"]);
  });

  it("统一分隔符与盘符，保留 UNC 的双反斜杠", () => {
    expect(normalizePathSegments("C:/Data/Docs").prefix).toBe("C:\\");
    expect(normalizePathSegments("C:/Data/Docs").segments).toEqual(["Data", "Docs"]);
    expect(normalizePathSegments("c:\\data\\").segments).toEqual(["data"]);
    expect(normalizePathSegments("\\\\server\\share\\dir").prefix).toBe("\\\\");
    expect(normalizePathSegments("\\\\server\\share\\dir").segments).toEqual(["server", "share", "dir"]);
  });

  it("相对路径没有前缀——无法证明落在绝对根下", () => {
    expect(normalizePathSegments("docs/../etc").prefix).toBe("");
    expect(normalizePathSegments("").prefix).toBe("");
  });
});

describe("isWithinRoot", () => {
  it("拒绝用 `..` 穿越出根的路径", () => {
    expect(isWithinRoot("/home/user/../etc", "/home/user")).toBe(false);
    expect(isWithinRoot("/home/user/../../etc", "/home/user")).toBe(false);
    // 退到根再下来，最终仍落在根内：字面路径没有越界，判定与解析后一致
    expect(isWithinRoot("/home/user/../user/docs", "/home/user")).toBe(true);
  });

  it("接受根内的普通与含 `.` 的路径", () => {
    expect(isWithinRoot("/home/user/docs", "/home/user")).toBe(true);
    expect(isWithinRoot("/home/user/./docs", "/home/user")).toBe(true);
    expect(isWithinRoot("/home/user", "/home/user")).toBe(true);
    // 结尾分隔符不算越界
    expect(isWithinRoot("/home/user/docs/", "/home/user")).toBe(true);
  });

  it("拒绝同前缀的兄弟目录（`/home/user2` 不是 `/home/user` 的子目录）", () => {
    expect(isWithinRoot("/home/user2", "/home/user")).toBe(false);
    expect(isWithinRoot("/home/user2/docs", "/home/user")).toBe(false);
    expect(isWithinRoot("C:\\data2\\x", "C:\\data")).toBe(false);
  });

  it("拒绝父目录、更浅的同前缀路径与不同根", () => {
    expect(isWithinRoot("/home", "/home/user")).toBe(false);
    expect(isWithinRoot("/home/userless", "/home/user")).toBe(false);
    expect(isWithinRoot("/home/user", "D:\\data")).toBe(false);
    expect(isWithinRoot("C:\\Windows", "C:\\data")).toBe(false);
    // 相对路径：无从判断，判越界而不是放行
    expect(isWithinRoot("docs", "/home/user")).toBe(false);
    expect(isWithinRoot("/home/user", "")).toBe(false);
  });

  it("Windows 根按盘符 + 逐段比较，大小写不敏感", () => {
    expect(isWithinRoot("C:\\data\\docs", "C:\\data")).toBe(true);
    expect(isWithinRoot("C:\\data/./docs", "C:\\data")).toBe(true);
    expect(isWithinRoot("C:\\data\\..\\data2", "C:\\data")).toBe(false);
    // 盘符不同 = 不同根
    expect(isWithinRoot("D:\\data\\docs", "C:\\data")).toBe(false);
    // Windows 上目录名不区分大小写：`C:\Data` 与 `C:\data` 是同一个目录
    expect(isWithinRoot("C:\\Data\\docs", "C:\\data")).toBe(true);
  });
});
