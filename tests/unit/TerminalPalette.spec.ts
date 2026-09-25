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
vi.mock("@xterm/addon-search", () => ({ SearchAddon: class {} }));
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
