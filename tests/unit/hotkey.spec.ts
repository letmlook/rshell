import { describe, expect, it, vi } from "vitest";
import {
  formatHotkey,
  hotkeyBlockedByTarget,
  isTextInputTarget,
  isTerminalHelperTextarea,
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

describe("hotkeyBlockedByTarget（热键该不该被文本输入挡住）", () => {
  // 真实事故：用户绑定的快速命令热键用裸 isTextInputTarget 判据，而 xterm 的
  // helper textarea 本身就是 <textarea> → 终端里按热键永远不触发，
  // 而热键唯一的用途就是在终端里发命令。应用快捷键那条路用的是排除 helper
  // 的判据，两条路径不一致。这里把「终端放行、普通输入框挡住」钉死。
  it("终端的 helper textarea 不挡热键（那里正是热键该生效的地方）", () => {
    const helper = document.createElement("textarea");
    helper.classList.add("xterm-helper-textarea");
    expect(isTextInputTarget(helper)).toBe(true);
    expect(isTerminalHelperTextarea(helper)).toBe(true);
    expect(hotkeyBlockedByTarget(helper)).toBe(false);
  });

  it("普通输入框挡热键（里面敲 Ctrl+Shift+1 只是文本）", () => {
    const input = document.createElement("input");
    const textarea = document.createElement("textarea");
    const editable = document.createElement("div");
    editable.setAttribute("contenteditable", "true");
    expect(hotkeyBlockedByTarget(input)).toBe(true);
    expect(hotkeyBlockedByTarget(textarea)).toBe(true);
    expect(hotkeyBlockedByTarget(editable)).toBe(true);
  });

  it("终端之外的普通元素不挡热键", () => {
    expect(hotkeyBlockedByTarget(document.createElement("div"))).toBe(false);
    expect(hotkeyBlockedByTarget(null)).toBe(false);
  });
});

// 与 ipcContract.spec.ts / elementPlusContract.spec.ts 同一写法：用
// vi.importActual 拿 node 模块（不引入 @types/node）。
const { readFileSync } = await vi.importActual<{
  readFileSync(path: string, encoding: "utf8"): string;
}>("node:fs");

describe("App.vue 的热键路径判据（源码契约）", () => {
  // 单元测试只测得出 util 的语义，测不出「App.vue 调的是哪个函数」——
  // 而真实事故正是在调用点：用户热键用了裸 isTextInputTarget（把终端的
  // helper textarea 也挡住了），应用快捷键用的是排除 helper 的判据。
  // 这里把调用点钉死：两条路径必须用同一套判据。
  const appSource = readFileSync("src/App.vue", "utf8");

  it("用户绑定的快速命令热键用 hotkeyBlockedByTarget（终端里放行）", () => {
    const body = appSource.slice(
      appSource.indexOf("async function triggerHotkeyCommand"),
      appSource.indexOf("async function triggerHotkeyCommand") + 900,
    );
    expect(body, "triggerHotkeyCommand 必须用 hotkeyBlockedByTarget").toContain(
      "hotkeyBlockedByTarget(event.target)",
    );
    expect(body, "不得再用会连终端一起挡掉的裸 isTextInputTarget").not.toContain(
      "isTextInputTarget(event.target)",
    );
  });

  it("应用快捷键与用户热键共用同一套判据", () => {
    expect(appSource).toContain('from "./utils/shortcuts"');
    // matchAppShortcut 内部用的 isTypingInTextField 与 hotkeyBlockedByTarget 同源
    expect(readFileSync("src/utils/shortcuts.ts", "utf8")).toContain(
      "hotkeyBlockedByTarget(target)",
    );
  });
});
