import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import ElementPlus from "element-plus";
import { computed, defineComponent, h, inject, provide, ref, type Ref } from "vue";
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
  extensions: ["SessionList"],
  permissions: ["NetworkAccess", "FileSystemAccess"],
};

const stubs = {
  "el-button-group": { template: "<div><slot /></div>" },
  "el-button": { template: "<button><slot /></button>" },
  "el-tag": { template: "<span class='el-tag'><slot /></span>" },
  "el-empty": { props: ["description"], template: '<div>{{ description }}</div>' },
  "el-table": defineComponent({
    name: "el-table",
    props: ["data"],
    setup(props, { slots }) {
      provide("rs-rows", computed(() => (props.data ?? []) as unknown[]));
      return () => h("div", [h("pre", JSON.stringify(props.data)), slots.default?.()]);
    },
  }),
  // 列桩按行渲染 #default：否则行内的权限标签根本没挂载，断言不到
  "el-table-column": defineComponent({
    name: "el-table-column",
    setup(_props, { slots }) {
      const rows = inject<Ref<unknown[]>>("rs-rows", ref([]));
      return () => h("div", rows.value.map((row) => slots.default?.({ row })));
    },
  }),
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
});
