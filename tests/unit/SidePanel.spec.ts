import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import SidePanel from "../../src/components/SidePanel.vue";

const stubs = {
  SessionList: { template: "<div data-testid='session-child' />" },
  KeyManagerPanel: { template: "<div data-testid='key-child' />" },
  QuickCommandPanel: { template: "<div data-testid='quick-child' />" },
  TriggerEditor: { template: "<div data-testid='trigger-child' />" },
  TunnelPanel: { template: "<div data-testid='tunnel-child' />" },
  ThemePanel: { template: "<div data-testid='theme-child' />" },
  PluginPanel: { template: "<div data-testid='plugin-child' />" },
  "el-button": { props: ["disabled"], template: "<button :disabled='disabled'><slot /></button>" },
};

describe("SidePanel", () => {
  it("renders only the selected top-level panel", () => {
    const wrapper = mount(SidePanel, { props: { active: "keys" }, global: { stubs } });
    expect(wrapper.find('[data-testid="key-child"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-child"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="quick-child"]').exists()).toBe(false);
  });

  it("renders only one tools subview", async () => {
    const wrapper = mount(SidePanel, { props: { active: "tools" }, global: { stubs } });
    expect(wrapper.find('[data-testid="quick-child"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="trigger-child"]').exists()).toBe(false);
    await wrapper.get('[data-testid="tools-triggers"]').trigger("click");
    expect(wrapper.find('[data-testid="quick-child"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="trigger-child"]').exists()).toBe(true);
  });

  it("exposes a keyboard separator with confirmed geometry", async () => {
    const wrapper = mount(SidePanel, { props: { active: "sessions", width: 280 }, global: { stubs } });
    const separator = wrapper.get('[data-testid="sidebar-separator"]');
    expect(separator.attributes("role")).toBe("separator");
    expect(separator.attributes("aria-valuemin")).toBe("200");
    expect(separator.attributes("aria-valuemax")).toBe("360");
    expect(separator.attributes("aria-valuenow")).toBe("280");
    await separator.trigger("keydown", { key: "ArrowRight" });
    expect(wrapper.emitted("update:width")).toEqual([[288]]);
  });

  it("offers the transfer workspace only for a connected SSH session", async () => {
    const wrapper = mount(SidePanel, { props: { active: "files", activeSessionId: "s", activeSessionConnected: false }, global: { stubs } });
    const button = wrapper.get(".placeholder button");
    expect(button.attributes("disabled")).toBeDefined();
    await wrapper.setProps({ activeSessionConnected: true });
    await button.trigger("click");
    expect(wrapper.emitted("open-sftp")).toEqual([["s"]]);
  });

  /**
   * 侧栏是会话事件的**中转层**：SessionList 发出的事件必须一路转发到 App.vue。
   *
   * 「编辑已有连接」此前就漏在这里：App.vue 监听的是 `<SidePanel>` 上的
   * `@edit-session`，而 SidePanel 既没声明也没转发这个事件 —— 右键菜单里的
   * 「编辑连接」点了毫无反应，编辑已有连接整个不可达。
   */
  it("把 SessionList 的编辑连接事件转发给上层", async () => {
    const sessionList = defineComponent({
      name: "SessionList",
      emits: ["edit-session"],
      template: "<button data-testid='edit-session' @click=\"$emit('edit-session', 's-1')\">编辑</button>",
    });
    const wrapper = mount(SidePanel, {
      props: { active: "sessions" },
      global: { stubs: { ...stubs, SessionList: sessionList } },
    });
    await wrapper.get('[data-testid="edit-session"]').trigger("click");
    expect(wrapper.emitted("edit-session")).toEqual([["s-1"]]);
  });
});
