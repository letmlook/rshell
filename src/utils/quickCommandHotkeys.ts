/**
 * 快速命令热键表 —— **共享**给「触发热键」与「终端吞键」两处使用。
 *
 * 真实事故（两处，都在这条链上）：
 * 1. 终端里按热键**永远不触发**：触发侧用裸 `isTextInputTarget` 判据，把 xterm 的
 *    helper textarea（它本身就是 `<textarea>`）也挡住了 —— 而那恰恰是热键该生效的
 *    地方。应用快捷键那条路用的是排除 helper 的判据，两条路径不一致。
 * 2. 即便触发了，**按键还会漏进远端 shell**：xterm 的按键处理在目标阶段先跑
 *    （事件冒泡到 window 之前），窗口处理器再 `preventDefault()` 已经太晚。
 *    终端侧必须自己把「用户绑定的热键」吞掉。
 *
 * 所以热键表不能只活在 App.vue 里：触发侧要它，终端侧也要它。放在这里，
 * 两边读同一份，行为必然一致。
 */
import { computed, ref } from "vue";
import { matchesHotkey, parseHotkey, type Hotkey } from "./hotkey";
import type { QuickCommand } from "../ipc/types";

/** 全部快速命令（含热键），由 App.vue 在列表变化时写入 */
const commands = ref<QuickCommand[]>([]);

/** 绑定了热键的快速命令：热键命中时从这里取 */
export const quickCommandHotkeys = computed<Array<{ command: QuickCommand; hotkey: Hotkey }>>(
  () => {
    const table: Array<{ command: QuickCommand; hotkey: Hotkey }> = [];
    for (const command of commands.value) {
      if (!command.hotkey) continue;
      const hotkey = parseHotkey(command.hotkey);
      if (hotkey) table.push({ command, hotkey });
    }
    return table;
  },
);

/** 写入快速命令列表（App.vue 在挂载与列表变化时调用） */
export function setQuickCommands(list: QuickCommand[]): void {
  commands.value = list;
}

/**
 * 这次按键是否命中某个用户绑定的热键。
 *
 * 终端侧用它决定「吞不吞这个键」：命中的键不能送进远端 shell
 * （否则按一次热键，远端会收到一串垃圾字符）。
 */
export function matchesUserHotkey(event: KeyboardEvent): boolean {
  return quickCommandHotkeys.value.some((item) => matchesHotkey(event, item.hotkey));
}
