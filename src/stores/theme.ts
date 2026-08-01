/**
 * theme pinia store —— 切片 3 + v2 主题生效链
 *
 * 持有当前主题名/方案名 + 可用列表。
 * 主题颜色 → CSS 变量的映射（设计 §4.2 "主题颜色 → CSS 变量"行）在
 * `utils/themeCss.ts` 实现：后端只持久化"当前主题名 + 自定义方案列表",
 * 颜色值由前端根据 scheme 计算。
 */
import { defineStore } from "pinia";
import { ref } from "vue";
import { listThemes, setAppTheme as invokeSetTheme, setTerminalColorScheme as invokeSetScheme } from "../ipc/client";
import type { ThemeInfo } from "../ipc/client";
import { applyThemeCssVars, themeColorSetToCssVars, type ThemeColorSet } from "../utils/themeCss";

export const useThemeStore = defineStore("theme", () => {
  const currentTheme = ref<string>("default");
  const currentScheme = ref<string>("default");
  const availableThemes = ref<string[]>([]);
  const availableSchemes = ref<string[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

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
  }

  async function applyScheme(schemeName: string) {
    await invokeSetScheme(schemeName);
    currentScheme.value = schemeName;
  }

  /**
   * v2 主题生效链:接收来自后端的颜色集,写入 :root。
   * 后端目前不返回 ThemeColorSet,这是占位实现 —— 一旦 backend 把
   * current colors 暴露在 listThemes 里,这里无缝生效。
   */
  function applyColors(set: ThemeColorSet) {
    applyThemeCssVars(themeColorSetToCssVars(set));
  }

  return {
    currentTheme,
    currentScheme,
    availableThemes,
    availableSchemes,
    loading,
    error,
    refresh,
    applyTheme,
    applyScheme,
    applyColors,
  };
});