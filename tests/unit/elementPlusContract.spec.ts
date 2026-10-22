/**
 * Element Plus 组件接入契约。
 *
 * **为什么要有这条测试**：本仓库为了控制包体，逐个组件从 `element-plus/es/...`
 * 引入并单独注册（见 `src/element-plus.ts`），样式也逐个 `@import`
 * （见 `src/styles/element-plus.css`）。两处一旦漏了同一个组件，
 * `<el-xxx>` 就会退化成**未知自定义元素**：
 * - 渲染成没有样式的纯文本；
 * - 事件与 v-model 全部失效 —— 按钮点不动、单选框选不了、下拉框选不了。
 *
 * 真实事故：`el-radio-group` / `el-radio-button`（新建连接的「认证方式」、
 * 撰写面板的「目标范围」）从未被注册，界面上只是一串**点不动的文字**；
 * 同时 `theme-chalk/base.css`（Element Plus 的 CSS 变量总表）没被引入，
 * `--el-font-size-base` 等 180 多个变量未定义，组件样式在「计算值阶段」
 * 失效，文字被压成零宽 —— 连选中的项都看不见。
 *
 * 单元测试（jsdom）不加载这些样式表，发现不了「注册漏了」与「变量表漏了」。
 * 这条测试直接比对**模板里实际用到的标签**与**注册/样式清单**，把缺口挡住。
 */
import { describe, expect, it, vi } from "vitest";

