/**
 * 侧栏列表不许用 el-table —— 窄容器里表格会塌列。
 *
 * 侧栏只有 200~360px 宽（见 src/utils/workspaceLayout.ts）。el-table 按列宽
 * 排版，放不下时有两种塌法：
 *
 * ① **弹性列宽度塌成 0**：没写 `width` / `min-width` 的列，Element Plus 在
 *    表格放不下时把 `realWidth` 设成 `Number(column.minWidth)`，而
 *    `min-width` 的默认值是空字符串 → `Number("")` = 0（见
 *    node_modules/element-plus/es/components/table/src/table-layout.mjs：
 *    `flexColumns.forEach(column => { column.realWidth = Number(column.minWidth) })`）
 *    → 整列内容不可见。真机实测：密钥面板「名称」列有值（AX 树能读到），
 *    屏幕上却是空白，「指纹」被挤成一列换行八行。
 * ② **最右侧操作列落在视口外**：固定列宽合计超过容器宽度时，表格横向溢出，
 *    而侧栏没有横向滚动条 → 用户看不到也点不到「执行 / 删除 / 加载 / 卸载 /
 *    启用」。真机实测：快速命令、触发器、插件三个面板的操作列整列不可见。
 *
 * 隧道面板早就改成了卡片列表（TunnelPanel 的 `.tunnel-list`），这里把同样的
 * 约束扩展到其余四个侧栏面板，并守住「弹性列必须显式写宽度」这条通用规则。
 */
import { describe, expect, it, vi } from "vitest";

const { readFileSync, readdirSync } = await vi.importActual<{
  readFileSync(path: string, encoding: "utf8"): string;
  readdirSync(
    path: string,
    options: { withFileTypes: boolean | string },
  ): Array<{ name: string; isDirectory(): boolean; isFile(): boolean }>;
}>("node:fs");
const { join } = await vi.importActual<{ join(...paths: string[]): string }>("node:path");

function walkSources(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...walkSources(full));
    else if (entry.name.endsWith(".vue")) out.push(full);
  }
  return out;
}

/** 模板里每个 `<el-table-column ...>` 标签的原文 */
function tableColumnTags(): { file: string; tag: string }[] {
  const out: { file: string; tag: string }[] = [];
  for (const file of walkSources("src")) {
    const body = readFileSync(file, "utf8");
    for (const m of body.matchAll(/<el-table-column\b[\s\S]*?>/g)) {
      out.push({ file, tag: m[0] });
    }
  }
  return out;
}

/**
 * 侧栏面板清单。
 *
 * 这些组件由 SidePanel.vue 渲染在 200~360px 的侧栏里（默认 280px），
 * 表格在这里必然溢出。
 */
const SIDE_PANEL_COMPONENTS = [
  "src/components/KeyManagerPanel.vue",
  "src/components/QuickCommandPanel.vue",
  "src/components/TriggerEditor.vue",
  "src/components/PluginPanel.vue",
];

/** 侧栏卡片布局用到的共享样式类（定义在 src/styles/global.css） */
const CARD_STYLE_CLASSES = [
  ".rs-cards",
  ".rs-card",
  ".rs-card-head",
  ".rs-card-title",
  ".rs-card-body",
  ".rs-card-line",
  ".rs-card-value",
  ".rs-card-actions",
];

describe("侧栏列表布局契约", () => {
  it("每个 el-table-column 都显式声明 width 或 min-width（否则窄容器里整列不可见）", () => {
    const missing = tableColumnTags()
      .filter(({ tag }) => !/\b(min-)?width\s*=/.test(tag))
      .map(({ file, tag }) => `${file}: ${tag.replace(/\s+/g, " ")}`);
    expect(missing).toEqual([]);
  });

  it("侧栏面板不使用 el-table（表格在 200~360px 的侧栏里必然溢出）", () => {
    const offenders: string[] = [];
    for (const file of SIDE_PANEL_COMPONENTS) {
      const body = readFileSync(file, "utf8");
      if (/<el-table\b/.test(body)) offenders.push(file);
    }
    expect(offenders).toEqual([]);
  });

  it("共享的侧栏卡片样式存在（否则卡片会退化成没有样式的裸文本）", () => {
    const globalCss = readFileSync("src/styles/global.css", "utf8");
    const missing = CARD_STYLE_CLASSES.filter(cls => !globalCss.includes(cls));
    expect(missing).toEqual([]);
  });
});
