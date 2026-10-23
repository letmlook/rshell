// xterm SearchAddon 的**真实行为**回归（不用 mock 替身）
//
// 背景：终端查找栏的匹配计数一直显示「无匹配」，哪怕目标文本就在眼前。
// 根因有两层，都必须钉住：
//
// 1. `@xterm/addon-search` 的 `onDidChangeResults` **只在传了 `decorations`
//    选项时才会触发**（addon 内部 `_fireResults` 的第一行就是
//    `if (options?.decorations)`）。不传 → 计数事件永不触发 → 计数恒为 0。
// 2. `decorations` 属于 xterm 的 **proposed API**：终端没有用
//    `allowProposedApi: true` 构造时，带 decorations 的 findNext 会**直接抛
//    异常**（"You must set the allowProposedApi option to true"），
//    查找整个报错。
//
// 所以应用必须同时做到：构造 Terminal 时开 `allowProposedApi`，查找时传
// `decorations`。本文件用**真实的 xterm + 真实的 addon** 把这两条钉死，
// 免得以后升级依赖时悄悄退化。

import { describe, expect, it, vi } from "vitest";
import { Terminal } from "@xterm/xterm";
import { SearchAddon } from "@xterm/addon-search";

/** jsdom 没有 matchMedia，xterm 的 devicePixelRatio 依赖它 */
function stubMatchMedia() {
  window.matchMedia = vi.fn().mockReturnValue({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
  }) as unknown as typeof window.matchMedia;
}

const DECORATIONS = {
  matchBackground: "#5c4b00",
  matchBorder: "#8a6d00",
  matchOverviewRuler: "#8a6d00",
  activeMatchBackground: "#b58900",
  activeMatchBorder: "#ffb300",
  activeMatchColorOverviewRuler: "#ffb300",
};

async function makeTerminal(options: { allowProposedApi?: boolean }) {
  stubMatchMedia();
  const term = new Terminal({ cols: 80, rows: 24, ...options });
  const host = document.createElement("div");
  document.body.appendChild(host);
  term.open(host);
  term.write("echo RSHELL_FIND_TEST\r\nRSHELL_FIND_TEST\r\n");
  // xterm 的写入是异步排队的，等它落到 buffer 里
  await new Promise((resolve) => setTimeout(resolve, 50));
  return term;
}

describe("xterm SearchAddon 的计数事件（真实依赖）", () => {
  it("不传 decorations 时不触发计数事件", async () => {
    const term = await makeTerminal({ allowProposedApi: true });
    const search = new SearchAddon();
    term.loadAddon(search);
    const events: { resultIndex: number; resultCount: number }[] = [];
    search.onDidChangeResults((event) => events.push(event));

    search.findNext("RSHELL", { caseSensitive: false });

    expect(
      events,
      "不传 decorations 就不该有计数事件（这正是计数恒为 0 的根因）",
    ).toHaveLength(0);
    term.dispose();
  });

  it("传 decorations 时触发计数事件，且命中数大于 0", async () => {
    const term = await makeTerminal({ allowProposedApi: true });
    const search = new SearchAddon();
    term.loadAddon(search);
    const events: { resultIndex: number; resultCount: number }[] = [];
    search.onDidChangeResults((event) => events.push(event));

    search.findNext("RSHELL", { caseSensitive: false, decorations: DECORATIONS });

    expect(events, "传了 decorations 就必须有计数事件").toHaveLength(1);
    expect(events[0].resultCount, "缓冲里明明有 2 处 RSHELL").toBe(2);
    term.dispose();
  });

  it("终端没开 allowProposedApi 时，带 decorations 的查找会抛异常", async () => {
    const term = await makeTerminal({ allowProposedApi: false });
    const search = new SearchAddon();
    term.loadAddon(search);

    expect(() =>
      search.findNext("RSHELL", { caseSensitive: false, decorations: DECORATIONS }),
    ).toThrow(/allowProposedApi/);
    term.dispose();
  });
});
