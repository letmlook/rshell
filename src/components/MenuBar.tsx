// MenuBar - 顶部菜单栏,32px 高
// 设计规范 §8.2.1.1

const MENU_ITEMS = ["文件", "编辑", "查看", "会话", "工具", "帮助"];

export function MenuBar() {
  return (
    <div className="menu-bar">
      {MENU_ITEMS.map((label) => (
        <div key={label} className="menu-item" role="button" tabIndex={0}>
          {label}
        </div>
      ))}
    </div>
  );
}