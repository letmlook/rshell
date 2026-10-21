import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import TerminalSettingsPanel from "../../src/components/TerminalSettingsPanel.vue";
import ThemePanel from "../../src/components/ThemePanel.vue";
import {
  copyOnSelect,
  setCopyOnSelect,
  COPY_ON_SELECT_KEY,
  loadTerminalAppearance,
  normalizeTerminalAppearance,
  terminalAppearance,
  DEFAULT_TERMINAL_APPEARANCE,
} from "../../src/utils/terminalPrefs";

const { applyScheme, refresh } = vi.hoisted(() => ({
  applyScheme: vi.fn().mockResolvedValue(undefined),
  refresh: vi.fn().mockResolvedValue(undefined),
}));

// 用受控 stub 替换 el-select：真实 el-select 的传送门与内部 watch 会与
// 测试里的状态注入互相触发（Maximum recursive updates），测不出我的逻辑。
const stubs = {
  "el-select": defineComponent({
    name: "el-select",
    props: ["modelValue", "options"],
    emits: ["change"],
    template: '<div class="el-select"><slot /></div>',
  }),
  "el-option": defineComponent({
    name: "el-option",
    props: ["label", "value", "disabled"],
    template: '<div class="el-option" :class="{ \'is-disabled\': disabled }">{{ label }}</div>',
  }),
  "el-switch": defineComponent({
    name: "el-switch",
    props: ["modelValue"],
    emits: ["update:modelValue"],
    template: '<button class="el-switch" @click="$emit(\'update:modelValue\', !modelValue)" />',
  }),
  "el-button": defineComponent({ name: "el-button", template: "<button><slot /></button>" }),
  "el-input-number": defineComponent({
    name: "el-input-number",
    props: ["modelValue", "min", "max", "step"],
    emits: ["change"],
    template: '<input type="number" :value="modelValue" />',
  }),
};

vi.mock("../../src/stores/theme", () => ({
  useThemeStore: () => ({
    currentTheme: "default",
    currentScheme: "Dracula",
    availableThemes: ["default", "gruvbox"],
    availableSchemes: ["Monokai", "Dracula", "Solarized"],
    loading: false,
    error: null as string | null,
    refresh,
    applyTheme: vi.fn(),
    applyScheme,
  }),
}));

/**
 * 终端配置项（配色方案、选中即复制）从终端浮层搬到了「设置 → 终端」。
 * 锁两件事：入口唯一（主题面板不再有第二处配色入口），偏好在设置与终端之间
 * 共享同一份响应式状态。
 */
describe("TerminalSettingsPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.removeItem(COPY_ON_SELECT_KEY);
    setCopyOnSelect(true);
  });

  function mountPanel() {
    return mount(TerminalSettingsPanel, { props: { embedded: true }, global: { stubs } });
  }

  it("列出后端已安装的配色方案，当前项禁用（避免重复切换）", () => {
    const wrapper = mountPanel();
    // 按 data-test 定位：面板里不止一个下拉（还有字体、光标样式）
    const options = wrapper
      .get('[data-test="ts-scheme"]')
      .findAll(".el-option");
    expect(options.map((o) => o.text())).toEqual(["Monokai", "Dracula", "Solarized"]);
    expect(options[1].classes()).toContain("is-disabled");
  });

  it("切换配色调用后端 applyScheme，而不是本地改状态", async () => {
    const wrapper = mountPanel();
    const select = wrapper.findComponent({ name: "el-select" });
    select.vm.$emit("change", "Monokai");
    await flushPromises();

    expect(applyScheme).toHaveBeenCalledWith("Monokai");
  });

  it("重新加载方案显式发 IPC，不在挂载时无条件拉取", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    refresh.mockClear();

    const reload = wrapper.find('[data-test="ts-scheme-reload"]');
    expect(reload.exists()).toBe(true);
    await reload.trigger("click");
    await flushPromises();
    expect(refresh).toHaveBeenCalled();
  });

  it("选中即复制开关写共享状态，已打开的终端能立即看到变化", async () => {
    const wrapper = mountPanel();
    expect(copyOnSelect.value).toBe(true);

    await wrapper.get('[data-test="ts-copy-on-select"]').trigger("click");
    await flushPromises();

    expect(copyOnSelect.value).toBe(false);
    expect(localStorage.getItem(COPY_ON_SELECT_KEY)).toBe("false");
  });

  it("写盘失败不抛错：偏好仍在本次运行内生效", async () => {
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = () => {
      throw new Error("QuotaExceededError");
    };
    try {
      const wrapper = mountPanel();
      await wrapper.get('[data-test="ts-copy-on-select"]').trigger("click");
      await flushPromises();
      // 内存态已切换，只是没记住
      expect(copyOnSelect.value).toBe(false);
    } finally {
      Storage.prototype.setItem = original;
    }
  });
});

