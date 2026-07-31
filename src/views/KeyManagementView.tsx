// KeyManagement - SSH 密钥管理
// 设计规范 §8.x 密钥

export function KeyManagementView() {
  return (
    <div style={{ padding: 16 }}>
      <h2 style={{ fontSize: 16, fontWeight: 600, marginBottom: 16 }}>SSH 密钥</h2>
      <p style={{ color: "var(--text-secondary)", fontSize: 13 }}>
        生成 / 导入 / 导出密钥对。后续接入 GenerateSshKey / ImportPrivateKey / KeysSnapshot。
      </p>
    </div>
  );
}