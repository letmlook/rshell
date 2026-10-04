import { describe, expect, it } from "vitest";
import {
  COPY_ON_SELECT_KEY,
  loadCopyOnSelect,
  normalizeCopyOnSelect,
  saveCopyOnSelect,
} from "../../src/utils/terminalPrefs";

/** 内存版 Storage：模拟隐私模式/配额异常之外的正常路径 */
function memoryStorage(initial: Record<string, string> = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => {
      map.set(key, value);
    },
    dump: () => Object.fromEntries(map),
  };
}

describe("终端偏好：选中即复制", () => {
  it("无记录时用默认值，读写往返保持布尔语义", () => {
    const storage = memoryStorage();
    expect(loadCopyOnSelect(storage)).toBe(true);

    expect(saveCopyOnSelect(storage, false)).toBe(true);
    expect(storage.dump()[COPY_ON_SELECT_KEY]).toBe("false");
    expect(loadCopyOnSelect(storage)).toBe(false);

    saveCopyOnSelect(storage, true);
    expect(loadCopyOnSelect(storage)).toBe(true);
  });

  it("存储不可用时降级为默认值，不抛错打断终端", () => {
    const throwing = {
      getItem: () => {
        throw new Error("denied");
      },
      setItem: () => {
        throw new Error("quota exceeded");
      },
    };
    expect(loadCopyOnSelect(throwing)).toBe(true);
    expect(saveCopyOnSelect(throwing, false)).toBe(false);
    // storage 直接缺失（SSR / 测试环境）同样不炸
    expect(loadCopyOnSelect(null)).toBe(true);
    expect(saveCopyOnSelect(null, true)).toBe(false);
  });

  it("损坏或非布尔的存储值不被当成 true/false 使用", () => {
    expect(normalizeCopyOnSelect("1")).toBe(true);
    expect(normalizeCopyOnSelect("0")).toBe(true);
    expect(normalizeCopyOnSelect(undefined, false)).toBe(false);
    expect(loadCopyOnSelect(memoryStorage({ [COPY_ON_SELECT_KEY]: "maybe" }))).toBe(true);
  });
});
