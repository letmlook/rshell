// src/ipc/events.ts - 订阅 AppEvent 流
//
// 后端通过 app.emit_to("main", "rshell://event", payload) 推送事件,
// payload 是 AppEvent 的 JSON 形式(就是 types.ts 里定义的 discriminated union)。
// 前端用 listen() 订阅并路由到 store。
//
// 切片 2.2 删除（设计 §3.3）：
//   - 7 个 *Snapshot 事件（Sessions / Keys / Tunnels / Plugins / Themes / PendingTunnels / RemoteDirListed）
//   - TerminalBufferUpdated 与 TerminalOutput（设计 §2.2：alacritty 净删除 → xterm 全接管）
//   - ClipboardCopy（设计 §5 上移剪贴板到前端 xterm 自持选区）
//
// R2-10（2026-10）：删除 makeDispatcher / EventDispatcher —— 全仓库 0 调用方的
// 死代码，且其 case 引用了同轮删除的 0 发布点事件（TerminalTitleChanged、
// TransferTaskAdded）。事件消费方直接在各 store / 组件的 subscribeAppEvents
// 回调里按 `"<Variant>" in event` 分支（见 sessions.ts、App.vue、TunnelPanel.vue）。

import { listen, UnlistenFn } from "@tauri-apps/api/event";
import type { AppEvent } from "./types";

export const EVENT_CHANNEL = "rshell://event";

export type EventHandler = (event: AppEvent) => void;

/**
 * 订阅后端事件流。返回 unlisten 函数。
 *
 * 用法:
 *   const unlisten = await subscribeAppEvents((event) => {
 *     if ("ConnectionStateChanged" in event) { ... }
 *   });
 *   // later: unlisten();
 */
export async function subscribeAppEvents(handler: EventHandler): Promise<UnlistenFn> {
  return listen<unknown>(EVENT_CHANNEL, (msg) => {
    const event = msg.payload as AppEvent;
    try {
      handler(event);
    } catch (err) {
      console.error("[rshell] event handler error:", err, event);
    }
  });
}
