import { describe, expect, it } from "vitest";
import {
  MAX_TERMINAL_TITLE_LENGTH,
  shortSessionId,
  shortTerminalTitle,
} from "../../src/utils/terminalTitle";

const UUID = "1a2b3c4d-5e6f-7a8b-9c0d-1e2f3a4b5c6d";

describe("shortSessionId", () => {
  it("keeps only the leading characters of a uuid", () => {
    expect(shortSessionId(UUID)).toBe("1a2b3c4d");
  });

  it("returns the whole value when it is already short", () => {
    expect(shortSessionId("abc")).toBe("abc");
  });
});

describe("shortTerminalTitle", () => {
  it("prefers the session name over the id", () => {
    expect(shortTerminalTitle("prod-web-01", UUID)).toBe("prod-web-01");
  });

  it("trims surrounding whitespace before using the name", () => {
    expect(shortTerminalTitle("  prod-web-01  ", UUID)).toBe("prod-web-01");
  });

  it("falls back to the short id when the name is missing", () => {
    expect(shortTerminalTitle(undefined, UUID)).toBe("1a2b3c4d");
    expect(shortTerminalTitle(null, UUID)).toBe("1a2b3c4d");
  });

  it("falls back to the short id when the name is only whitespace", () => {
    expect(shortTerminalTitle("   \t  ", UUID)).toBe("1a2b3c4d");
  });

  it("keeps a name that already fits the limit untouched", () => {
    const name = "a".repeat(MAX_TERMINAL_TITLE_LENGTH);
    expect(shortTerminalTitle(name, UUID)).toBe(name);
  });

  it("truncates an over-long name with an ellipsis", () => {
    const title = shortTerminalTitle("b".repeat(60), UUID);
    expect(Array.from(title)).toHaveLength(MAX_TERMINAL_TITLE_LENGTH);
    expect(title.endsWith("…")).toBe(true);
  });

  // 回归：dockview 在 addPanel 未传 title 时用 options.id 当标签，
  // 面板 id 为 `terminal-<uuid>`（45 字符），会把标签栏挤满。
  it("never reproduces the long `terminal-<uuid>` panel id", () => {
    const title = shortTerminalTitle(undefined, UUID);
    expect(title).not.toContain(UUID);
    expect(title.length).toBeLessThan(MAX_TERMINAL_TITLE_LENGTH);
  });

  it("does not split a surrogate pair when truncating", () => {
    // 每个 emoji 占 2 个 UTF-16 码元，按码元切会切出半个代理对
    const title = shortTerminalTitle("🚀".repeat(30), UUID);
    expect(title).not.toMatch(/[\uD800-\uDBFF]$/);
    expect(title.endsWith("…")).toBe(true);
  });
});
