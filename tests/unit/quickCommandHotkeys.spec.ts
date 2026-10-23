// 快速命令热键 —— 共享热键表与「终端吞键」的回归
//
// 真实事故（两处）：
// 1. 终端里按热键**永远不触发**：触发侧用裸 `isTextInputTarget` 判据，把 xterm 的
//    helper textarea（本身就是 `<textarea>`）也挡住了 —— 而那恰恰是热键该生效的地方。
// 2. 即便触发了，**按键还会漏进远端 shell**：xterm 的按键处理在目标阶段先跑
//    （早于冒泡到 window），窗口处理器再 preventDefault() 已经太晚，
//    按一次热键远端会收到一串垃圾字符（真机实测屏幕上多出 "hhh"）。
//
// 修法：热键表提到共享模块 `utils/quickCommandHotkeys`，触发侧与终端侧读同一份。
import { describe, expect, it, beforeEach, vi } from "vitest";
import {
  matchesUserHotkey,
  quickCommandHotkeys,
  setQuickCommands,
} from "../../src/utils/quickCommandHotkeys";
import type { QuickCommand } from "../../src/ipc/types";

const { readFileSync } = await vi.importActual<{
  readFileSync(path: string, encoding: "utf8"): string;
}>("node:fs");

function keyEvent(init: Partial<KeyboardEvent> & { key: string }): KeyboardEvent {
  return {
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    ...init,
  } as KeyboardEvent;
}

function command(hotkey: string | null, name = "cmd"): QuickCommand {
  return {
    id: "11111111-1111-4111-8111-111111111111",
    name,
    command: "echo hi",
    send_enter: true,
    description: "",
    scope: "CurrentSession",
    hotkey,
    group: null,
  } as QuickCommand;
}

beforeEach(() => {
  setQuickCommands([]);
});

describe("共享热键表", () => {
  it("只收录写得出、绑了热键的命令", () => {
    setQuickCommands([command("Ctrl+Shift+H", "a"), command(null, "b"), command("这不是热键", "c")]);
    expect(quickCommandHotkeys.value.map((item) => item.command.name)).toEqual(["a"]);
  });

  it("命中判断：修饰键与主键全中才算命中", () => {
    setQuickCommands([command("Ctrl+Shift+H")]);
    expect(matchesUserHotkey(keyEvent({ key: "h", ctrlKey: true, shiftKey: true }))).toBe(true);
    expect(matchesUserHotkey(keyEvent({ key: "h", ctrlKey: true }))).toBe(false);
    expect(matchesUserHotkey(keyEvent({ key: "H", ctrlKey: true, shiftKey: true }))).toBe(true);
  });

  it("没有绑定热键的命令不会命中任何键", () => {
    setQuickCommands([command(null)]);
    expect(matchesUserHotkey(keyEvent({ key: "h", ctrlKey: true, shiftKey: true }))).toBe(false);
  });
});

describe("终端必须吞掉用户热键（源码契约）", () => {
  const paneSource = readFileSync("src/components/TerminalPane.vue", "utf8");

  it("TerminalPane 的按键处理会拦下用户热键，不让它漏进远端 shell", () => {
    // xterm 的按键处理在目标阶段先跑，窗口处理器再 preventDefault() 已经太晚；
    // 不在终端侧吞掉，按一次热键远端会收到一串垃圾字符。
    expect(paneSource).toContain("matchesUserHotkey");
    expect(paneSource).toMatch(/matchesUserHotkey\(event\)\)\s*return false/);
  });

  it("触发侧与终端侧共用同一份热键表", () => {
    expect(readFileSync("src/App.vue", "utf8")).toContain("from \"./utils/quickCommandHotkeys\"");
    expect(paneSource).toContain("from \"../utils/quickCommandHotkeys\"");
  });
});
