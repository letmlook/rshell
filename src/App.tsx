// App - 主应用根组件
// 还原 docs/ui-design-preview.html 完整结构:
//   MenuBar(32) → Toolbar(40) → Main(sidebar 240 + workspace) → StatusBar(28)

import { useState, useEffect } from "react";
import { MenuBar } from "./components/MenuBar";
import { Toolbar } from "./components/Toolbar";
import { StatusBar } from "./components/StatusBar";
import { SessionTree } from "./views/SessionTree";
import { TabBar } from "./views/TabBar";
import { Workspace } from "./views/TerminalPanel";
import { SettingsModal } from "./views/SettingsModal";
import { useAppStore } from "./store/useAppStore";
import { useThemeStore } from "./store/useThemeStore";
import { listSessions, listThemes } from "./ipc/client";
import { subscribeAppEvents, makeDispatcher } from "./ipc/events";

export default function App() {
  const toggleSidebar = useAppStore((s) => s.toggleSidebar);
  const setSessions = useAppStore((s) => s.setSessions);
  const setSessionState = useAppStore((s) => s.setSessionState);
  const toggleTheme = useThemeStore((s) => s.toggle);
  const themeMode = useThemeStore((s) => s.mode);

  const [settingsOpen, setSettingsOpen] = useState(false);

  // 启动时拉取初始数据 + 订阅事件
  useEffect(() => {
    let unlisten: (() => void) | undefined;

    (async () => {
      try {
        // 初始数据
        const sessions = await listSessions();
        if (sessions?.sessions) setSessions(sessions.sessions as never);
        await listThemes().catch(() => {});
      } catch (err) {
        // 后端未启动时静默(开发模式)
        console.warn("[rshell] 初始数据拉取失败(后端未启动?):", err);
      }

      // 订阅事件
      unlisten = await subscribeAppEvents(
        makeDispatcher({
          onSessionsSnapshot: (sessions) => {
            setSessions(sessions as never);
          },
          onConnectionStateChanged: (session_id, state) => {
            setSessionState(session_id, state as never);
          },
          onThemesSnapshot: (snapshot) => {
            // 应用主题模式
            if (snapshot.current_theme?.toLowerCase().includes("light")) {
              document.documentElement.setAttribute("data-theme", "light");
            } else {
              document.documentElement.setAttribute("data-theme", "dark");
            }
          },
        }),
      );
    })();

    return () => {
      unlisten?.();
    };
  }, [setSessions, setSessionState]);

  // Esc 关闭设置
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setSettingsOpen(false);
      // Ctrl+Shift+S 切换侧边栏
      if (e.ctrlKey && e.shiftKey && (e.key === "S" || e.key === "s")) {
        e.preventDefault();
        toggleSidebar();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggleSidebar]);

  return (
    <div className="app-container">
      <MenuBar />
      <Toolbar
        onToggleSidebar={toggleSidebar}
        onOpenSettings={() => setSettingsOpen(true)}
        onConnect={() => alert("连接流程 — 后续接入")}
        onOpenFileManager={() => alert("文件管理器 — 后续接入")}
        onOpenQuickCommands={() => alert("快速命令 — 后续接入")}
        onOpenTunnelPanel={() => alert("隧道 — 后续接入")}
      />
      <div className="main-content">
        <SessionTree />
        <div className="workspace">
          <TabBar />
          <div className="tab-content">
            <div className="tab-panel active">
              <Workspace />
            </div>
          </div>
        </div>
      </div>
      <StatusBar
        connectionState="Disconnected"
        protocol="SSH2"
        encoding="UTF-8"
        terminal="xterm-256color"
      />
      <button
        onClick={toggleTheme}
        title="切换主题"
        style={{
          position: "fixed",
          top: 50,
          right: 20,
          width: 40,
          height: 40,
          borderRadius: "50%",
          background: "var(--bg-surface)",
          color: "var(--text-primary)",
          border: "1px solid var(--border)",
          cursor: "pointer",
          fontSize: 18,
          zIndex: 100,
          boxShadow: "var(--shadow-md)",
        }}
      >
        {themeMode === "dark" ? "☀" : "🌙"}
      </button>
      <SettingsModal open={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </div>
  );
}