/**
 * 快速命令热键 —— 解析与匹配（纯函数）
 *
 * 热键写法：`Ctrl+Shift+1`、`Alt+F5`、`Meta+K`。修饰键大小写不敏感、顺序不限；
 * 主键必须是一个字符或一个功能键名。
 *
 * 匹配要求修饰键**完全一致**：按 `Ctrl+1` 注册的快捷键不会被 `Ctrl+Shift+1`
 * 触发，也不会被光秃秃的 `1` 触发——否则一个随手敲的字符就可能执行命令。
 */
export interface Hotkey {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
  /** 主键，统一转成小写 */
  key: string;
}

/** 功能键别名 → 标准键名 */
const FUNCTION_KEYS = new Set([
  "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12",
  "escape", "enter", "tab", "space", "backspace", "delete", "insert", "home", "end",
  "pageup", "pagedown", "up", "down", "left", "right",
]);

const MODIFIER_NAMES = new Set(["ctrl", "control", "alt", "option", "shift", "meta", "cmd", "command"]);

/**
 * 解析热键字符串；无法解析时返回 null（界面据此提示用户，不静默接受）。
 */
export function parseHotkey(text: string | null | undefined): Hotkey | null {
  if (!text) return null;
  const parts = text
    .split("+")
    .map((part) => part.trim().toLowerCase())
    .filter((part) => part.length > 0);
  if (parts.length === 0) return null;

  let ctrl = false;
  let alt = false;
  let shift = false;
  let meta = false;
  let key: string | null = null;

  for (const part of parts) {
    if (MODIFIER_NAMES.has(part)) {
      if (part === "ctrl" || part === "control") ctrl = true;
      else if (part === "alt" || part === "option") alt = true;
      else if (part === "shift") shift = true;
      else meta = true;
      continue;
    }
    if (key !== null) {
      // 两个主键：无法判定用哪个
      return null;
    }
    key = part;
  }

  if (key === null) return null;
  if (key.length > 1 && !FUNCTION_KEYS.has(key)) {
    // 多字符且不是已知功能键：多半是手滑多打了
    return null;
  }

  return { ctrl, alt, shift, meta, key };
}

/** 事件里的键名 → 与热键主键同一套写法 */
function eventKey(event: KeyboardEvent): string {
  const key = event.key.toLowerCase();
  if (key === " ") return "space";
  if (key === "escape") return "escape";
  if (key === "arrowup") return "up";
  if (key === "arrowdown") return "down";
  if (key === "arrowleft") return "left";
  if (key === "arrowright") return "right";
  return key;
}

/** 按下事件是否命中该热键 */
export function matchesHotkey(event: KeyboardEvent, hotkey: Hotkey): boolean {
  // Cmd 在 macOS 上对应 meta，在其它平台对应 ctrl：两边都按同一套语义处理
  const ctrl = event.ctrlKey;
  const meta = event.metaKey;
  if (hotkey.ctrl !== ctrl) return false;
  if (hotkey.meta !== meta) return false;
  if (hotkey.alt !== event.altKey) return false;
  if (hotkey.shift !== event.shiftKey) return false;
  return eventKey(event) === hotkey.key;
}

/** 热键的可读形式（用于界面回显） */
export function formatHotkey(hotkey: Hotkey): string {
  const parts: string[] = [];
  if (hotkey.ctrl) parts.push("Ctrl");
  if (hotkey.alt) parts.push("Alt");
  if (hotkey.shift) parts.push("Shift");
  if (hotkey.meta) parts.push("Meta");
  // 单字符与功能键（F1…F12）大写回显，其余按原样
  const isFunctionKey = /^f\d{1,2}$/.test(hotkey.key);
  parts.push(
    hotkey.key.length === 1 || isFunctionKey
      ? hotkey.key.toUpperCase()
      : hotkey.key,
  );
  return parts.join("+");
}

/**
 * 事件目标是否处在文本输入中。
 *
 * 输入框里敲 `Ctrl+Shift+1` 应当只是打字，不能顺手触发热键。
 */
export function isTextInputTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  // isContentEditable 在部分环境（含 jsdom）不生效，属性本身也要看
  if (target.isContentEditable) return true;
  const editable = target.getAttribute("contenteditable");
  if (editable !== null && editable !== "false") return true;
  const tag = target.tagName.toLowerCase();
  return tag === "input" || tag === "textarea" || tag === "select";
}

/**
 * 终端的隐藏输入框（xterm 的 helper textarea）。
 *
 * 它是 `<textarea>`，按普通输入框处理的话，**终端里就一个快捷键都触发不了**；
 * 但它确实是「用户正在终端里打字」的地方，所以要区别对待。
 */
export function isTerminalHelperTextarea(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement && target.classList.contains("xterm-helper-textarea")
  );
}

/**
 * 这次按键是否该被「文本输入」挡住（即不该触发用户绑定的热键）。
 *
 * 普通输入框（对话框、搜索框）里敲 Ctrl+Shift+1 只是文本 → 挡住。
 * **终端的 helper textarea 要放行** —— 那恰恰是热键该生效的地方。
 *
 * 真实事故：应用快捷键（Ctrl+F / Ctrl+Shift+C）用的是排除 helper 的判据
 * （`utils/shortcuts.ts` 的 `isTypingInTextField`），而用户绑定的快速命令热键
 * 用的是裸 `isTextInputTarget` —— 于是**终端里按热键永远不触发**
 * （helper textarea 本身就是 textarea），而热键唯一的用途就是在终端里发命令。
 * 两条路径必须用同一套判据。
 */
export function hotkeyBlockedByTarget(target: EventTarget | null): boolean {
  return isTextInputTarget(target) && !isTerminalHelperTextarea(target);
}
