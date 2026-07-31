// SessionTree - 会话管理器侧边栏
// 设计规范 §8.2.2
// 结构: 标题栏 + 搜索框 + 文件夹分组树 + 底部按钮
//
// 数据来自 useAppStore.sessions;后续会通过订阅 SessionsSnapshot 事件自动同步。

import { useState, useMemo } from "react";
import { useAppStore } from "../store/useAppStore";
import type { ConnectionState, SessionConfig, Uuid } from "../ipc/types";

const PROTOCOL_ICON: Record<string, string> = {
  SSH: "🔗",
  Telnet: "🔗",
  Serial: "🔌",
  RDP: "🖥",
};

function statusClass(state: ConnectionState | undefined): string {
  switch (state) {
    case "Connected":
      return "connected";
    case "Connecting":
    case "Authenticating":
      return "connecting";
    case "Disconnected":
      return "disconnected";
    case "Disconnecting":
      return "connecting";
    default:
      return "disconnected";
  }
}

/** 按 folder_id 把会话分组,返回文件夹 id -> 会话数组的映射,以及顶级无文件夹会话 */
function groupByFolder(
  sessions: SessionConfig[],
): { folders: Map<string | null, SessionConfig[]> } {
  const folders = new Map<string | null, SessionConfig[]>();
  for (const s of sessions) {
    const key = s.folder_id;
    const list = folders.get(key) ?? [];
    list.push(s);
    folders.set(key, list);
  }
  return { folders };
}

export function SessionTree() {
  const sessions = useAppStore((s) => s.sessions);
  const sessionStates = useAppStore((s) => s.sessionStates);
  const sidebarCollapsed = useAppStore((s) => s.sidebarCollapsed);

  const [search, setSearch] = useState("");
  const [expandedFolders, setExpandedFolders] = useState<Set<string>>(
    () => new Set(["root"]),
  );

  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return sessions;
    return sessions.filter(
      (s) =>
        s.name.toLowerCase().includes(q) ||
        s.host.toLowerCase().includes(q) ||
        s.protocol.toLowerCase().includes(q),
    );
  }, [sessions, search]);

  const { folders } = useMemo(() => groupByFolder(filtered), [filtered]);

  const toggleFolder = (id: string) => {
    setExpandedFolders((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  if (sidebarCollapsed) return null;

  return (
    <aside className="sidebar">
      <div className="sidebar-header">
        <span>会话管理器</span>
        <span style={{ cursor: "pointer" }} title="会话管理器设置">⚙</span>
      </div>
      <div className="sidebar-search">
        <input
          type="text"
          placeholder="🔍 搜索会话..."
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>
      <div className="session-tree">
        {folders.size === 0 && (
          <div style={{ padding: 8, color: "var(--text-disabled)", fontSize: 12 }}>
            暂无会话
          </div>
        )}
        {Array.from(folders.entries()).map(([folderId, items]) => {
          // folder_id === null 视为顶级,展开
          const key = folderId ?? "root";
          const expanded = expandedFolders.has(key);
          return (
            <FolderSection
              key={key}
              folderId={folderId}
              name={folderId === null ? "默认分组" : `文件夹 ${folderId.slice(0, 6)}`}
              sessions={items}
              expanded={expanded}
              onToggle={() => toggleFolder(key)}
              sessionStates={sessionStates}
            />
          );
        })}
      </div>
      <div className="sidebar-footer">
        <button onClick={() => alert("新建会话 — 后续接入 openCreateSessionDialog")}>
          ＋ 新建会话
        </button>
        <button onClick={() => alert("新建文件夹 — 后续接入")}>
          ＋ 新建文件夹
        </button>
      </div>
    </aside>
  );
}

interface FolderSectionProps {
  folderId: Uuid | null;
  name: string;
  sessions: SessionConfig[];
  expanded: boolean;
  onToggle: () => void;
  sessionStates: Record<Uuid, ConnectionState>;
}

function FolderSection({
  name,
  sessions,
  expanded,
  onToggle,
  sessionStates,
}: FolderSectionProps) {
  return (
    <>
      <div
        className="session-item session-folder"
        onClick={onToggle}
        role="button"
        tabIndex={0}
      >
        <span className="session-icon">{expanded ? "📂" : "📁"}</span>
        <span>{name}</span>
        <span style={{ marginLeft: "auto", fontSize: 11, color: "var(--text-secondary)" }}>
          {sessions.length}
        </span>
      </div>
      {expanded && (
        <div style={{ paddingLeft: 16 }}>
          {sessions.map((session) => (
            <SessionRow
              key={session.id}
              session={session}
              state={sessionStates[session.id]}
            />
          ))}
        </div>
      )}
    </>
  );
}

interface SessionRowProps {
  session: SessionConfig;
  state?: ConnectionState;
}

function SessionRow({ session, state }: SessionRowProps) {
  const [selected, setSelected] = useState(false);

  return (
    <div
      className={`session-item ${selected ? "selected" : ""}`}
      onClick={() => setSelected(true)}
      role="button"
      tabIndex={0}
      title={`${session.protocol} ${session.host}:${session.port}`}
    >
      <span className="session-icon">{PROTOCOL_ICON[session.protocol] ?? "🔗"}</span>
      <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {session.name}
      </span>
      <span className={`status-dot ${statusClass(state)}`} />
    </div>
  );
}