import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import ElementPlus from "element-plus";
import PluginPanel from "../../src/components/PluginPanel.vue";
import { listPlugins } from "../../src/ipc/client";
import type { PluginInfo } from "../../src/ipc/types";

vi.mock("../../src/ipc/client", () => ({
  listPlugins: vi.fn(),
  scanPlugins: vi.fn().mockResolvedValue(undefined),
  loadPlugin: vi.fn().mockResolvedValue(undefined),
  unloadPlugin: vi.fn().mockResolvedValue(undefined),
}));

const plugin: PluginInfo = {
  id: "demo",
  name: "demo",
  version: "0.1.0",
  author: "someone",
  description: "示例插件",
  plugin_type: "Wasm",
  state: "Discovered",
  // 后端用 Debug 输出传扩展点（见 rshell-plugin-sdk 的 loader.rs）
  extensions: ["SessionList", 'ToolPanel { id: "demo", title: "示例面板" }'],
  permissions: ["NetworkAccess", "FileSystemAccess"],
};

const stubs = {
  "el-button-group": { template: "<div><slot /></div>" },
  "el-button": { template: "<button><slot /></button>" },
  "el-tag": { template: "<span class='el-tag'><slot /></span>" },
  "el-empty": { props: ["description"], template: '<div>{{ description }}</div>' },
};

async function mountPanel() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const wrapper = mount(PluginPanel, {
    global: { plugins: [pinia, ElementPlus], stubs },
  });
  await flushPromises();
  return wrapper;
}

describe("PluginPanel 能力边界", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(listPlugins).mockResolvedValue([plugin]);
  });

  it("说明「已加载」只表示模块编译并登记，不等于拿到宿主能力", async () => {
    const wrapper = await mountPanel();
    const note = wrapper.find('[data-test="plugin-capability-note"]');
    expect(note.exists()).toBe(true);
    expect(note.text()).toContain("编译");
    expect(note.text()).toContain("不会生效");
    wrapper.unmount();
  });

  it("展示清单声明的权限（翻成可读说法）", async () => {
    const wrapper = await mountPanel();
    const text = wrapper.text();
    expect(text).toContain("网络访问");
    expect(text).toContain("文件系统");
    wrapper.unmount();
  });

  it("展示清单声明的扩展点", async () => {
    const wrapper = await mountPanel();
    expect(wrapper.text()).toContain("SessionList");
    wrapper.unmount();
  });

  // 后端把扩展点用 Rust 的 Debug 输出传过来（`ToolPanel { id: …, title: … }`），
  // 直接显示等于把内部结构甩给用户，而且这串文本很长，会把卡片撑破、
  // 让侧栏出现横向滚动条。
  it("扩展点翻成可读说法，不显示 Rust 的 Debug 串", async () => {
    const wrapper = await mountPanel();
    const card = wrapper.get('[data-test="plugin-card-demo"]');
    expect(card.text()).toContain("工具面板（示例面板）");
    expect(card.text()).not.toContain("ToolPanel {");
    wrapper.unmount();
  });

  // 侧栏只有 200~360px：表格会把「名称」压成 0 宽（内容看不见），
  // 把「加载 / 卸载」整列推到视口外。插件列表必须用卡片渲染。
  it("插件列表用卡片渲染，不渲染表格", async () => {
    const wrapper = await mountPanel();
    expect(wrapper.find(".el-table").exists()).toBe(false);
    const list = wrapper.find('[data-test="plugin-list"]');
    expect(list.exists()).toBe(true);
    // 名称必须真的出现在界面上（表格版本里这一列宽度是 0，看不见）
    expect(list.text()).toContain("demo");
    expect(list.text()).toContain("0.1.0");
    wrapper.unmount();
  });
});
