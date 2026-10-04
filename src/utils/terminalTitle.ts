/**
 * 终端面板标签的短标题生成。
 *
 * dockview 的 `createPanel` 在 `addPanel` 未传 `title` 时会回退到
 * `options.id`（dockviewComponent.js: `title: options.title ?? options.id`）。
 * 本应用的面板 id 是 `terminal-<uuid>`，直接当标签会渲染成 45 个字符的
 * `terminal-1a2b3c4d-5e6f-7a8b-9c0d-1e2f3a4b5c6d`，把标签栏挤满。
 *
 * 这里统一按「会话名优先、短 id 兜底」的规则产出可读短标题。
 */

/** 标签最大字符数；超出部分用省略号收尾，避免挤压相邻标签。 */
export const MAX_TERMINAL_TITLE_LENGTH = 24;

/** 短 id 兜底时保留的 UUID 字符数。 */
const SHORT_ID_LENGTH = 8;

/** 省略号占 1 个字符宽。 */
const ELLIPSIS = "…";

function truncate(text: string, max: number): string {
  // Array.from 按码点切分，避免把代理对（emoji 等）截成半个字符。
  const chars = Array.from(text);
  if (chars.length <= max) return text;
  return chars.slice(0, Math.max(1, max - 1)).join("") + ELLIPSIS;
}

/**
 * 会话 id 的短格式：取首 8 位。UUID 本身对用户没有辨识度，
 * 仅在没有会话名（会话已被删除或尚未加载）时作为占位。
 */
export function shortSessionId(sessionId: string): string {
  return sessionId.slice(0, SHORT_ID_LENGTH);
}

/**
 * 生成终端面板的短标签。
 *
 * - 会话名去除首尾空白后优先使用；过长的名字按字符数截断加省略号。
 * - 会话名缺失或只有空白时，退回短 id。
 */
export function shortTerminalTitle(sessionName: string | null | undefined, sessionId: string): string {
  const name = (sessionName ?? "").trim();
  if (!name) return shortSessionId(sessionId);
  return truncate(name, MAX_TERMINAL_TITLE_LENGTH);
}
