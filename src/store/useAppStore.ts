// src/store/useAppStore.ts - 主应用状态
// 持有会话/标签/侧边栏显示/激活面板。

import { create } from "zustand";
import type { ConnectionState, SessionConfig, Uuid } from "../ipc/types";

export interface TabInfo {
  id: Uuid;
  title: string;
  type: "terminal" | "sftp";
  connected: boolean;
  sessionId: Uuid;
}

export type PanelKind =
  | "terminal"
  | "fileManager"
  | "quickCommands"
  | "tunnels"
  | "keys"
  | "plugins"
  | "theme"
  | "compose";

interface AppState {
  // 会话
  sessions: SessionConfig[];
  setSessions: (sessions: SessionConfig[]) => void;
  upsertSession: (session: SessionConfig) => void;
  removeSession: (id: Uuid) => void;

  // 会话连接状态映射
  sessionStates: Record<Uuid, ConnectionState>;
  setSessionState: (sessionId: Uuid, state: ConnectionState) => void;

  // 标签栏
  tabs: TabInfo[];
  activeTabIndex: number | null;
  openTab: (sessionId: Uuid, type?: TabInfo["type"], title?: string) => void;
  closeTab: (id: Uuid) => void;
  setActiveTab: (index: number) => void;
  setTabTitle: (id: Uuid, title: string) => void;
  setTabConnected: (id: Uuid, connected: boolean) => void;

  // 当前激活终端 session
  activeSessionId: Uuid | null;
  setActiveSessionId: (id: Uuid | null) => void;

  // 侧边栏
  sidebarCollapsed: boolean;
  toggleSidebar: () => void;
  setSidebarCollapsed: (collapsed: boolean) => void;

  // 当前主面板
  activePanel: PanelKind;
  setActivePanel: (panel: PanelKind) => void;
}

export const useAppStore = create<AppState>((set, get) => ({
  // 会话
  sessions: [],
  setSessions: (sessions) => set({ sessions }),
  upsertSession: (session) =>
    set((s) => {
      const idx = s.sessions.findIndex((x) => x.id === session.id);
      if (idx >= 0) {
        const next = [...s.sessions];
        next[idx] = session;
        return { sessions: next };
      }
      return { sessions: [...s.sessions, session] };
    }),
  removeSession: (id) =>
    set((s) => ({
      sessions: s.sessions.filter((x) => x.id !== id),
      sessionStates: Object.fromEntries(
        Object.entries(s.sessionStates).filter(([k]) => k !== id),
      ),
    })),

  // 连接状态
  sessionStates: {},
  setSessionState: (sessionId, state) =>
    set((s) => ({
      sessionStates: { ...s.sessionStates, [sessionId]: state },
      tabs: s.tabs.map((t) =>
        t.sessionId === sessionId ? { ...t, connected: state === "Connected" } : t,
      ),
    })),

  // 标签栏
  tabs: [],
  activeTabIndex: null,
  openTab: (sessionId, type = "terminal", title) => {
    const { tabs, sessions } = get();
    const existing = tabs.findIndex((t) => t.sessionId === sessionId && t.type === type);
    if (existing >= 0) {
      set({ activeTabIndex: existing });
      return;
    }
    const sessionName = sessions.find((s) => s.id === sessionId)?.name || sessionId.slice(0, 8);
    const newTab: TabInfo = {
      id: sessionId, // 终端/SFTP tab id = session id(简化)
      sessionId,
      title: title || sessionName,
      type,
      connected: false,
    };
    set({
      tabs: [...tabs, newTab],
      activeTabIndex: tabs.length,
    });
  },
  closeTab: (id) =>
    set((s) => {
      const idx = s.tabs.findIndex((t) => t.id === id);
      if (idx < 0) return s;
      const tabs = s.tabs.filter((t) => t.id !== id);
      let activeTabIndex = s.activeTabIndex;
      if (activeTabIndex !== null) {
        if (activeTabIndex === idx) {
          activeTabIndex = tabs.length > 0 ? Math.min(idx, tabs.length - 1) : null;
        } else if (activeTabIndex > idx) {
          activeTabIndex = activeTabIndex - 1;
        }
      }
      return { tabs, activeTabIndex };
    }),
  setActiveTab: (index) => set({ activeTabIndex: index }),
  setTabTitle: (id, title) =>
    set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, title } : t)) })),
  setTabConnected: (id, connected) =>
    set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, connected } : t)) })),

  // 激活终端
  activeSessionId: null,
  setActiveSessionId: (id) => set({ activeSessionId: id }),

  // 侧边栏
  sidebarCollapsed: false,
  toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
  setSidebarCollapsed: (collapsed) => set({ sidebarCollapsed: collapsed }),

  // 主面板
  activePanel: "terminal",
  setActivePanel: (panel) => set({ activePanel: panel }),
}));