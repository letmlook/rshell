/**
 * CustomTitleBar 拖动区契约 —— 锁住 macOS 行为。
 *
 * jsdom 不会模拟 `-webkit-app-region`,但合约要求:
 * - 根 <header> 自身 **不** 带 drag 属性,避免与子节点嵌套冲突。
 * - 中心可拖动区带 `data-tauri-drag-region` 与 `-webkit-app-region: drag`。
 * - 左侧(logo + 菜单)与右侧(窗口按钮)必须带 `-webkit-app-region: no-drag`,
 *   防止点击穿透或菜单被吞。
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";

// 标题栏在 mount 时会调用 `getCurrentWindow().isMaximized()` 之类的 API,
// 这里在导入前先 stub 掉,避免 jsdom 报错。
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isMaximized: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => undefined),
    minimize: () => Promise.resolve(),
    maximize: () => Promise.resolve(),
    unmaximize: () => Promise.resolve(),
    close: () => Promise.resolve(),
  }),
}));

// el-dropdown 在测试中不需要真实行为,挂一个简单 stub 防止控制台噪音。
vi.mock("element-plus", () => ({
  default: {
    install() {
      /* no-op in tests */
    },
  },
}));
// 显式 stub 模板里出现的 Element Plus 组件,避免"Failed to resolve component" 警告。
const elStub = (name: string) => ({
  name,
  props: ["trigger"],
  emits: ["click"],
  template: "<div class='el-stub' data-name='" + name + "'><slot /></div>",
});
const elDropdownItemStub = {
  name: "el-dropdown-item",
  props: ["disabled"],
  emits: ["click"],
  template: "<button class='el-stub-dropdown-item'><slot /></button>",
};
const elDropdownMenuStub = {
  name: "el-dropdown-menu",
  template: "<div class='el-stub-dropdown-menu'><slot /></div>",
};

import CustomTitleBar from "../../src/components/CustomTitleBar.vue";

beforeEach(() => {
  setActivePinia(createPinia());
});

const globalStubs = {
  "el-dropdown": elStub("el-dropdown"),
  "el-dropdown-item": elDropdownItemStub,
  "el-dropdown-menu": elDropdownMenuStub,
};

function mountTitleBar() {
  return mount(CustomTitleBar, { global: { stubs: globalStubs } });
}

describe("CustomTitleBar drag contract (macOS)", () => {
  it("root header does not declare drag itself", () => {
    const wrapper = mountTitleBar();
    const header = wrapper.element as HTMLElement;
    expect(header.hasAttribute("data-tauri-drag-region")).toBe(false);
    expect(header.classList.contains("is-draggable")).toBe(false);
  });

  it("center region is the only drag-enabled area", () => {
    const wrapper = mountTitleBar();
    const center = wrapper.find(".titlebar-center").element as HTMLElement;
    expect(center.getAttribute("data-tauri-drag-region")).toBe("");
    expect(center.classList.contains("is-draggable")).toBe(true);
  });

  it("left and right groups are excluded from drag", () => {
    const wrapper = mountTitleBar();
    const left = wrapper.find(".titlebar-left").element as HTMLElement;
    const right = wrapper.find(".titlebar-right").element as HTMLElement;
    expect(left.getAttribute("data-tauri-drag-region")).toBe("false");
    expect(left.classList.contains("is-draggable")).toBe(false);
    expect(right.getAttribute("data-tauri-drag-region")).toBe("false");
    expect(right.classList.contains("is-draggable")).toBe(false);
  });

  it("center region forwards dblclick to maximize toggle", async () => {
    const wrapper = mountTitleBar();
    const center = wrapper.find(".titlebar-center");
    await center.trigger("dblclick");
    expect(center.exists()).toBe(true);
  });
});
