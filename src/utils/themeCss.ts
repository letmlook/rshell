/**
 * 主题颜色 → CSS 变量映射（设计 §5 / §4.2 "主题颜色 → CSS 变量"行）
 *
 * 后端只持久化当前主题名 + 配色方案名（不传整套颜色 JSON）;
 * 颜色值由前端根据 scheme 计算,写入 :root 的 CSS 变量供整个 UI 使用。
 *
 * 纯函数 —— 切片 3 起有 Vitest 单测覆盖。
 */

import type { ITheme } from "@xterm/xterm";

/** CSS 变量定义,key 是变量名,value 是 hex/rgb 字符串 */
export type ThemeCssVars = Record<string, string>;

/** 主题颜色 RGBA u32（与 rshell_api::types::ThemeColors 对齐） */
export interface ThemeColorSet {
  background: number;
  foreground: number;
  accent: number;
  border: number;
  sidebar_bg: number;
  toolbar_bg: number;
  statusbar_bg: number;
  selection_bg: number;
  hover_bg: number;
}

/** 终端 ANSI 16 色调色板 + 默认 fg/bg/cursor/selection */
export interface TerminalPalette {
  ansi_colors: [number, number, number, number, number, number, number, number, number, number, number, number, number, number, number, number];
  default_fg: number;
  default_bg: number;
  cursor_fg: number;
  cursor_bg: number;
  selection_fg: number;
  selection_bg: number;
}

/** u32 RGBA → "#rrggbb" 字符串（忽略 alpha,前端主题色用 RGB 表达） */
export function rgbaToCss(c: number): string {
  const r = (c >> 24) & 0xff;
  const g = (c >> 16) & 0xff;
  const b = (c >> 8) & 0xff;
  return `#${[r, g, b].map((v) => v.toString(16).padStart(2, "0")).join("")}`;
}

/** u32 RGBA → xterm ITheme 期望的 "#rrggbb" 字符串 */
function paletteToXterm(c: number): string {
  return rgbaToCss(c);
}

/** 把 ThemeColorSet 摊平到 CSS 变量字典 */
export function themeColorSetToCssVars(set: ThemeColorSet): ThemeCssVars {
  return {
    "--rshell-bg": rgbaToCss(set.background),
    "--rshell-fg": rgbaToCss(set.foreground),
    "--rshell-accent": rgbaToCss(set.accent),
    "--rshell-border": rgbaToCss(set.border),
    "--rshell-sidebar-bg": rgbaToCss(set.sidebar_bg),
    "--rshell-toolbar-bg": rgbaToCss(set.toolbar_bg),
    "--rshell-statusbar-bg": rgbaToCss(set.statusbar_bg),
    "--rshell-selection-bg": rgbaToCss(set.selection_bg),
    "--rshell-hover-bg": rgbaToCss(set.hover_bg),
  };
}

/** 把 TerminalPalette 转为 xterm.js 期望的 ITheme */
export function paletteToXtermTheme(p: TerminalPalette): ITheme {
  const [c0, c1, c2, c3, c4, c5, c6, c7, c8, c9, c10, c11, c12, c13, c14, c15] = p.ansi_colors;
  return {
    foreground: paletteToXterm(p.default_fg),
    background: paletteToXterm(p.default_bg),
    cursor: paletteToXterm(p.cursor_bg),
    cursorAccent: paletteToXterm(p.cursor_fg),
    selectionBackground: paletteToXterm(p.selection_bg),
    selectionForeground: paletteToXterm(p.selection_fg),
    black: paletteToXterm(c0),
    red: paletteToXterm(c1),
    green: paletteToXterm(c2),
    yellow: paletteToXterm(c3),
    blue: paletteToXterm(c4),
    magenta: paletteToXterm(c5),
    cyan: paletteToXterm(c6),
    white: paletteToXterm(c7),
    brightBlack: paletteToXterm(c8),
    brightRed: paletteToXterm(c9),
    brightGreen: paletteToXterm(c10),
    brightYellow: paletteToXterm(c11),
    brightBlue: paletteToXterm(c12),
    brightMagenta: paletteToXterm(c13),
    brightCyan: paletteToXterm(c14),
    brightWhite: paletteToXterm(c15),
  };
}

/** 写入 :root CSS 变量;SSR/Node 环境安全（无 document 时直接返回） */
export function applyThemeCssVars(vars: ThemeCssVars): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  for (const [k, v] of Object.entries(vars)) {
    root.style.setProperty(k, v);
  }
}