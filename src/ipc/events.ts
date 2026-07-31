// src/ipc/events.ts - 订阅 AppEvent 流
//
// 后端通过 app.emit_to("main", "rshell://event", payload) 推送事件,
// payload 是 AppEvent 的 JSON 形式(就是 types.ts 里定义的 discriminated union)。
// 前端用 listen() 订阅并路由到 zustand store。

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
    // Tauri 已经把 payload 解析成对象了,直接 cast
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
  onTerminalOutput?: (session_id: string, data: number[]) => void;
  onTerminalBufferUpdated?: (session_id: string, snapshot: unknown) => void;
  onTerminalTitleChanged?: (session_id: string, title: string) => void;
  onSessionsSnapshot?: (sessions: unknown[]) => void;
  onTunnelsSnapshot?: (tunnels: unknown[]) => void;
  onKeysSnapshot?: (keys: unknown[]) => void;
  onPluginsSnapshot?: (plugins: unknown[]) => void;
  onThemesSnapshot?: (snapshot: {
    current_theme: string;
    current_scheme: string;
    available_themes: string[];
    available_schemes: string[];
  }) => void;
  onTransferProgress?: (task_id: string, bytes: number, total: number, speed_bps: number) => void;
  onTransferCompleted?: (task_id: string) => void;
  onTransferFailed?: (task_id: string, error: string) => void;
  onTransferTaskAdded?: (task_id: string, filename: string, direction: string) => void;
  onRemoteDirListed?: (session_id: string, path: string, entries: unknown[]) => void;
  onHostKeyMismatch?: (data: {
    decision_id: string;
    host: string;
    port: number;
    key_type: string;
    expected: string;
    received: string;
    public_key_blob: string;
  }) => void;
  onClipboardCopy?: (text: string) => void;
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
      case "TerminalOutput": {
        const e = (event as Extract<AppEvent, { TerminalOutput: unknown }>).TerminalOutput;
        dispatcher.onTerminalOutput?.(e.session_id, e.data);
        break;
      }
      case "TerminalBufferUpdated": {
        const e = (event as Extract<AppEvent, { TerminalBufferUpdated: unknown }>).TerminalBufferUpdated;
        dispatcher.onTerminalBufferUpdated?.(e.session_id, e.snapshot);
        break;
      }
      case "TerminalTitleChanged": {
        const e = (event as Extract<AppEvent, { TerminalTitleChanged: unknown }>).TerminalTitleChanged;
        dispatcher.onTerminalTitleChanged?.(e.session_id, e.title);
        break;
      }
      case "SessionsSnapshot": {
        const e = (event as Extract<AppEvent, { SessionsSnapshot: unknown }>).SessionsSnapshot;
        dispatcher.onSessionsSnapshot?.(e.sessions);
        break;
      }
      case "TunnelsSnapshot": {
        const e = (event as Extract<AppEvent, { TunnelsSnapshot: unknown }>).TunnelsSnapshot;
        dispatcher.onTunnelsSnapshot?.(e.tunnels);
        break;
      }
      case "KeysSnapshot": {
        const e = (event as Extract<AppEvent, { KeysSnapshot: unknown }>).KeysSnapshot;
        dispatcher.onKeysSnapshot?.(e.keys);
        break;
      }
      case "PluginsSnapshot": {
        const e = (event as Extract<AppEvent, { PluginsSnapshot: unknown }>).PluginsSnapshot;
        dispatcher.onPluginsSnapshot?.(e.plugins);
        break;
      }
      case "ThemesSnapshot": {
        const e = (event as Extract<AppEvent, { ThemesSnapshot: unknown }>).ThemesSnapshot;
        dispatcher.onThemesSnapshot?.(e);
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
      case "RemoteDirListed": {
        const e = (event as Extract<AppEvent, { RemoteDirListed: unknown }>).RemoteDirListed;
        dispatcher.onRemoteDirListed?.(e.session_id, e.path, e.entries);
        break;
      }
      case "HostKeyMismatch": {
        const e = (event as Extract<AppEvent, { HostKeyMismatch: unknown }>).HostKeyMismatch;
        dispatcher.onHostKeyMismatch?.(e);
        break;
      }
      case "ClipboardCopy": {
        const e = (event as Extract<AppEvent, { ClipboardCopy: unknown }>).ClipboardCopy;
        dispatcher.onClipboardCopy?.(e.text);
        break;
      }
      default:
        // 未注册的事件:留给后续阶段
        break;
    }
  };
}