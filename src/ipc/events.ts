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

/**
 * 派发到多个 handler — 按 event 类型分支。
 * 用于把不同事件路由到不同 store。
 */
export type EventDispatcher = {
  onConnectionStateChanged?: (session_id: string, state: string, info: unknown) => void;
  onTerminalTitleChanged?: (session_id: string, title: string) => void;
  onTransferProgress?: (task_id: string, bytes: number, total: number, speed_bps: number) => void;
  onTransferCompleted?: (task_id: string) => void;
  onTransferFailed?: (task_id: string, error: string) => void;
  onTransferTaskAdded?: (task_id: string, filename: string, direction: string) => void;
  onHostKeyMismatch?: (data: {
    decision_id: string;
    host: string;
    port: number;
    key_type: string;
    expected: string;
    received: string;
    public_key_blob: string;
  }) => void;
};

export function makeDispatcher(dispatcher: EventDispatcher): EventHandler {
  return (event) => {
    const key = Object.keys(event)[0] as keyof AppEvent;
    switch (key) {
      case "ConnectionStateChanged": {
        const e = (event as Extract<AppEvent, { ConnectionStateChanged: unknown }>).ConnectionStateChanged;
        dispatcher.onConnectionStateChanged?.(e.session_id, e.state, e.info);
        break;
      }
      case "TerminalTitleChanged": {
        const e = (event as Extract<AppEvent, { TerminalTitleChanged: unknown }>).TerminalTitleChanged;
        dispatcher.onTerminalTitleChanged?.(e.session_id, e.title);
        break;
      }
      case "TransferProgress": {
        const e = (event as Extract<AppEvent, { TransferProgress: unknown }>).TransferProgress;
        dispatcher.onTransferProgress?.(e.task_id, e.bytes, e.total, e.speed_bps);
        break;
      }
      case "TransferCompleted": {
        const e = (event as Extract<AppEvent, { TransferCompleted: unknown }>).TransferCompleted;
        dispatcher.onTransferCompleted?.(e.task_id);
        break;
      }
      case "TransferFailed": {
        const e = (event as Extract<AppEvent, { TransferFailed: unknown }>).TransferFailed;
        dispatcher.onTransferFailed?.(e.task_id, e.error);
        break;
      }
      case "TransferTaskAdded": {
        const e = (event as Extract<AppEvent, { TransferTaskAdded: unknown }>).TransferTaskAdded;
        dispatcher.onTransferTaskAdded?.(e.task_id, e.filename, e.direction);
        break;
      }
      case "HostKeyMismatch": {
        const e = (event as Extract<AppEvent, { HostKeyMismatch: unknown }>).HostKeyMismatch;
        dispatcher.onHostKeyMismatch?.(e);
        break;
      }
      default:
        // 未注册的事件:留给后续阶段
        break;
    }
  };
}