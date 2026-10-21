/**
 * 终端偏好：选中即复制。
 *
 * 存 `localStorage` 而非后端：这是纯 UI 偏好（不是凭据、不是会话数据），
 * 与 lastLocalDir 同理，走 IPC 往返只会让首屏多一次阻塞等待。
 * 存储不可用（隐私模式、配额满、WebView 禁用）时降级为「进程内有效」，
 * 绝不抛错打断终端。
 *
 * 响应式导出而非只读函数：设置面板改开关时，已经打开的终端要**立即**
 * 生效，不必等面板重挂。只在读取时查一次 localStorage 的写法做不到这点。
 */
import { ref } from "vue";

export const COPY_ON_SELECT_KEY = "rshell.terminal.copyOnSelect.v1";

/** 存储里只认 "true"/"false"；缺失、损坏、非字符串一律回落到默认值 */
export function normalizeCopyOnSelect(raw: unknown, fallback = true): boolean {
  if (raw === "true") return true;
  if (raw === "false") return false;
  return fallback;
}

/** 从 localStorage 读取「选中即复制」；无记录或读取失败时返回默认值 */
export function loadCopyOnSelect(
  storage: Pick<Storage, "getItem"> | null | undefined,
  fallback = true,
): boolean {
  if (!storage) return fallback;
  try {
    return normalizeCopyOnSelect(storage.getItem(COPY_ON_SELECT_KEY), fallback);
  } catch {
    return fallback;
  }
}

/** 写入「选中即复制」；返回是否真的落盘（配额满时为 false，界面仍应显示新状态） */
export function saveCopyOnSelect(
  storage: Pick<Storage, "setItem"> | null | undefined,
  value: boolean,
): boolean {
  if (!storage) return false;
  try {
    storage.setItem(COPY_ON_SELECT_KEY, value ? "true" : "false");
    return true;
  } catch {
    return false;
  }
}

/** localStorage 可能不存在（SSR / 测试环境），统一走这个取值口 */
export function defaultLocalStorage(): Pick<Storage, "getItem" | "setItem"> | null {
  return typeof localStorage === "undefined" ? null : localStorage;
}

/**
 * 当前生效的「选中即复制」。设置面板写它，终端面板 watch 它。
 * 模块级单例：与 dialog/transferLog 同一套取舍，跨组件共享一份偏好。
 */
export const copyOnSelect = ref(loadCopyOnSelect(defaultLocalStorage()));

/** 改开关：更新内存态并落盘；落盘失败只说明「不记住」，不影响本次运行 */
export function setCopyOnSelect(value: boolean): boolean {
  copyOnSelect.value = value;
  return saveCopyOnSelect(defaultLocalStorage(), value);
}

// ===== 终端外观 =====

/**
 * 终端外观偏好：字号、字体、光标样式、滚动回���行数。
 *
 * 这些值此前在 TerminalPane 里写死（13px / 固定字体栈 / block 光标 / 10000 行），
 * 而 `TerminalConfig` 类型早已声明了同样���字段却从未接线——类型在承诺一个
 * 并不存在的能力。这里把声明接上：面板可改、已打开的终端立即生效。
 *
 * 与「选中即复制」同理存 localStorage：纯 UI 偏好，走 IPC 只会拖慢首屏。
 */
export const TERMINAL_APPEARANCE_KEY = "rshell.terminal.appearance.v1";

export type TerminalCursorStyle = "block" | "underline" | "bar";

export interface TerminalAppearance {
  fontSize: number;
  /** 字体栈（CSS font-family 值） */
  fontFamily: string;
  cursorStyle: TerminalCursorStyle;
  cursorBlink: boolean;
  scrollback: number;
}

