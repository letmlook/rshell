/**
 * 终端配色方案文件的解析与校验 —— 纯函数，供「导入配色方案」使用。
 *
 * 方案文件是 JSON，颜色为 0xRRGGBB 整数（与后端 `rshell_api::types::TerminalColorScheme`
 * 的字段一一对应）。解析失败必须给出**具体**原因（缺哪个字段、哪个值越界），
 * 不能只说「格式错误」让用户自己猜。
 */
import type { TerminalColorScheme } from "../ipc/types";

/** 允许的颜色取值范围：0x000000 ~ 0xFFFFFF */
const MAX_COLOR = 0xffffff;

/** ANSI 调色板必须正好 16 项 */
const ANSI_COUNT = 16;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readColor(
  source: Record<string, unknown>,
  field: string,
): number | string {
  const value = source[field];
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return `字段「${field}」必须是数字（0xRRGGBB）`;
  }
  if (!Number.isInteger(value) || value < 0 || value > MAX_COLOR) {
    return `字段「${field}」超出范围（0..${MAX_COLOR}）`;
  }
  return value;
}

/**
 * 解析并校验一个配色方案对象。
 *
 * @throws Error 中文错误信息，说明具体哪里不合法
 */
export function parseColorScheme(raw: unknown): TerminalColorScheme {
  if (!isRecord(raw)) {
    throw new Error("配色方案文件必须是一个 JSON 对象");
  }

  const name = raw.name;
  if (typeof name !== "string" || name.trim() === "") {
    throw new Error("配色方案缺少「name」字段");
  }

  const ansi = raw.ansi_colors;
  if (!Array.isArray(ansi)) {
    throw new Error("配色方案缺少「ansi_colors」数组");
  }
  if (ansi.length !== ANSI_COUNT) {
    throw new Error(`「ansi_colors」必须有 ${ANSI_COUNT} 项，实际 ${ansi.length} 项`);
  }

  const ansiColors: number[] = [];
  for (let i = 0; i < ansi.length; i += 1) {
    const value = ansi[i];
    if (typeof value !== "number" || !Number.isInteger(value) || value < 0 || value > MAX_COLOR) {
      throw new Error(`「ansi_colors[${i}]」必须是 0..${MAX_COLOR} 的整数`);
    }
    ansiColors.push(value);
  }

  const fields = [
    "default_fg",
    "default_bg",
    "cursor_fg",
    "cursor_bg",
    "selection_fg",
    "selection_bg",
  ] as const;

  const colors: Record<string, number> = {};
  for (const field of fields) {
    const result = readColor(raw, field);
    if (typeof result === "string") throw new Error(result);
    colors[field] = result;
  }

  return {
    name: name.trim(),
    ansi_colors: ansiColors as TerminalColorScheme["ansi_colors"],
    default_fg: colors.default_fg,
    default_bg: colors.default_bg,
    cursor_fg: colors.cursor_fg,
    cursor_bg: colors.cursor_bg,
    selection_fg: colors.selection_fg,
    selection_bg: colors.selection_bg,
  };
}

/** 解析 JSON 文本并校验；错误信息统一带前缀 */
export function parseColorSchemeText(text: string): TerminalColorScheme {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch (e) {
    throw new Error(`不是合法的 JSON：${e instanceof Error ? e.message : String(e)}`);
  }
  return parseColorScheme(raw);
}
