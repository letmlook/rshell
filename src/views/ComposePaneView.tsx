// ComposePane - 撰写窗格 (同时给多个会话发文本)
// 设计规范 §8.x 撰写窗格

export function ComposePaneView() {
  return (
    <div style={{ padding: 16, display: "flex", flexDirection: "column", gap: 12 }}>
      <h2 style={{ fontSize: 16, fontWeight: 600 }}>撰写窗格</h2>
      <textarea
        className="form-input"
        rows={6}
        placeholder="输入文本,可同时发送到多个会话..."
        style={{ height: "auto", padding: 8, fontFamily: "var(--font-mono)", resize: "vertical" }}
      />
      <div style={{ display: "flex", gap: 8 }}>
        <button className="btn">发送</button>
        <button className="btn">清空</button>
      </div>
    </div>
  );
}