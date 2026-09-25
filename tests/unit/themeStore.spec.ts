import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useThemeStore } from "../../src/stores/theme";
import { listThemes } from "../../src/ipc/client";
import { subscribeAppEvents } from "../../src/ipc/events";
import type { AppEvent } from "../../src/ipc/types";

vi.mock("../../src/ipc/client", () => ({
  listThemes: vi.fn(), setAppTheme: vi.fn(), setTerminalColorScheme: vi.fn(),
}));
vi.mock("../../src/ipc/events", () => ({ subscribeAppEvents: vi.fn() }));

const colors = {
  background: 0x111111, foreground: 0xeeeeee, accent: 0x123456,
  border: 0x333333, sidebar_bg: 0x222222, toolbar_bg: 0x282828,
  statusbar_bg: 0x121212, selection_bg: 0x444444, hover_bg: 0x555555,
};
const palette = {
  name: "Default", ansi_colors: Array(16).fill(0) as [number, number, number, number, number, number, number, number, number, number, number, number, number, number, number, number],
  default_fg: 0xffffff, default_bg: 0x000000, cursor_fg: 0, cursor_bg: 0xffffff,
  selection_fg: 0xffffff, selection_bg: 0x333333,
};

describe("theme store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    vi.mocked(listThemes).mockResolvedValue({ current_theme: "Dark", current_scheme: "Default", current_colors: colors, current_palette: palette, available_themes: ["Dark", "Light"], available_schemes: ["Default"] });
  });

  it("applies real backend colors to the CSS tokens components use", async () => {
    const store = useThemeStore();
    await store.refresh();
    expect(document.documentElement.style.getPropertyValue("--rs-bg")).toBe("#111111");
    expect(document.documentElement.style.getPropertyValue("--rs-accent")).toBe("#123456");
  });

  it("retains the selected palette for terminals mounted after refresh", async () => {
    const store = useThemeStore();
    await store.refresh();
    expect(store.terminalTheme).toMatchObject({ background: "#000000", foreground: "#ffffff", selectionBackground: "#333333" });
    let handler: ((event: AppEvent) => void) | undefined;
    vi.mocked(subscribeAppEvents).mockImplementation(async callback => { handler = callback; return vi.fn<() => void>(); });
    await store.subscribeEvents();
    handler?.({ ColorSchemeChanged: { scheme: { ...palette, default_bg: 0x123456 } } });
    expect(store.terminalTheme?.background).toBe("#123456");
  });

  it("unsubscribes once after receiving theme events", async () => {
    const unlisten = vi.fn();
    let handler: ((event: AppEvent) => void) | undefined;
    vi.mocked(subscribeAppEvents).mockImplementation(async (callback) => { handler = callback; return unlisten; });
    const store = useThemeStore();
    await store.subscribeEvents();
    await store.subscribeEvents();
    expect(subscribeAppEvents).toHaveBeenCalledOnce();
    handler?.({ ThemeChanged: { theme: { name: "Light", mode: "Light", colors: { ...colors, background: 0xffffff } } } });
    expect(store.currentTheme).toBe("Light");
    expect(document.documentElement.style.getPropertyValue("--rs-bg")).toBe("#ffffff");
    store.disposeEvents();
    store.disposeEvents();
    expect(unlisten).toHaveBeenCalledOnce();
  });
});
