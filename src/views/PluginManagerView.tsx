// PluginManager - 插件管理
// 设计规范 §8.x 插件

export function PluginManagerView() {
  return (
    <div style={{ padding: 16 }}>
      <h2 style={{ fontSize: 16, fontWeight: 600, marginBottom: 16 }}>插件</h2>
      <p style={{ color: "var(--text-secondary)", fontSize: 13 }}>
        加载 WASM 插件扩展功能。后续接入 ScanPlugins + PluginsSnapshot。
      </p>
    </div>
  );
}