/**
 * 全局右键菜单守卫
 *
 * macOS 的 WKWebView 在右键时会弹出 WebKit 原生菜单（Reload / Inspect Element）。
 * 对桌面客户端来说这是浏览器味的残留，两个害处：
 *
 *   - 原生菜单盖在应用自己的右键菜单上，用户同时看到两套菜单，不知道该点哪个；
 *   - 其中的 Reload 会**重载整个 WebView**，前端靠事件维持的状态全部丢失
 *     （连接状态、传输队列、面板布局都得重来一遍）。
 *
 * 所以默认吞掉 contextmenu 的默认行为，只给「文本输入区」留原生菜单 ——
 * 主密码框、路径输入框这类地方用户要靠右键粘贴。
 *
 * 用 capture 阶段监听：早于组件自己的处理器，但不影响它们 ——
 * preventDefault 只取消默认行为，不阻止事件继续传播，组件照常弹出自己的菜单。
 */

function isEditableTextTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  // contenteditable 的**子元素**上右键时，目标自身没有这个属性 ——
  // 要向上找，否则用户在可编辑区域里右键粘贴会被吞掉。
  const editableHost = target.closest("[contenteditable]");
  if (editableHost) {
    const value = editableHost.getAttribute("contenteditable");
    if (value !== "false") return true;
  }
  const tag = target.tagName.toLowerCase();
  if (tag === "input") return true;
  if (tag === "textarea") {
    // xterm 的 helper textarea 在终端画布里，终端自己管右键菜单
    return !target.classList.contains("xterm-helper-textarea");
  }
  return false;
}

/**
 * 是否显式要求保留原生右键菜单。
 *
 * 只读文本区（传输日志等）没有自己的右键菜单，用户靠原生菜单复制选中的文字 ——
 * 全局守卫把它们一并吞掉就会失去复制能力。这类区域用
 * `data-allow-native-menu` 标记。
 */
function allowsNativeMenu(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && !!target.closest("[data-allow-native-menu]");
}

/**
 * 安装守卫。返回卸载函数（调用方必须在卸载时调用，否则监听会残留）。
 */
export function installContextMenuGuard(target: EventTarget = window): () => void {
  /**
   * 本次右键按下期间已经收到的 contextmenu 次数。
   *
   * WebKit（macOS 的 WKWebView）一次右键会派发**两次** contextmenu：
   * mousedown 一次、mouseup 一次。第一次落在被右击的元素上 → 应用弹出自己的
   * 菜单；菜单随即出现在光标下，第二次就落在**菜单自己**（或菜单的遮罩）身上。
   *
   * 应用自己的菜单普遍按「收到 contextmenu 就关自己 / 点别处就关菜单」的语义
   * 处理，于是第二次事件把刚弹出的菜单**立刻关掉** —— 真机实测（macOS）：
   * 会话列表、文件列表、传输面板、终端四个右键菜单全部「一点开就没」。
   *
   * 判据是「**同一次按下**里收到的第几次」，而**不是时间窗口**：两次事件的
   * 间隔等于用户按住右键的时长（迟疑一下可以超过一秒），任何固定的时间窗口
   * 都会在用户按住稍久时失效。每一次右键按下（mousedown）把计数清零，
   * 之后收到的第一个 contextmenu 放行、后续的吞掉。
   */
  let contextMenuCount = 0;

  const onMouseDown = (event: Event) => {
    if ((event as MouseEvent).button === 2) {
      contextMenuCount = 0;
    }
  };

  const onContextMenu = (event: Event) => {
    if (isEditableTextTarget(event.target)) return;
    if (allowsNativeMenu(event.target)) return;
    event.preventDefault();
    contextMenuCount += 1;
    if (contextMenuCount > 1) {
      // 同一次右键的第二个 contextmenu：吞掉，不让它冒泡到
      // 「点别处就关菜单」的 window 监听器，也不让它触发菜单自身的关闭逻辑。
      event.stopPropagation();
    }
  };
  target.addEventListener("mousedown", onMouseDown, true);
  target.addEventListener("contextmenu", onContextMenu, true);
  return () => {
    target.removeEventListener("mousedown", onMouseDown, true);
    target.removeEventListener("contextmenu", onContextMenu, true);
  };
}
