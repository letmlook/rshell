// ThemeSettings - 主题与配色方案
// 设计规范 §8.x 主题

import { useThemeStore } from "../store/useThemeStore";

export function ThemeSettingsView() {
  const mode = useThemeStore((s) => s.mode);
  const setMode = useThemeStore((s) => s.setMode);

  return (
    <div style={{ padding: 16, display: "flex", flexDirection: "column", gap: 16 }}>
      <h2 style={{ fontSize: 16, fontWeight: 600 }}>主题</h2>
      <div style={{ display: "flex", gap: 8 }}>
        <button
          className={`btn ${mode === "dark" ? "primary" : ""}`}
          onClick={() => setMode("dark")}
        >
          深色
        </button>
        <button
          className={`btn ${mode === "light" ? "primary" : ""}`}
          onClick={() => setMode("light")}
        >
          浅色
        </button>
      </div>
      <p style={{ color: "var(--text-secondary)", fontSize: 13 }}>
        配色方案列表由后端 ThemesSnapshot 推送。后续接入。
      </p>
    </div>
  );
}