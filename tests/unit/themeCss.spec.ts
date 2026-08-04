/**
 * themeCss 单元测试 —— 切片 3（设计 §6.2）
 *
 * 验证 scheme → CSS 变量 / xterm ITheme 的映射是纯函数,
 * 任何对 u32 RGBA 的边界(0xFFFFFFFF / 0)都能正确处理。
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
  background: 0x000000ff,
  foreground: 0xffffffff,
  accent: 0x0066ccff,
  border: 0xccccccff,
  sidebar_bg: 0x101010ff,
  toolbar_bg: 0x202020ff,
  statusbar_bg: 0x303030ff,
  selection_bg: 0x4060a0ff,
  hover_bg: 0x505050ff,
};

const samplePalette: TerminalPalette = {
  ansi_colors: [
    0x000000ff, 0xcd0000ff, 0x00cd00ff, 0xcdcd00ff,
    0x0000eeff, 0xcd00cdff, 0x00cdcdff, 0xe5e5e5ff,
    0x7f7f7fff, 0xff0000ff, 0x00ff00ff, 0xffff00ff,
    0x5c5cffff, 0xff00ffff, 0x00ffffff, 0xffffffff,
  ],
  default_fg: 0xffffffff,
  default_bg: 0x000000ff,
  cursor_fg: 0x000000ff,
  cursor_bg: 0xffffffff,
  selection_fg: 0xffffffff,
  selection_bg: 0x4060a0ff,
};

describe("rgbaToCss", () => {
  it("formats RGB only, ignoring alpha", () => {
    expect(rgbaToCss(0x000000ff)).toBe("#000000");
    expect(rgbaToCss(0xffffffff)).toBe("#ffffff");
    expect(rgbaToCss(0x0066ccff)).toBe("#0066cc");
  });
  it("pads single-digit components to 2", () => {
    expect(rgbaToCss(0x010203ff)).toBe("#010203");
  });
});

describe("themeColorSetToCssVars", () => {
  it("emits both v2 --rs-* and v1 --rshell-* namespaces", () => {
    const vars = themeColorSetToCssVars(black);
    // v2 主语义层（驱动 Element Plus + 现代组件）
    expect(vars["--rs-bg"]).toBe("#000000");
    expect(vars["--rs-fg"]).toBe("#ffffff");
    expect(vars["--rs-accent"]).toBe("#0066cc");
    expect(vars["--rs-border"]).toBe("#cccccc");
    expect(vars["--rs-bg-panel"]).toBe("#101010");
    expect(vars["--rs-row-selected"]).toBe("#4060a0");
    // v1 旧别名层（供仍直接消费 --rshell-* 的组件）
    expect(vars["--rshell-bg"]).toBe("#000000");
    expect(vars["--rshell-fg"]).toBe("#ffffff");
    expect(vars["--rshell-accent"]).toBe("#0066cc");
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