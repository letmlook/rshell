/**
 * 应用级快捷键 —— 唯一定义处
 *
 * 之前快捷键散在两处：窗口的 keydown 监听（App.vue）与终端的按键处理
 * （TerminalPane 的 attachCustomKeyEventHandler）。两处各写一份判断，导致两个
 * 问题：
 *
 * 1. **工具栏承诺的快捷键不存在**：「新建连接 (Ctrl+N)」写在 title 上，
 *    全局处理器里却只有 Ctrl+Shift+C / Ctrl+Shift+V / Ctrl+F —— 按了没反应。
 * 2. **快捷键会漏给远端 shell**：xterm 的按键处理在目标阶段先跑（事件冒泡到
 *    window 之前），窗口处理器再 `preventDefault()` 已经太晚，Ctrl+F 之类
 *    照样把控制字符送进了远端 shell。
 *
 * 这里给出唯一判据：窗口处理器据此决定做什么，终端据此决定**吞不吞**这个键。
 * 应用接管的键一律不进远端 shell——否则「按 Ctrl+F 打开查找」的同时还会往
 * shell 里塞一个 0x06。
 */
import { hotkeyBlockedByTarget } from "./hotkey";

/** 应用接管的快捷键 */
export type AppShortcut = "new-session" | "find" | "copy" | "paste";

/**
 * 终端的隐藏输入框（xterm 的 helper textarea）。
 *
 * 它是 `<textarea>`，按普通输入框处理的话，终端里就一个快捷键都触发不了；
 * 但它确实是「用户正在终端里打字」的地方，所以要区别对待。
 *
 * 判据统一放在 `utils/hotkey.ts` 的 `hotkeyBlockedByTarget`：
 * 应用快捷键与**用户绑定的快速命令热键**必须用同一套，否则两者的行为不一致
 * （真实事故：应用快捷键在终端里有效，用户热键在终端里永远不触发）。
 */

/**
 * 事件是否落在普通文本输入里（对话框的输入框、搜索框等）。
 *
 * 在这些地方应用的终端级快捷键不该抢：原生复制粘贴与输入法更合适。
 * 终端自身的 helper textarea 是例外——那里快捷键就该生效。
 */
export function isTypingInTextField(target: EventTarget | null): boolean {
  return hotkeyBlockedByTarget(target);
}

/**
 * 判断按键事件对应哪个应用快捷键；不属于应用快捷键返回 null。
 *
 * 修饰键要求**完全一致**：`Ctrl+Shift+C` 与 `Ctrl+C` 是不同的键。
 */
export function matchAppShortcut(event: KeyboardEvent): AppShortcut | null {
  if (event.type !== "keydown") return null;
  if (isTypingInTextField(event.target)) return null;

  const accel = event.ctrlKey || event.metaKey;
  if (!accel) return null;
  const key = event.key.toLowerCase();

  if (event.shiftKey) {
    if (key === "c") return "copy";
    if (key === "v") return "paste";
    if (key === "n") return "new-session";
    return null;
  }
  if (key === "f") return "find";
  if (key === "n") return "new-session";
  return null;
}