// 与 ipcContract.spec.ts 同一写法：用 vi.importActual 拿 node 模块，
// 路径一律相对仓库根（vitest 的 cwd 就是仓库根）—— 不引入 @types/node。
const { readFileSync, readdirSync } = await vi.importActual<{
  readFileSync(path: string, encoding: "utf8"): string;
  readdirSync(
    path: string,
    options: { withFileTypes: true },
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

/** 模板里实际用到的所有 `<el-xxx>` 标签 */
function usedElementTags(): Set<string> {
  const tags = new Set<string>();
  for (const file of walkSources("src")) {
    const body = readFileSync(file, "utf8");
    for (const m of body.matchAll(/<el-([a-z-]+)/g)) tags.add(`el-${m[1]}`);
  }
  return tags;
}

/**
 * 标签 → 注册名。
 *
 * `el-radio-group` / `el-radio-button` 由 `ElRadio` 的 install 一并注册
 * （见 element-plus/es/components/radio/index.mjs），所以三者共用一个注册名。
 */
const TAG_TO_REGISTRATION: Record<string, string> = {
  "el-button": "ElButton",
  "el-button-group": "ElButtonGroup",
  "el-checkbox": "ElCheckbox",
  "el-dialog": "ElDialog",
  "el-dropdown": "ElDropdown",
  "el-dropdown-item": "ElDropdownItem",
  "el-dropdown-menu": "ElDropdownMenu",
  "el-empty": "ElEmpty",
  "el-form": "ElForm",
  "el-form-item": "ElFormItem",
  "el-input": "ElInput",
  "el-input-number": "ElInputNumber",
  "el-option": "ElOption",
  "el-radio": "ElRadio",
  "el-radio-button": "ElRadio",
  "el-radio-group": "ElRadio",
  "el-select": "ElSelect",
  "el-switch": "ElSwitch",
  "el-table": "ElTable",
  "el-table-column": "ElTableColumn",
  "el-tag": "ElTag",
  "el-tooltip": "ElTooltip",
};

/**
 * 明确不引入样式表的组件。
 *
 * `el-tooltip` 的 theme-chalk 样式表是**空文件**（仓库注释里写明了），
 * 引入它没有任何作用。
 */
const STYLE_EXEMPT: Record<string, string> = {
  "el-tooltip": "el-tooltip.css 是空文件，仓库明确不引入",
};

/** 标签 → 样式文件名（`el-radio-group` 与 `el-radio` 共用一份变量） */
const TAG_TO_STYLE: Record<string, string> = {
  "el-button": "el-button.css",
  "el-button-group": "el-button-group.css",
  "el-checkbox": "el-checkbox.css",
  "el-dialog": "el-dialog.css",
  "el-dropdown": "el-dropdown.css",
  "el-dropdown-item": "el-dropdown-item.css",
  "el-dropdown-menu": "el-dropdown-menu.css",
  "el-empty": "el-empty.css",
  "el-form": "el-form.css",
  "el-form-item": "el-form-item.css",
  "el-input": "el-input.css",
  "el-input-number": "el-input-number.css",
  "el-option": "el-option.css",
  "el-radio": "el-radio.css",
  "el-radio-button": "el-radio-button.css",
  "el-radio-group": "el-radio-group.css",
  "el-select": "el-select.css",
  "el-switch": "el-switch.css",
  "el-table": "el-table.css",
  "el-table-column": "el-table-column.css",
  "el-tag": "el-tag.css",
  // el-tooltip 的样式表是空文件，仓库明确不引入（见 element-plus.css 注释）
  "el-tooltip": "",
};

/**
 * 以 **API 形式**调用（而不是写 `<el-xxx>` 标签）的 Element Plus 组件。
 *
 * 这类组件没有模板标签，上面的「标签 → 样式」对账完全看不到它们，
 * 于是漏掉样式表时不会报错、只会**静默不可见**：
 *   - 缺 `el-notification.css` → ElNotification 退化成追加到 <body> 末尾的
 *     普通块级元素（position 不再是 fixed），落到视口之外；
 *   - 缺 `el-message.css` → ElMessage 同样没有定位与配色。
 * 真实事故：触发器「显示通知」动作触发成功（后端日志有 Trigger matched），
 * 但界面上什么也没出现 —— 通知被渲染到了看不见的地方。
 */
const API_COMPONENTS: Record<string, string> = {
  ElMessage: "el-message.css",
  ElNotification: "el-notification.css",
  ElMessageBox: "el-message-box.css",
};

/** src/ 里真正调用了哪些 API 形式组件（`ElMessage.info(...)` 这类） */
function usedApiComponents(): Set<string> {
  const used = new Set<string>();
  for (const file of walkSources("src")) {
    const body = readFileSync(file, "utf8");
    for (const name of Object.keys(API_COMPONENTS)) {
      // 只认「调用点」：`ElXxx.` 开头；导出/导入语句不算调用
      if (new RegExp(`\\b${name}\\s*\\.`).test(body)) used.add(name);
    }
  }
  return used;
}

describe("Element Plus 接入契约", () => {
  const registrationSource = readFileSync("src/element-plus.ts", "utf8");
  const styleSource = readFileSync("src/styles/element-plus.css", "utf8");

  it("模板里用到的每个 el-* 组件都已注册（否则退化成点不动的纯文本）", () => {
    const imported = new Set<string>();
    for (const m of registrationSource.matchAll(
      /import\s+([\s\S]*?)\s+from\s+["'][^"']+["']/g,
    )) {
      const clause = m[1];
      const named = clause.match(/\{([\s\S]*?)\}/);
      if (named) {
        for (const part of named[1].split(",")) {
          const name = part.trim().split(/\s+as\s+/)[0].trim();
          if (name) imported.add(name);
        }
      }
      const def = clause.replace(/\{[\s\S]*?\}/, "").replace(/,/g, "").trim();
      if (def) imported.add(def);
    }
    const plugged = new Set(
      [...registrationSource.matchAll(/^\s*(El[A-Za-z]+),?\s*$/gm)].map((m) => m[1]),
    );

    const missing: string[] = [];
    for (const tag of usedElementTags()) {
      const name = TAG_TO_REGISTRATION[tag];
      if (!name) {
        missing.push(`${tag}: 未知标签 —— 需要在 TAG_TO_REGISTRATION 里登记并完成注册`);
        continue;
      }
      // 既要 import 进来，也要放进 componentPlugins 才会被 app.use()
      if (!imported.has(name) || !plugged.has(name)) {
        missing.push(`${tag}: ${name} 未在 src/element-plus.ts 中 import 并注册`);
      }
    }
    expect(missing).toEqual([]);
  });

  it("模板里用到的每个 el-* 组件都有对应的样式导入", () => {
    const missing: string[] = [];
    for (const tag of usedElementTags()) {
      const file = TAG_TO_STYLE[tag];
      if (file === undefined) {
        missing.push(`${tag}: 未知标签 —— 需要在 TAG_TO_STYLE 里登记`);
        continue;
      }
      if (file === "") continue; // 明确不导入（空样式表）
      if (!styleSource.includes(`theme-chalk/${file}`)) {
        missing.push(
          `${tag}: 缺少 theme-chalk/${file} 的样式导入${
            STYLE_EXEMPT[tag] ? `（${STYLE_EXEMPT[tag]}）` : ""
          }`,
        );
      }
    }
    expect(missing).toEqual([]);
  });

  it("引入了 Element Plus 的 CSS 变量总表（base.css）", () => {
    // 变量总表缺失 → 组件样式里的 var(--el-xxx) 在计算值阶段失效，
    // 文字/尺寸会退回继承值（例如单选按钮文字继承到 font-size:0 而消失）。
    expect(styleSource).toContain("theme-chalk/base.css");
  });

  it("以 API 形式调用的每个 Element Plus 组件都有对应的样式导入", () => {
    // 这类组件没有 <el-xxx> 标签，漏掉样式表时提示会静默不可见。
    const missing: string[] = [];
    for (const name of usedApiComponents()) {
      const file = API_COMPONENTS[name];
      if (!file) {
        missing.push(`${name}: 未知组件 —— 需要在 API_COMPONENTS 里登记`);
        continue;
      }
      if (!styleSource.includes(`theme-chalk/${file}`)) {
        missing.push(`${name}: 缺少 theme-chalk/${file} 的样式导入`);
      }
    }
    expect(missing).toEqual([]);
  });

  it("没有零引用的 Element Plus API 导出（避免用起来才发现没样式）", () => {
    // 导出了却没有任何调用方 —— 与「两侧零引用的类型」同理：它在声明
    // 一个不存在的能力，而且一旦有人用它，样式表还得现补。
    const exported = new Set(
      [...registrationSource.matchAll(
        /export\s*\{([^}]+)\}\s*from\s*["']element-plus[^"']*["']/g,
      )].flatMap((m) => m[1].split(",").map((s) => s.trim()).filter(Boolean)),
    );
    const used = usedApiComponents();
    const unused = [...exported].filter((name) => !used.has(name));
    expect(unused).toEqual([]);
  });

  it("样式导入清单与注册清单没有互相遗漏的孤儿条目", () => {
    const registered = [...registrationSource.matchAll(/^\s*(El[A-Za-z]+),?\s*$/gm)].map(
      (m) => m[1],
    );
    const styles = [...styleSource.matchAll(/theme-chalk\/(el-[a-z-]+\.css)/g)].map((m) => m[1]);
    // 注册了但没有任何样式：组件会以无样式形态出现
    const styleFor = (name: string) => {
      const kebab = name.replace(/^El/, "el-").replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase();
      return styles.some((s) => s === `${kebab}.css` || s.startsWith(`${kebab}-`));
    };
    const kebab = (name: string) =>
      name.replace(/^El/, "el-").replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase();
    const withoutStyle = registered.filter((n) => !styleFor(n));
    // 例外：样式表是空文件、仓库明确不引入的组件
    expect(withoutStyle.filter((n) => STYLE_EXEMPT[kebab(n)] === undefined)).toEqual([]);
  });
});
