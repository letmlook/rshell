// 终端查找栏回归测试
//
// **背景（真机点击挖出来的缺陷）**：在真实 macOS 应用里打开查找栏、输入
// `RSHELL_GUI`（终端里明明有 `RSHELL_GUI_TELNET_OK_42`），匹配计数一直显示
// 「无匹配」——但目标文本确实被选中并高亮了。
//
// **根因**：`@xterm/addon-search` 的 `onDidChangeResults` **只在传了
// `decorations` 选项时才会触发**（addon 的 `_fireResults` 第一行就是
// `if (options?.decorations)`）。应用此前调用
// `search.findNext(term, { caseSensitive })` 没传 `decorations`，于是计数事件
// 永不触发，`searchCount` 恒为 0 → 界面永远显示「无匹配」。
//
// 这里的 SearchAddon 替身**照抄真实语义**：只有 `options.decorations` 存在时才
// 触发计数事件。撤掉修复后本用例必须失败（计数停在「无匹配」）。

import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import ElementPlus from "element-plus";
import TerminalPane from "../../src/components/TerminalPane.vue";

const terminalRuntime = vi.hoisted(() => ({
  input: (_data: string) => {},
  key: (_event: KeyboardEvent): boolean => true,
  options: { disableStdin: false },
  /** 记录 findNext/findPrevious 收到的选项，供断言 */
  searchCalls: [] as { method: string; term: string; options: unknown }[],
  /** Terminal 的构造选项（allowProposedApi 等） */
  constructorOptions: {} as Record<string, unknown>,
}));

const { sendInputMock, resizeTerminalMock, invokeMock } = vi.hoisted(() => ({
  sendInputMock: vi.fn().mockResolvedValue(undefined),
  resizeTerminalMock: vi.fn().mockResolvedValue(undefined),
  invokeMock: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    selection = "";
    options: { theme: unknown; disableStdin: boolean } = {
      theme: { background: "#1e1e2e" },
      disableStdin: false,
    };
    constructor(options?: Record<string, unknown>) {
      terminalRuntime.options = this.options;
      terminalRuntime.constructorOptions = options ?? {};
    }
    loadAddon() {}
    open() {}
    onData(callback: (data: string) => void) {
      terminalRuntime.input = callback;
      terminalRuntime.options = this.options;
    }
    dispose() {}
    write() {}
    attachCustomKeyEventHandler(callback: (event: KeyboardEvent) => boolean) {
      terminalRuntime.key = callback;
    }
    onSelectionChange() {}
    getSelection() {
      return this.selection;
    }
    clear() {
      this.selection = "";
    }
    paste(text: string) {
      terminalRuntime.input(text);
    }
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));
vi.mock("@xterm/addon-search", () => ({
  SearchAddon: class {
    private listeners: ((e: { resultIndex: number; resultCount: number }) => void)[] = [];
    clearDecorations() {}
    findNext(term: string, options?: { decorations?: unknown }) {
      terminalRuntime.searchCalls.push({ method: "next", term, options });
      // 真实 addon：没有 decorations 就不触发计数事件
      if (options?.decorations) {
        for (const l of this.listeners) l({ resultIndex: 0, resultCount: 2 });
      }
      return true;
    }
    findPrevious(term: string, options?: { decorations?: unknown }) {
      terminalRuntime.searchCalls.push({ method: "prev", term, options });
      if (options?.decorations) {
        for (const l of this.listeners) l({ resultIndex: 1, resultCount: 2 });
      }
      return true;
    }
    onDidChangeResults(cb: (e: { resultIndex: number; resultCount: number }) => void) {
      this.listeners.push(cb);
      return { dispose() {} };
    }
  },
}));
vi.mock("@xterm/addon-webgl", () => ({ WebglAddon: class { onContextLoss() {} } }));
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage?: (d: number[]) => void;
  },
  invoke: invokeMock,
}));
vi.mock("../../src/ipc/client", () => ({
  openTerminal: vi.fn().mockResolvedValue(undefined),
  closeTerminal: vi.fn().mockResolvedValue(undefined),
  sendInput: sendInputMock,
  resizeTerminal: resizeTerminalMock,
  ipcErrorMessage: (e: unknown) => String(e),
  listThemes: vi.fn(),
  setAppTheme: vi.fn(),
  setTerminalColorScheme: vi.fn(),
}));
vi.mock("../../src/ipc/events", () => ({ subscribeAppEvents: vi.fn() }));

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  setActivePinia(createPinia());
  terminalRuntime.searchCalls = [];
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

/** 打开查找栏（走 window 级快捷键路由，与真实 Ctrl+F 同一条路） */
async function openSearchBar(wrapper: ReturnType<typeof mount>) {
  window.dispatchEvent(
    new CustomEvent("rshell:terminal-action", { detail: { sessionId: "session-1", action: "find" } }),
  );
  await flushPromises();
  expect(wrapper.find('[data-test="term-search-count"]').exists()).toBe(true);
}

