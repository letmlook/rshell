/**
 * 快速命令的作用范围解析 —— 纯函数
 *
 * 手动点「执行」与热键触发走**同一套**范围解析：否则热键会把命令发到
 * 与按钮不同的会话上，用户会以为热键坏了。
 */
import type { QuickCommand, SessionConfig, Uuid } from "../ipc/types";

export interface SessionScopeView {
  items: SessionConfig[];
  currentId: Uuid | null;
  connectionState: Map<Uuid, string>;
}

/**
 * 解析快速命令的目标会话。
 *
 * 只发往**已连接**的会话：往未连接的会话发等于静默失败。
 */
export function resolveQuickCommandTargets(
  command: QuickCommand,
  view: SessionScopeView,
): Uuid[] {
  const connected = view.items
    .filter((item) => view.connectionState.get(item.id) === "connected")
    .map((item) => item.id);

  if (command.scope === "CurrentSession") {
    return view.currentId && connected.includes(view.currentId) ? [view.currentId] : [];
  }
  if (command.scope === "AllSessions") return connected;
  return command.scope.SelectedSessions.filter((id) => connected.includes(id));
}
