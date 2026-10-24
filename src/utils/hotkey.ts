/**
 * 快速命令热键 —— 解析与匹配（纯函数）
 *
 * 热键写法：`Ctrl+Shift+1`、`Alt+F5`、`Meta+K`。修饰键大小写不敏感、顺序不限；
 * 主键必须是一个字符或一个功能键名。
 *
 * 匹配要求修饰键**完全一致**：按 `Ctrl+1` 注册的快捷键不会被 `Ctrl+Shift+1`
 * 触发，也不会被光秃秃的 `1` 触发——否则一个随手敲的字符就可能执行命令。
 *
 * **主键按物理键位（`event.code`）匹配，而不是 `event.key`**：
 * 带 Shift 时 `event.key` 会变成**符号**（US 布局下 Shift+9 是 `(`、Shift+1 是 `!`），
 * 于是「绑定了带 Shift 的数字/符号热键」永远匹配不上 —— 用户在面板里写
 * `Ctrl+Shift+9`，按下去毫无反应。`event.code` 是布局无关的物理键位
 * （`Digit9`），不受 Shift 影响，是热键该用的字段。
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

/**
 * 物理键位（`event.code`）→ 与热键主键同一套写法。
 *
 * 数字/符号键的 code 是 `Digit1` / `Minus` 这类物理位置名，**不受 Shift 影响** ——
 * 这正是热键匹配需要的语义。字母键的 code 是 `KeyA`，与用户写的 `a` 对应。
 */
function codeKey(code: string): string | null {
  if (!code) return null;
  if (/^Key[A-Z]$/.test(code)) return code.slice(3).toLowerCase();
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^Numpad[0-9]$/.test(code)) return code.slice(6);
  return null;
}

/** 事件里的键名 → 与热键主键同一套写法（`event.key` 语义） */
function keyKey(event: KeyboardEvent): string {
  const key = event.key.toLowerCase();
  if (key === " ") return "space";
  if (key === "escape") return "escape";
  if (key === "arrowup") return "up";
  if (key === "arrowdown") return "down";
  if (key === "arrowleft") return "left";
  if (key === "arrowright") return "right";
  return key;
}

/**
 * 事件里的主键候选 → 与热键主键同一套写法。
 *
 * **数字键**给两个候选：物理键位（`event.code` → `Digit9` → `9`）与实际打出来的
 * 字符（`event.key` → `(`）。两者任一命中即算匹配：
 *
 * - 用户写 `Ctrl+Shift+9`（键帽上的 9）→ 靠 code 命中。带 Shift 时 `event.key`
 *   是 `(`，只看 key 永远匹配不上 —— 这正是原来的缺陷。
 * - 用户写 `Ctrl+Shift+(`（符号键）→ 靠 key 命中。
 *
 * **字母键只认 `event.key`**（与改动前的行为完全一致）：`event.key` 是**实际打出来
 * 的字符**，在 QWERTZ / Dvorak 布局上它与物理键位不同（物理 `KeyY` 那一格打出来
 * 是 `z`）。若字母也收物理键位候选，一次按键会**同时命中 `y` 与 `z` 两条热键**。
 */
function eventKeys(event: KeyboardEvent): string[] {
  const fromCode = codeKey(event.code);
  const fromKey = keyKey(event);
  if (!fromCode || fromCode === fromKey) return [fromKey];
  // 物理键位候选只用于数字键（见上面的理由）
  if (!/^[0-9]$/.test(fromCode)) return [fromKey];
  return [fromCode, fromKey];
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
  return eventKeys(event).includes(hotkey.key);
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
