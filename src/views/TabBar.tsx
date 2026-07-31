// TabBar - 工作区标签栏
// 设计规范 §8.2.3.1
// 36px 高,活动标签底部 2px 品牌色,min-width 120px / max-width 200px,
// 关闭按钮 hover 红圆点,右侧独立 "+" 按钮。

import { useAppStore } from "../store/useAppStore";

export function TabBar() {
  const tabs = useAppStore((s) => s.tabs);
  const activeTabIndex = useAppStore((s) => s.activeTabIndex);
  const setActiveTab = useAppStore((s) => s.setActiveTab);
  const closeTab = useAppStore((s) => s.closeTab);

  if (tabs.length === 0) {
    return (
      <div className="tab-bar">
        <div
          style={{
            padding: "0 12px",
            color: "var(--text-disabled)",
            fontSize: 12,
            alignSelf: "center",
          }}
        >
          无打开的标签页
        </div>
      </div>
    );
  }

  return (
    <div className="tab-bar">
      {tabs.map((tab, idx) => {
        const isActive = activeTabIndex === idx;
        return (
          <div
            key={tab.id}
            className={`tab ${isActive ? "active" : ""}`}
            onClick={() => setActiveTab(idx)}
            role="tab"
            aria-selected={isActive}
            tabIndex={0}
          >
            <span>{tab.type === "sftp" ? "📁" : "🔗"}</span>
            <span className="tab-title">{tab.title}</span>
            {tab.connected && (
              <span
                style={{
                  width: 6,
                  height: 6,
                  borderRadius: "50%",
                  background: "var(--success)",
                  flexShrink: 0,
                }}
              />
            )}
            <span
              className="tab-close"
              onClick={(e) => {
                e.stopPropagation();
                closeTab(tab.id);
              }}
              role="button"
              aria-label={`关闭 ${tab.title}`}
              tabIndex={0}
            >
              ×
            </span>
          </div>
        );
      })}
      <button className="tab-add" title="新建标签" aria-label="新建标签">
        +
      </button>
    </div>
  );
}