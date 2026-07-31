// src/store/useThemeStore.ts - 主题切换 (Dark/Light)
// 把当前主题写到 document.documentElement 的 data-theme 属性上,CSS 变量自动切换。

import { create } from "zustand";

export type ThemeMode = "dark" | "light";

interface ThemeState {
  mode: ThemeMode;
  setMode: (mode: ThemeMode) => void;
  toggle: () => void;
}

function applyMode(mode: ThemeMode) {
  document.documentElement.setAttribute("data-theme", mode);
}

export const useThemeStore = create<ThemeState>((set, get) => ({
  // 初始从 <html data-theme> 读取,默认为 dark(SPEC §8.1 默认深色)
  mode: (document.documentElement.getAttribute("data-theme") as ThemeMode) || "dark",
  setMode: (mode) => {
    applyMode(mode);
    set({ mode });
  },
  toggle: () => {
    const next = get().mode === "dark" ? "light" : "dark";
    applyMode(next);
    set({ mode: next });
  },
}));

// 启动时应用一次,确保 html 属性和 store 状态一致
if (!document.documentElement.hasAttribute("data-theme")) {
  applyMode("dark");
}