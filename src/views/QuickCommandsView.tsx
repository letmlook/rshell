// QuickCommands - 快速命令面板
// 设计规范 §8.x 快速命令

export function QuickCommandsView() {
  return (
    <div style={{ padding: 16 }}>
      <h2 style={{ fontSize: 16, fontWeight: 600, marginBottom: 16 }}>快速命令</h2>
      <p style={{ color: "var(--text-secondary)", fontSize: 13 }}>
        快速命令可绑定热键、批量发送到多个会话。后续接入 ExecuteQuickCommand + 列表。
      </p>
    </div>
  );
}