describe("ThemePanel 只保留应用主题", () => {
  it("不再出现终端配色入口（终端设置项只在「设置 → 终端」）", () => {
    const wrapper = mount(ThemePanel, { props: { embedded: true }, global: { stubs } });
    // 只剩一个 el-select（应用主题），配色方案那一条已移走
    expect(wrapper.findAllComponents({ name: "el-select" })).toHaveLength(1);
    expect(wrapper.text()).toContain("设置 → 终端");
  });
});

// 终端外观：字号 / 字体 / 光标 / 回滚行数此前在 TerminalPane 里写死，
// 而 TerminalConfig 类型早已声明了同样的字段却从未接线。
describe("TerminalSettingsPanel 终端外观", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    refresh.mockResolvedValue(undefined);
  });

  it("提供字号、字体、光标与回滚行数的控件", async () => {
    const wrapper = mount(TerminalSettingsPanel, { global: { stubs } });
    await flushPromises();

    expect(wrapper.find('[data-test="ts-font-size"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="ts-font-family"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="ts-cursor-style"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="ts-scrollback"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="ts-cursor-blink"]').exists()).toBe(true);
    wrapper.unmount();
  });

  it("改字号会更新共享偏好（已打开的终端据此立即生效）", async () => {
    const wrapper = mount(TerminalSettingsPanel, { global: { stubs } });
    await flushPromises();

    // data-test 落在桩的根元素上：按组件 + 属性定位，别用 DOMWrapper（它没有 vm）
    const size = wrapper
      .findAllComponents({ name: "el-input-number" })
      .find((component) => component.attributes("data-test") === "ts-font-size");
    expect(size).toBeDefined();
    await size!.vm.$emit("change", 18);
    await flushPromises();

    expect(terminalAppearance.value.fontSize).toBe(18);
    wrapper.unmount();
  });

  it("字号被夹到合法范围", () => {
    expect(normalizeTerminalAppearance({ fontSize: 2 }).fontSize).toBe(8);
    expect(normalizeTerminalAppearance({ fontSize: 999 }).fontSize).toBe(32);
    expect(normalizeTerminalAppearance({ fontSize: "abc" }).fontSize).toBe(13);
  });

  it("非法光标样式回落到默认值", () => {
    expect(normalizeTerminalAppearance({ cursorStyle: "weird" }).cursorStyle).toBe("block");
    expect(normalizeTerminalAppearance({ cursorStyle: "bar" }).cursorStyle).toBe("bar");
  });

  it("回滚行数被夹到合法范围", () => {
    expect(normalizeTerminalAppearance({ scrollback: -5 }).scrollback).toBe(0);
    expect(normalizeTerminalAppearance({ scrollback: 10_000_000 }).scrollback).toBe(200000);
  });

  it("损坏的存储内容回落到默认值而不是抛错", () => {
    const storage = { getItem: () => "{not json" };
    expect(loadTerminalAppearance(storage)).toEqual(DEFAULT_TERMINAL_APPEARANCE);
  });
});
