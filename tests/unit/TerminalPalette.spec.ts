import { afterEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import TerminalPane from "../../src/components/TerminalPane.vue";
import { useThemeStore } from "../../src/stores/theme";

const { terminalOptions, keyHandlers, sendInputMock } = vi.hoisted(() => ({
  terminalOptions: [] as Array<{ theme: unknown }>,
  // 记录 attachCustomKeyEventHandler 收到的回调，供退格键位测试直接调用
  keyHandlers: [] as Array<(e: KeyboardEvent) => boolean>,
  sendInputMock: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("@xterm/xterm", () => ({ Terminal: class {
  cols = 80; rows = 24;
  constructor(public options: { theme: unknown }) { terminalOptions.push(options); }
  loadAddon() {} open() {} onData() {} dispose() {} write() {}
  attachCustomKeyEventHandler(cb: (e: KeyboardEvent) => boolean) { keyHandlers.push(cb); }
} }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));
vi.mock("@xterm/addon-search", () => ({ SearchAddon: class { clearDecorations() {} } }));
vi.mock("@xterm/addon-webgl", () => ({ WebglAddon: class { onContextLoss() {} } }));
vi.mock("@tauri-apps/api/core", () => ({ Channel: class {}, invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock("../../src/ipc/client", () => ({ sendInput: sendInputMock, resizeTerminal: vi.fn(), listThemes: vi.fn(), setAppTheme: vi.fn(), setTerminalColorScheme: vi.fn() }));
vi.mock("../../src/ipc/events", () => ({ subscribeAppEvents: vi.fn() }));

afterEach(() => vi.unstubAllGlobals());

it("starts a newly mounted terminal with the already selected palette", async () => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const pinia = createPinia();
  setActivePinia(pinia);
  useThemeStore().terminalTheme = { background: "#123456", red: "#987654" };
  const wrapper = mount(TerminalPane, { props: { sessionId: "session-1" }, global: { plugins: [pinia] } });
  await flushPromises();
  expect(terminalOptions.at(-1)?.theme).toEqual({ background: "#123456", red: "#987654" });
  wrapper.unmount();
});

// R2-13：搜索快捷键只作用于当前激活终端——window 级 Ctrl+F/Escape 由 App.vue
// 统一拦截后经 rshell:terminal-action（带 sessionId）路由，面板按 sessionId
// 过滤；两个面板并存时一次按键不得同时作用于所有搜索栏。
it("applies routed find/closeFind only to the matching session and ignores raw window keydown", async () => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const pinia = createPinia();
  setActivePinia(pinia);
  const pane1 = mount(TerminalPane, { props: { sessionId: "session-1" }, global: { plugins: [pinia] } });
  const pane2 = mount(TerminalPane, { props: { sessionId: "session-2" }, global: { plugins: [pinia] } });
  await flushPromises();
  expect(pane1.find(".search-bar").exists()).toBe(false);
  expect(pane2.find(".search-bar").exists()).toBe(false);

  // 激活终端是 session-1：find 只弹出它的搜索栏
  window.dispatchEvent(new CustomEvent("rshell:terminal-action", { detail: { sessionId: "session-1", action: "find" } }));
  await flushPromises();
  expect(pane1.find(".search-bar").exists()).toBe(true);
  expect(pane2.find(".search-bar").exists()).toBe(false);

  // 原始 window keydown（Ctrl+F）不再被任何面板直接响应（回归：旧实现会同时切换所有面板）
  window.dispatchEvent(new KeyboardEvent("keydown", { ctrlKey: true, key: "f" }));
  await flushPromises();
  expect(pane1.find(".search-bar").exists()).toBe(true);
  expect(pane2.find(".search-bar").exists()).toBe(false);

  // closeFind 只关闭激活面板的搜索栏
  window.dispatchEvent(new CustomEvent("rshell:terminal-action", { detail: { sessionId: "session-1", action: "closeFind" } }));
  await flushPromises();
  expect(pane1.find(".search-bar").exists()).toBe(false);
  expect(pane2.find(".search-bar").exists()).toBe(false);

  // 切换激活终端为 session-2：find 只弹出 session-2 的搜索栏
  window.dispatchEvent(new CustomEvent("rshell:terminal-action", { detail: { sessionId: "session-2", action: "find" } }));
  await flushPromises();
  expect(pane1.find(".search-bar").exists()).toBe(false);
  expect(pane2.find(".search-bar").exists()).toBe(true);

  pane1.unmount();
  pane2.unmount();
});

  // 退格必须送 \x08 (BS)：xterm.js 默认送 \x7f (DEL)，而远端 shell 的行规程
  // 多以 \x08 作 erase 字符，收到 \x7f 时屏幕字符不消失（用户报「backspace 无效」）。
  it("sends BS for Backspace and leaves other keys to xterm", async () => {
    keyHandlers.length = 0;
    sendInputMock.mockClear();
    const wrapper = mount(TerminalPane, { props: { sessionId: "session-1" } });
    await flushPromises();

    const handler = keyHandlers.at(-1);
    expect(handler).toBeTypeOf("function");

    const ev = new KeyboardEvent("keydown", { key: "Backspace", cancelable: true });
    const handled = handler!(ev);
    expect(handled).toBe(false);
    expect(ev.defaultPrevented).toBe(true);
    expect(sendInputMock).toHaveBeenCalledOnce();
    const bytes = sendInputMock.mock.calls[0][1] as Uint8Array;
    expect(Array.from(bytes)).toEqual([0x08]);

    // 普通字符键交回 xterm 自行处理
    const other = new KeyboardEvent("keydown", { key: "a", cancelable: true });
    expect(handler!(other)).toBe(true);
    // Ctrl+Backspace 保留 xterm 默认（部分 shell 用 \x7f 删前一个词）
    const ctrlBack = new KeyboardEvent("keydown", { key: "Backspace", ctrlKey: true, cancelable: true });
    expect(handler!(ctrlBack)).toBe(true);
    expect(sendInputMock).toHaveBeenCalledOnce();

    wrapper.unmount();
  });