/** 字体预设：只给几种常见等宽字体，避免自由输入导致的不可用字体 */
export const TERMINAL_FONT_PRESETS: ReadonlyArray<{ label: string; value: string }> = [
  {
    label: "系统等宽",
    value: 'Menlo, Monaco, "Courier New", monospace',
  },
  {
    label: "JetBrains Mono",
    value: '"JetBrains Mono", Menlo, Consolas, "DejaVu Sans Mono", monospace',
  },
  { label: "Menlo", value: "Menlo, monospace" },
  { label: "Consolas", value: 'Consolas, "DejaVu Sans Mono", monospace' },
  { label: "SF Mono", value: '"SF Mono", Menlo, monospace' },
];

export const DEFAULT_TERMINAL_APPEARANCE: TerminalAppearance = {
  fontSize: 13,
  fontFamily: TERMINAL_FONT_PRESETS[1].value,
  cursorStyle: "block",
  cursorBlink: true,
  scrollback: 10000,
};

const FONT_SIZE_RANGE = { min: 8, max: 32 } as const;
const SCROLLBACK_RANGE = { min: 0, max: 200000 } as const;
const CURSOR_STYLES: ReadonlySet<string> = new Set(["block", "underline", "bar"]);

/** 把任意输入夹到合法范围；非法值回落到默认值 */
export function normalizeTerminalAppearance(raw: unknown): TerminalAppearance {
  const fallback = DEFAULT_TERMINAL_APPEARANCE;
  if (typeof raw !== "object" || raw === null) return { ...fallback };
  const source = raw as Record<string, unknown>;

  const fontSize = Number(source.fontSize);
  const scrollback = Number(source.scrollback);
  const cursorStyle = String(source.cursorStyle ?? "");
  const fontFamily = String(source.fontFamily ?? "");

  return {
    fontSize: Number.isFinite(fontSize)
      ? Math.min(FONT_SIZE_RANGE.max, Math.max(FONT_SIZE_RANGE.min, Math.round(fontSize)))
      : fallback.fontSize,
    fontFamily: fontFamily.trim() ? fontFamily : fallback.fontFamily,
    cursorStyle: CURSOR_STYLES.has(cursorStyle)
      ? (cursorStyle as TerminalCursorStyle)
      : fallback.cursorStyle,
    cursorBlink:
      typeof source.cursorBlink === "boolean" ? source.cursorBlink : fallback.cursorBlink,
    scrollback: Number.isFinite(scrollback)
      ? Math.min(SCROLLBACK_RANGE.max, Math.max(SCROLLBACK_RANGE.min, Math.round(scrollback)))
      : fallback.scrollback,
  };
}

/** 从 localStorage 读取终端外观；无记录或读取失败时返回默认值 */
export function loadTerminalAppearance(
  storage: Pick<Storage, "getItem"> | null | undefined,
): TerminalAppearance {
  if (!storage) return { ...DEFAULT_TERMINAL_APPEARANCE };
  try {
    const raw = storage.getItem(TERMINAL_APPEARANCE_KEY);
    if (!raw) return { ...DEFAULT_TERMINAL_APPEARANCE };
    return normalizeTerminalAppearance(JSON.parse(raw));
  } catch {
    return { ...DEFAULT_TERMINAL_APPEARANCE };
  }
}

/** 写入终端外观；返回是否真的落盘 */
export function saveTerminalAppearance(
  storage: Pick<Storage, "setItem"> | null | undefined,
  value: TerminalAppearance,
): boolean {
  if (!storage) return false;
  try {
    storage.setItem(TERMINAL_APPEARANCE_KEY, JSON.stringify(value));
    return true;
  } catch {
    return false;
  }
}

/** 当前生效的终端外观（模块级单例，跨组件共享） */
export const terminalAppearance = ref<TerminalAppearance>(
  loadTerminalAppearance(defaultLocalStorage()),
);

/** 更新终端外观：内存态立即变，落盘失败只说明「不记住」 */
export function setTerminalAppearance(
  patch: Partial<TerminalAppearance>,
): TerminalAppearance {
  const next = normalizeTerminalAppearance({ ...terminalAppearance.value, ...patch });
  terminalAppearance.value = next;
  saveTerminalAppearance(defaultLocalStorage(), next);
  return next;
}
