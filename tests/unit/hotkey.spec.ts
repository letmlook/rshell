import { describe, expect, it } from "vitest";
import {
  formatHotkey,
  isTextInputTarget,
  matchesHotkey,
  parseHotkey,
} from "../../src/utils/hotkey";

function keyEvent(init: Partial<KeyboardEvent> & { key: string }): KeyboardEvent {
  return {
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    ...init,
  } as KeyboardEvent;
}

describe("parseHotkey", () => {
  it("解析修饰键与主键", () => {
    const hotkey = parseHotkey("Ctrl+Shift+1");
    expect(hotkey).toEqual({ ctrl: true, alt: false, shift: true, meta: false, key: "1" });
  });

  it("修饰键大小写不敏感、顺序不限", () => {
    expect(parseHotkey("shift+ctrl+K")).toEqual(parseHotkey("Ctrl+Shift+k"));
  });

  it("支持别名（Control / Option / Cmd）", () => {
    expect(parseHotkey("Control+A")?.ctrl).toBe(true);
    expect(parseHotkey("Option+A")?.alt).toBe(true);
    expect(parseHotkey("Cmd+A")?.meta).toBe(true);
  });

  it("支持功能键", () => {
    expect(parseHotkey("Alt+F5")?.key).toBe("f5");
    expect(parseHotkey("Ctrl+PageUp")?.key).toBe("pageup");
  });

  it("空值与非法写法返回 null", () => {
    expect(parseHotkey("")).toBeNull();
    expect(parseHotkey(null)).toBeNull();
    expect(parseHotkey(undefined)).toBeNull();
    expect(parseHotkey("Ctrl+")).toBeNull();
    expect(parseHotkey("Ctrl")).toBeNull();
    // 两个主键：无法判定用哪个
    expect(parseHotkey("Ctrl+A+B")).toBeNull();
    // 多字符且不是已知功能键
    expect(parseHotkey("Ctrl+abc")).toBeNull();
  });
});

describe("matchesHotkey", () => {
  const hotkey = parseHotkey("Ctrl+Shift+1")!;

  it("修饰键与主键全中才算命中", () => {
    expect(matchesHotkey(keyEvent({ key: "1", ctrlKey: true, shiftKey: true }), hotkey)).toBe(true);
  });

  it("少一个修饰键就不命中", () => {
    expect(matchesHotkey(keyEvent({ key: "1", ctrlKey: true }), hotkey)).toBe(false);
    expect(matchesHotkey(keyEvent({ key: "1", shiftKey: true }), hotkey)).toBe(false);
  });

  it("多一个修饰键也不命中", () => {
    expect(
      matchesHotkey(keyEvent({ key: "1", ctrlKey: true, shiftKey: true, altKey: true }), hotkey),
    ).toBe(false);
  });

  it("主键不中就不命中", () => {
    expect(matchesHotkey(keyEvent({ key: "2", ctrlKey: true, shiftKey: true }), hotkey)).toBe(false);
  });

  it("主键大小写不敏感", () => {
    const letter = parseHotkey("Ctrl+Shift+K")!;
    expect(matchesHotkey(keyEvent({ key: "k", ctrlKey: true, shiftKey: true }), letter)).toBe(true);
  });
});

describe("formatHotkey", () => {
  it("回显成可读形式", () => {
    expect(formatHotkey(parseHotkey("ctrl+shift+1")!)).toBe("Ctrl+Shift+1");
    expect(formatHotkey(parseHotkey("alt+f5")!)).toBe("Alt+F5");
  });
});

describe("isTextInputTarget", () => {
  it("输入框、文本域与可编辑元素算文本输入", () => {
    const input = document.createElement("input");
    const textarea = document.createElement("textarea");
    const editable = document.createElement("div");
    editable.setAttribute("contenteditable", "true");
    const div = document.createElement("div");
    expect(isTextInputTarget(input)).toBe(true);
    expect(isTextInputTarget(textarea)).toBe(true);
    expect(isTextInputTarget(editable)).toBe(true);
    expect(isTextInputTarget(div)).toBe(false);
    expect(isTextInputTarget(null)).toBe(false);
  });
});