describe("终端查找栏的匹配计数", () => {
  it("查找必须传 decorations，否则计数事件不触发、界面永远显示「无匹配」", async () => {
    const wrapper = mount(TerminalPane, {
      props: { sessionId: "session-1", connectionState: "connected" },
      global: { plugins: [ElementPlus] },
    });
    await flushPromises();
    await openSearchBar(wrapper);

    await wrapper.find(".search-bar input").setValue("RSHELL_GUI");
    await flushPromises();

    const calls = terminalRuntime.searchCalls;
    expect(calls.length, "输入关键词后必须真的发起一次查找").toBeGreaterThan(0);
    for (const call of calls) {
      expect(
        (call.options as { decorations?: unknown } | undefined)?.decorations,
        "查找必须传 decorations：addon 只在有 decorations 时才触发 onDidChangeResults",
      ).toBeTruthy();
    }

    // 计数必须是真实命中数，而不是「无匹配」
    const count = wrapper.find('[data-test="term-search-count"]').text();
    expect(count, "计数不得停留在「无匹配」").not.toContain("无匹配");
    expect(count).toBe("1/2");
    wrapper.unmount();
  });

  it("「上」按钮同样走带 decorations 的查找", async () => {
    const wrapper = mount(TerminalPane, {
      props: { sessionId: "session-1", connectionState: "connected" },
      global: { plugins: [ElementPlus] },
    });
    await flushPromises();
    await openSearchBar(wrapper);

    await wrapper.find(".search-bar input").setValue("RSHELL_GUI");
    await flushPromises();
    terminalRuntime.searchCalls = [];

    const prev = wrapper.findAll(".search-bar button").find((b) => b.text() === "上");
    expect(prev, "查找栏必须有「上」按钮").toBeTruthy();
    await prev!.trigger("click");
    await flushPromises();

    expect(terminalRuntime.searchCalls.map((c) => c.method)).toContain("prev");
    expect(
      (terminalRuntime.searchCalls.at(-1)?.options as { decorations?: unknown } | undefined)?.decorations,
      "「上」也必须传 decorations",
    ).toBeTruthy();
    wrapper.unmount();
  });

  it("清空关键词后计数归零，不再显示旧命中数", async () => {
    const wrapper = mount(TerminalPane, {
      props: { sessionId: "session-1", connectionState: "connected" },
      global: { plugins: [ElementPlus] },
    });
    await flushPromises();
    await openSearchBar(wrapper);

    await wrapper.find(".search-bar input").setValue("RSHELL_GUI");
    await flushPromises();
    expect(wrapper.find('[data-test="term-search-count"]').text()).toBe("1/2");

    await wrapper.find(".search-bar input").setValue("");
    await flushPromises();
    expect(wrapper.find('[data-test="term-search-count"]').text()).toBe("—");
    wrapper.unmount();
  });
});

describe("断线提示条文案", () => {
  it("不得拼出「会话已已断开」这种叠字", async () => {
    const wrapper = mount(TerminalPane, {
      props: { sessionId: "session-1", connectionState: "disconnected" },
      global: { plugins: [ElementPlus] },
    });
    await flushPromises();
    const text = wrapper.text();
    expect(text, "断线提示不得出现叠字「已已」").not.toContain("已已");
    wrapper.unmount();
  });
});

describe("查找栏的焦点", () => {
  it("打开查找栏必须把焦点交给输入框", async () => {
    // 查找栏是盖在终端上的：焦点若留在终端，用户接着打的字会被送进
    // **远端 shell**（等于盲敲远端命令），搜索框却一个字都没有。
    const wrapper = mount(TerminalPane, {
      props: { sessionId: "session-1", connectionState: "connected" },
      global: { plugins: [ElementPlus] },
      attachTo: document.body,
    });
    await flushPromises();

    window.dispatchEvent(
      new CustomEvent("rshell:terminal-action", { detail: { sessionId: "session-1", action: "find" } }),
    );
    await flushPromises();
    await flushPromises();

    const input = wrapper.find(".search-bar input").element as HTMLInputElement;
    expect(input, "查找栏必须有输入框").toBeTruthy();
    expect(
      document.activeElement,
      "打开查找栏后焦点必须落在输入框，否则键盘输入会进远端终端",
    ).toBe(input);
    wrapper.unmount();
  });
});

describe("xterm 的 proposed API 开关", () => {
  it("构造 Terminal 时必须开 allowProposedApi，否则带 decorations 的查找会抛异常", async () => {
    const wrapper = mount(TerminalPane, {
      props: { sessionId: "session-1", connectionState: "connected" },
      global: { plugins: [ElementPlus] },
    });
    await flushPromises();

    expect(
      terminalRuntime.constructorOptions.allowProposedApi,
      "查找高亮要用 SearchAddon 的 decorations（proposed API），终端必须开 allowProposedApi",
    ).toBe(true);
    wrapper.unmount();
  });
});
