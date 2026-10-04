import { describe, expect, it } from "vitest";
import {
  LAST_LOCAL_DIR_KEY,
  loadLastLocalDir,
  normalizeLocalDir,
  saveLastLocalDir,
} from "../../src/utils/lastLocalDir";

/** 最小 Storage 桩：只实现本模块用到的两个方法 */
function fakeStorage(initial: Record<string, string> = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => { map.set(k, v); },
    read: () => Object.fromEntries(map),
  };
}

describe("normalizeLocalDir", () => {
  it("keeps an absolute posix path", () => {
    expect(normalizeLocalDir("/home/test/files")).toBe("/home/test/files");
  });

  it("keeps an absolute windows path and normalizes separators at the tail", () => {
    expect(normalizeLocalDir("C:\\Users\\test\\files")).toBe("C:\\Users\\test\\files");
    expect(normalizeLocalDir("C:/Users/test/files/")).toBe("C:/Users/test/files");
  });

  it("rejects relative paths (meaning depends on the process working directory)", () => {
    expect(normalizeLocalDir("files")).toBeNull();
    expect(normalizeLocalDir("..\\files")).toBeNull();
    expect(normalizeLocalDir("./files")).toBeNull();
  });

  it("rejects empty and whitespace-only values", () => {
    expect(normalizeLocalDir("")).toBeNull();
    expect(normalizeLocalDir("   ")).toBeNull();
    expect(normalizeLocalDir(null)).toBeNull();
    expect(normalizeLocalDir(undefined)).toBeNull();
  });

  // 回归：同一目录若以不同尾部分隔符存成两条，恢复时会多出一次「不存在」错误
  it("collapses trailing separators so one directory is stored once", () => {
    expect(normalizeLocalDir("C:\\Users\\test\\files\\")).toBe("C:\\Users\\test\\files");
    expect(normalizeLocalDir("/home/test/files//")).toBe("/home/test/files");
  });

  it("preserves filesystem roots", () => {
    expect(normalizeLocalDir("/")).toBe("/");
    expect(normalizeLocalDir("C:\\")).toBe("C:\\");
    // 盘符根的两种写法归一到同一值，否则同一目录会被当成两条记录
    expect(normalizeLocalDir("C:/")).toBe("C:\\");
    expect(normalizeLocalDir("C:/")).toBe(normalizeLocalDir("C:\\"));
  });
});

describe("saveLastLocalDir / loadLastLocalDir", () => {
  it("round-trips a directory", () => {
    const storage = fakeStorage();
    expect(saveLastLocalDir(storage, "C:\\Users\\test\\files")).toBe(true);
    expect(loadLastLocalDir(storage)).toBe("C:\\Users\\test\\files");
  });

  it("uses the versioned storage key", () => {
    const storage = fakeStorage();
    saveLastLocalDir(storage, "/home/test");
    expect(storage.read()[LAST_LOCAL_DIR_KEY]).toBe("/home/test");
  });

  it("returns null when nothing was ever stored", () => {
    expect(loadLastLocalDir(fakeStorage())).toBeNull();
  });

  it("returns null for a corrupt or non-absolute stored value", () => {
    expect(loadLastLocalDir(fakeStorage({ [LAST_LOCAL_DIR_KEY]: "relative/path" }))).toBeNull();
    expect(loadLastLocalDir(fakeStorage({ [LAST_LOCAL_DIR_KEY]: "   " }))).toBeNull();
  });

  it("does not write an invalid path", () => {
    const storage = fakeStorage();
    expect(saveLastLocalDir(storage, "relative")).toBe(false);
    expect(saveLastLocalDir(storage, "")).toBe(false);
    expect(storage.read()[LAST_LOCAL_DIR_KEY]).toBeUndefined();
  });

  // 存储不可用（隐私模式/配额满/WebView 禁用）时必须降级为「不记忆」，
  // 绝不能抛错打断传输工作区。
  it("degrades to no-op when the storage is unavailable", () => {
    const throwing = {
      getItem: () => { throw new Error("denied"); },
      setItem: () => { throw new Error("quota exceeded"); },
    };
    expect(loadLastLocalDir(throwing)).toBeNull();
    expect(saveLastLocalDir(throwing, "/home/test")).toBe(false);
    expect(loadLastLocalDir(null)).toBeNull();
    expect(saveLastLocalDir(null, "/home/test")).toBe(false);
  });
});
