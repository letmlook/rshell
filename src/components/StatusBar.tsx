// StatusBar - 底部状态栏,28px 高
// 设计规范 §8.2.1.1
// 显示连接状态、加密标志、host:port、协议、上下行速率、编码、Terminal 类型

import type { ConnectionState } from "../ipc/types";

interface StatusBarProps {
  connectionState: ConnectionState;
  host?: string;
  port?: number;
  protocol?: string;
  uploadSpeed?: string;
  downloadSpeed?: string;
  encoding?: string;
  terminal?: string;
}

export function StatusBar({
  connectionState,
  host,
  port,
  protocol = "SSH2",
  uploadSpeed = "↑ 0 B/s",
  downloadSpeed = "↓ 0 B/s",
  encoding = "UTF-8",
  terminal = "xterm-256color",
}: StatusBarProps) {
  const isConnected = connectionState === "Connected";
  return (
    <div className="status-bar">
      <div className="status-item">
        <span
          className={`status-indicator ${isConnected ? "connected" : "disconnected"}`}
        />
        <span>{isConnected ? "已连接" : "未连接"}</span>
      </div>
      {host && port && (
        <div className="status-item">
          <span>🔒</span>
          <span className="mono">
            {host}:{port}
          </span>
        </div>
      )}
      <div className="status-item">
        <span>{protocol}</span>
      </div>
      <div className="status-item">
        <span>{uploadSpeed}</span>
      </div>
      <div className="status-item">
        <span>{downloadSpeed}</span>
      </div>
      <div className="status-spacer" />
      <div className="status-item">
        <span>{encoding}</span>
      </div>
      <div className="status-item">
        <span>{terminal}</span>
      </div>
    </div>
  );
}