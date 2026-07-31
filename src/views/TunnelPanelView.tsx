// TunnelPanel - 端口转发面板
// 设计规范 §8.x 隧道管理

export function TunnelPanelView() {
  return (
    <div style={{ padding: 16 }}>
      <h2 style={{ fontSize: 16, fontWeight: 600, marginBottom: 16 }}>端口转发</h2>
      <p style={{ color: "var(--text-secondary)", fontSize: 13 }}>
        支持本地转发 / 远程转发 / 动态 SOCKS。后续接入 CreateTunnel + TunnelsSnapshot。
      </p>
    </div>
  );
}