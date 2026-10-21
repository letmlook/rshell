import { describe, expect, it } from "vitest";
import { matchAppShortcut, isTypingInTextField } from "../../src/utils/shortcuts";

function keyEvent(init: Partial<KeyboardEvent> & { key: string }): KeyboardEvent {
  return {
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    type: "keydown",
    ...init,
  } as KeyboardEvent;
}

/** 终端的隐藏输入框：快捷键在这里要生效 */
function terminalHelper(): HTMLElement {
  const el = document.createElement("textarea");
  el.classList.add("xterm-helper-textarea");
  document.body.appendChild(el);
  return el;
}

/** 普通对话框输入框：应用的终端级快捷键不该抢 */
function plainInput(): HTMLElement {
  const el = document.createElement("input");
  document.body.appendChild(el);
  return el;
}

describe("matchAppShortcut", () => {
  it("识别应用接管的快捷键", () => {
    expect(matchAppShortcut(keyEvent({ key: "n", ctrlKey: true }))).toBe("new-session");
    expect(matchAppShortcut(keyEvent({ key: "N", metaKey: true }))).toBe("new-session");
    expect(matchAppShortcut(keyEvent({ key: "f", ctrlKey: true }))).toBe("find");
    expect(matchAppShortcut(keyEvent({ key: "c", ctrlKey: true, shiftKey: true }))).toBe("copy");
    expect(matchAppShortcut(keyEvent({ key: "v", ctrlKey: true, shiftKey: true }))).toBe("paste");
  });

  it("修饰键必须完全一致", () => {
    // Ctrl+Shift+F 不是查找
    expect(matchAppShortcut(keyEvent({ key: "f", ctrlKey: true, shiftKey: true }))).toBeNull();
    // 光秃秃的 n 不是新建
    expect(matchAppShortcut(keyEvent({ key: "n" }))).toBeNull();
    // Alt 组合不是应用快捷键
    expect(matchAppShortcut(keyEvent({ key: "n", altKey: true }))).toBeNull();
  });

  it("在普通输入框里不抢快捷键（原生复制粘贴更好用）", () => {
    const input = plainInput();
    expect(matchAppShortcut(keyEvent({ key: "c", ctrlKey: true, shiftKey: true, target: input }))).toBeNull();
    expect(matchAppShortcut(keyEvent({ key: "n", ctrlKey: true, target: input }))).toBeNull();
  });

  it("在终端的隐藏输入框里快捷键照常生效", () => {
    const helper = terminalHelper();
    expect(matchAppShortcut(keyEvent({ key: "n", ctrlKey: true, target: helper }))).toBe("new-session");
    expect(matchAppShortcut(keyEvent({ key: "f", ctrlKey: true, target: helper }))).toBe("find");
  });

  it("Escape 不属于应用快捷键：它要照常送到远端 shell", () => {
    expect(matchAppShortcut(keyEvent({ key: "Escape" }))).toBeNull();
  });
});

describe("isTypingInTextField", () => {
  it("区分普通输入框与终端的隐藏输入框", () => {
    const input = plainInput();
    const helper = terminalHelper();
    expect(isTypingInTextField(input)).toBe(true);
    expect(isTypingInTextField(helper)).toBe(false);
    expect(isTypingInTextField(null)).toBe(false);
  });
});
