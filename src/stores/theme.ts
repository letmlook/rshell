/**
 * theme pinia store —— 切片 3 + v2 主题生效链
 *
 * 持有当前主题名/方案名 + 可用列表。
 * 后端返回当前主题和终端配色的真实颜色;本 store 将其应用到 CSS/xterm。
 */
import { defineStore } from "pinia";
import { ref } from "vue";
import { listThemes, setAppTheme as invokeSetTheme, setTerminalColorScheme as invokeSetScheme } from "../ipc/client";
import type { ThemeInfo } from "../ipc/client";
import { subscribeAppEvents } from "../ipc/events";
import type { TerminalColorScheme } from "../ipc/types";
import { applyThemeCssVars, themeColorSetToCssVars, paletteToXtermTheme, type ThemeColorSet } from "../utils/themeCss";

export const useThemeStore = defineStore("theme", () => {
  const currentTheme = ref<string>("default");
  const currentScheme = ref<string>("default");
  const terminalTheme = ref<ReturnType<typeof paletteToXtermTheme> | null>(null);
  const availableThemes = ref<string[]>([]);
  const availableSchemes = ref<string[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  let unlisten: (() => void) | null = null;
  let subscriptionGeneration = 0;

  function applyPalette(palette: TerminalColorScheme) {
    terminalTheme.value = paletteToXtermTheme(palette);
    if (typeof window !== "undefined") window.dispatchEvent(new CustomEvent("rshell:terminal-theme", { detail: terminalTheme.value }));
  }

  async function refresh() {
    loading.value = true;
    error.value = null;
    try {
      const info: ThemeInfo = await listThemes();
      currentTheme.value = info.current_theme;
      currentScheme.value = info.current_scheme;
      availableThemes.value = info.available_themes;
      availableSchemes.value = info.available_schemes;
      applyColors(info.current_colors);
      applyPalette(info.current_palette);
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  async function applyTheme(themeName: string) {
    const previous = currentTheme.value;
    try { await invokeSetTheme(themeName); await refresh(); }
    catch (e) { error.value = String(e); currentTheme.value = previous; }
  }

  async function applyScheme(schemeName: string) {
    const previous = currentScheme.value;
    try { await invokeSetScheme(schemeName); await refresh(); }
    catch (e) { error.value = String(e); currentScheme.value = previous; }
  }

  /**
   * 接收后端的颜色集并写入 :root。
   */
  function applyColors(set: ThemeColorSet) {
    applyThemeCssVars(themeColorSetToCssVars(set));
  }

  async function subscribeEvents() {
    if (unlisten) return;
    const generation = ++subscriptionGeneration;
    const stop = await subscribeAppEvents((event) => {
      if (typeof event === "string") return;
      if ("ThemeChanged" in event) {
        currentTheme.value = event.ThemeChanged.theme.name;
        applyColors(event.ThemeChanged.theme.colors);
      } else if ("ColorSchemeChanged" in event) {
        currentScheme.value = event.ColorSchemeChanged.scheme.name;
        applyPalette(event.ColorSchemeChanged.scheme);
      }
    });
    if (generation === subscriptionGeneration) unlisten = stop;
    else stop();
  }

  function disposeEvents() { subscriptionGeneration++; unlisten?.(); unlisten = null; }

  return {
    currentTheme,
    currentScheme,
    terminalTheme,
    availableThemes,
    availableSchemes,
    loading,
    error,
    refresh,
    applyTheme,
    applyScheme,
    applyColors,
    subscribeEvents,
    disposeEvents,
  };
});
