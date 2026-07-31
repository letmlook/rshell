// Toolbar - 主工具栏,40px 高
// 设计规范 §8.2.1.1
// 提供连接、文件、快速命令、隧道、侧栏切换、设置入口

interface ToolbarProps {
  onToggleSidebar: () => void;
  onOpenSettings: () => void;
  onConnect: () => void;
  onOpenFileManager: () => void;
  onOpenQuickCommands: () => void;
  onOpenTunnelPanel: () => void;
}

export function Toolbar({
  onToggleSidebar,
  onOpenSettings,
  onConnect,
  onOpenFileManager,
  onOpenQuickCommands,
  onOpenTunnelPanel,
}: ToolbarProps) {
  return (
    <div className="toolbar">
      <button className="toolbar-btn primary" onClick={onConnect}>
        <span>🔗</span>
        <span>连接</span>
      </button>
      <button className="toolbar-btn" onClick={onOpenFileManager}>
        <span>📁</span>
        <span>文件</span>
      </button>
      <button className="toolbar-btn" onClick={onOpenQuickCommands}>
        <span>⚡</span>
        <span>快速命令</span>
      </button>
      <button className="toolbar-btn" onClick={onOpenTunnelPanel}>
        <span>🔧</span>
        <span>隧道</span>
      </button>
      <div className="toolbar-spacer" />
      <button className="toolbar-btn" onClick={onToggleSidebar}>
        <span>☰</span>
      </button>
      <button className="toolbar-btn" onClick={onOpenSettings}>
        <span>⚙</span>
        <span>设置</span>
      </button>
    </div>
  );
}