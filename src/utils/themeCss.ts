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
  ansi_colors: number[];
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

/**
 * 把 ThemeColorSet 摊平到 CSS 变量字典。
 *
 * 同时写 `--rs-*`（v2 主语义层，被 tokens.css 与 Element Plus 经引用消费）
 * 与 `--rshell-*`（v1 旧别名层，仍有少量组件直接消费）。任一命名空间被改写
 * 都会立即驱动整个 UI。
 */
export function themeColorSetToCssVars(set: ThemeColorSet): ThemeCssVars {
  const bg = rgbaToCss(set.background);
  const fg = rgbaToCss(set.foreground);
  const accent = rgbaToCss(set.accent);
  const border = rgbaToCss(set.border);
  const sidebarBg = rgbaToCss(set.sidebar_bg);
  const toolbarBg = rgbaToCss(set.toolbar_bg);
  const statusbarBg = rgbaToCss(set.statusbar_bg);
  const selectionBg = rgbaToCss(set.selection_bg);
  const hoverBg = rgbaToCss(set.hover_bg);
  return {
    // v2 主语义层（驱动 Element Plus + 现代组件）
    "--rs-bg": bg,
    "--rs-bg-panel": sidebarBg,
    "--rs-bg-surface": toolbarBg,
    "--rs-bg-surface-hover": hoverBg,
    "--rs-fg": fg,
    "--rs-border": border,
    "--rs-border-strong": border,
    "--rs-row-selected": selectionBg,
    "--rs-row-hover": hoverBg,
    "--rs-accent": accent,
    "--rs-accent-dim": accent,

    // v1 旧别名层（仅供仍直接消费 --rshell-* 的组件）
    "--rshell-bg": bg,
    "--rshell-fg": fg,
    "--rshell-accent": accent,
    "--rshell-border": border,
    "--rshell-sidebar-bg": sidebarBg,
    "--rshell-toolbar-bg": toolbarBg,
    "--rshell-statusbar-bg": statusbarBg,
    "--rshell-selection-bg": selectionBg,
    "--rshell-hover-bg": hoverBg,
  };
}

/** 把 TerminalPalette 转为 xterm.js 期望的 ITheme */
export function paletteToXtermTheme(p: TerminalPalette): ITheme {
  const c = p.ansi_colors;
  const pick = (i: number, fallback: string) =>
    rgbaToCss(c[i] ?? fallbackToU32(fallback));
  return {
    foreground: rgbaToCss(p.default_fg),
    background: rgbaToCss(p.default_bg),
    cursor: rgbaToCss(p.cursor_bg),
    cursorAccent: rgbaToCss(p.cursor_fg),
    selectionBackground: rgbaToCss(p.selection_bg),
    selectionForeground: rgbaToCss(p.selection_fg),
    black: pick(0, "#000000"),
    red: pick(1, "#cd3131"),
    green: pick(2, "#0dbc79"),
    yellow: pick(3, "#e5e510"),
    blue: pick(4, "#2472c8"),
    magenta: pick(5, "#bc3fbc"),
    cyan: pick(6, "#11a8cd"),
    white: pick(7, "#e5e5e5"),
    brightBlack: pick(8, "#666666"),
    brightRed: pick(9, "#f14c4c"),
    brightGreen: pick(10, "#23d18b"),
    brightYellow: pick(11, "#f5f543"),
    brightBlue: pick(12, "#3b8eea"),
    brightMagenta: pick(13, "#d670d6"),
    brightCyan: pick(14, "#29b8db"),
    brightWhite: pick(15, "#ffffff"),
  };
}

/**
 * 把 `"#rrggbb"` 字符串编码为 u32 RGBA（高字节是 R，与 rgbaToCss 互逆）。
 * 用于 ANSI 缺位时的兜底，避免 null 检查散落到调用点。
 */
function fallbackToU32(hex: string): number {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  return ((r & 0xff) << 24) | ((g & 0xff) << 16) | ((b & 0xff) << 8);
}

/** 写入 :root CSS 变量;SSR/Node 环境安全（无 document 时直接返回） */
export function applyThemeCssVars(vars: ThemeCssVars): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  for (const [k, v] of Object.entries(vars)) {
    root.style.setProperty(k, v);
  }
}

// ===== xterm 句柄注册表 =====
//
// 主题变化时,所有已挂载的 xterm 实例需要同步刷新 term.options.theme 与 fit()
// 以重新计算 canvas 大小。这里提供一个轻量注册表,TerminalPane 在
// onMounted/onBeforeUnmount 维护,store 在主题切换时统一触发。

import type { Terminal } from "@xterm/xterm";

const xtermRegistry = new Set<Terminal>();

/** 注册 xterm 句柄;返回的函数用于注销。重复注册返回 no-op。 */
export function registerXterm(term: Terminal): () => void {
  if (xtermRegistry.has(term)) return () => unregisterXterm(term);
  xtermRegistry.add(term);
  return () => unregisterXterm(term);
}

/** 注销 xterm 句柄。 */
export function unregisterXterm(term: Terminal): void {
  xtermRegistry.delete(term);
}

/** 把同一组 ITheme 应用到所有已注册的 xterm 实例。 */
export function applyXtermTheme(theme: ITheme): void {
  for (const term of xtermRegistry) {
    try {
      term.options.theme = theme;
    } catch (e) {
      console.warn("[themeCss] failed to apply theme to xterm", e);
    }
  }
}

/** 仅供测试与调试:当前注册表大小。 */
export function _xtermRegistrySize(): number {
  return xtermRegistry.size;
}