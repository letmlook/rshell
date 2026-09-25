/**
 * themeCss 单元测试 —— 切片 3（设计 §6.2）
 *
 * 验证 scheme → CSS 变量 / xterm ITheme 的映射是纯函数,
 * Rust ThemeColors 使用 0xRRGGBB，所有语义 token 必须与组件一致。
 */
import { describe, it, expect } from "vitest";
import {
  rgbaToCss,
  themeColorSetToCssVars,
  paletteToXtermTheme,
  applyThemeCssVars,
  type ThemeColorSet,
  type TerminalPalette,
} from "../../src/utils/themeCss";

const black: ThemeColorSet = {
  background: 0x000000,
  foreground: 0xffffff,
  accent: 0x0066cc,
  border: 0xcccccc,
  sidebar_bg: 0x101010,
  toolbar_bg: 0x202020,
  statusbar_bg: 0x303030,
  selection_bg: 0x4060a0,
  hover_bg: 0x505050,
};

const samplePalette: TerminalPalette = {
  ansi_colors: [
    0x000000, 0xcd0000, 0x00cd00, 0xcdcd00,
    0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5,
    0x7f7f7f, 0xff0000, 0x00ff00, 0xffff00,
    0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
  ],
  default_fg: 0xffffff,
  default_bg: 0x000000,
  cursor_fg: 0x000000,
  cursor_bg: 0xffffff,
  selection_fg: 0xffffff,
  selection_bg: 0x4060a0,
};

describe("rgbaToCss", () => {
  it("formats Rust's 0xRRGGBB values", () => {
    expect(rgbaToCss(0x000000)).toBe("#000000");
    expect(rgbaToCss(0xffffff)).toBe("#ffffff");
    expect(rgbaToCss(0x0066cc)).toBe("#0066cc");
  });
  it("pads single-digit components to 2", () => {
    expect(rgbaToCss(0x010203)).toBe("#010203");
  });
});

describe("themeColorSetToCssVars", () => {
  it("emits all 9 CSS variables", () => {
    const vars = themeColorSetToCssVars(black);
    expect(vars["--rs-bg"]).toBe("#000000");
    expect(vars["--rs-fg"]).toBe("#ffffff");
    expect(vars["--rs-accent"]).toBe("#0066cc");
    expect(vars["--rs-bg-panel"]).toBe("#101010");
  });
});

describe("paletteToXtermTheme", () => {
  it("maps all 16 ANSI + foreground/background/cursor/selection", () => {
    const theme = paletteToXtermTheme(samplePalette);
    expect(theme.foreground).toBe("#ffffff");
    expect(theme.background).toBe("#000000");
    expect(theme.black).toBe("#000000");
    expect(theme.red).toBe("#cd0000");
    expect(theme.brightWhite).toBe("#ffffff");
  });
});

describe("applyThemeCssVars", () => {
  it("does not throw when document is undefined", () => {
    const original = (globalThis as { document?: unknown }).document;
    (globalThis as { document?: unknown }).document = undefined;
    expect(() => applyThemeCssVars(themeColorSetToCssVars(black))).not.toThrow();
    (globalThis as { document?: unknown }).document = original;
  });
});
