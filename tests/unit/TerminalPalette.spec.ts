import { afterEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import TerminalPane from "../../src/components/TerminalPane.vue";
import { useThemeStore } from "../../src/stores/theme";

const { terminalOptions } = vi.hoisted(() => ({ terminalOptions: [] as Array<{ theme: unknown }> }));
vi.mock("@xterm/xterm", () => ({ Terminal: class {
  cols = 80; rows = 24;
  constructor(public options: { theme: unknown }) { terminalOptions.push(options); }
  loadAddon() {} open() {} onData() {} dispose() {} write() {}
} }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));
vi.mock("@xterm/addon-search", () => ({ SearchAddon: class { clearDecorations() {} } }));
vi.mock("@xterm/addon-webgl", () => ({ WebglAddon: class { onContextLoss() {} } }));
vi.mock("@tauri-apps/api/core", () => ({ Channel: class {}, invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock("../../src/ipc/client", () => ({ sendInput: vi.fn(), resizeTerminal: vi.fn(), listThemes: vi.fn(), setAppTheme: vi.fn(), setTerminalColorScheme: vi.fn() }));
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
