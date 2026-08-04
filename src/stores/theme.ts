/**
 * theme pinia store —— 切片 3 + v2 主题生效链
 *
 * 持有当前主题名/方案名 + 可用列表 + 最近一次后端推送的完整颜色集。
 *
 * 生效链路：
 *   1. 用户在 ThemePanel 切换 → applyTheme / applyScheme 走 IPC SetAppTheme / SetTerminalColorScheme
 *   2. 后端把当前主题的最新 AppTheme 推 AppEvent::ThemeChanged
 *   3. 事件订阅 onThemeChanged 写入 current colors 并调 applyColors
 *   4. applyColors 把 RGBA u32 摊平到 :root 的 --rs-* / --rshell-* CSS 变量
 *   5. applyColors 同步刷新所有已注册 xterm 实例的 term.options.theme
 *   6. Element Plus 与全部组件通过引用 --rs-* 自动跟随
 */
import { defineStore } from "pinia";
import { ref } from "vue";
import {
  listThemes,
  setAppTheme as invokeSetTheme,
  setTerminalColorScheme as invokeSetScheme,
} from "../ipc/client";
import type { ThemeInfo } from "../ipc/client";
import { makeDispatcher, subscribeAppEvents, type UnlistenFn } from "../ipc/events";
import type { AppTheme, TerminalColorScheme } from "../ipc/types";
import {
  applyThemeCssVars,
  applyXtermTheme,
  paletteToXtermTheme,
  registerXterm,
  themeColorSetToCssVars,
  unregisterXterm,
  type ThemeColorSet,
} from "../utils/themeCss";

export const useThemeStore = defineStore("theme", () => {
  const currentTheme = ref<string>("default");
  const currentScheme = ref<string>("default");
  const availableThemes = ref<string[]>([]);
  const availableSchemes = ref<string[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

  // 后端最近一次推送的完整主题对象（含 colors）。ListThemes 当前不带 colors
  // （设计 §4.2 行），所以这个字段仅由 ThemeChanged 事件维护，用于后续切换
  // 触发 CSS 写入。Terminal 方案同理。
  const currentColors = ref<ThemeColorSet | null>(null);
  const currentTerminalPalette = ref<TerminalColorScheme["ansi_colors"] | null>(null);
  const currentTerminalTheme = ref<{
    default_fg: number;
    default_bg: number;
    cursor_fg: number;
    cursor_bg: number;
    selection_fg: number;
    selection_bg: number;
  } | null>(null);

  let unlisten: UnlistenFn | null = null;

  async function refresh() {
    loading.value = true;
    error.value = null;
    try {
      const info: ThemeInfo = await listThemes();
      currentTheme.value = info.current_theme;
      currentScheme.value = info.current_scheme;
      availableThemes.value = info.available_themes;
      availableSchemes.value = info.available_schemes;
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  async function applyTheme(themeName: string) {
    await invokeSetTheme(themeName);
    currentTheme.value = themeName;
    // 颜色集由后端 ThemeChanged 事件推送，无需前端再次拉取
  }

  async function applyScheme(schemeName: string) {
    await invokeSetScheme(schemeName);
    currentScheme.value = schemeName;
    // 终端配色由后端 ColorSchemeChanged 事件推送
  }

  /**
   * v2 主题生效链：接收 ThemeColors，写入 :root，并刷新 xterm。
   * 由事件订阅自动调用；外部不直接使用。
   */
  function applyColors(set: ThemeColorSet) {
    applyThemeCssVars(themeColorSetToCssVars(set));
  }

  /** 终端配色生效：刷新已挂载 xterm 实例的 ITheme。 */
  function applyTerminalPalette(scheme: TerminalColorScheme) {
    applyXtermTheme(paletteToXtermTheme(scheme));
  }

  /**
   * 订阅 AppEvent 流，路由 ThemeChanged / ColorSchemeChanged 到本 store。
   * 返回 unlisten 函数；App.vue 在 onBeforeUnmount 中调用。
   */
  async function subscribeEvents(): Promise<UnlistenFn> {
    if (unlisten) return unlisten;
    unlisten = await subscribeAppEvents(
      makeDispatcher({
        onThemeChanged: (theme: AppTheme) => {
          currentTheme.value = theme.name;
          currentColors.value = theme.colors;
          applyColors(theme.colors);
        },
        onColorSchemeChanged: (scheme: TerminalColorScheme) => {
          currentScheme.value = scheme.name;
          currentTerminalPalette.value = scheme.ansi_colors;
          currentTerminalTheme.value = {
            default_fg: scheme.default_fg,
            default_bg: scheme.default_bg,
            cursor_fg: scheme.cursor_fg,
            cursor_bg: scheme.cursor_bg,
            selection_fg: scheme.selection_fg,
            selection_bg: scheme.selection_bg,
          };
          applyTerminalPalette(scheme);
        },
      }),
    );
    return unlisten;
  }

  /**
   * 取消订阅。仅用于测试与组件卸载清理。
   */
  function unsubscribeEvents() {
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
  }

  return {
    currentTheme,
    currentScheme,
    availableThemes,
    availableSchemes,
    loading,
    error,
    currentColors,
    refresh,
    applyTheme,
    applyScheme,
    applyColors,
    applyTerminalPalette,
    subscribeEvents,
    unsubscribeEvents,
    // xterm 句柄代理 —— 暴露到 store 让 TerminalPane 注册/注销
    registerXterm,
    unregisterXterm,
  };
});