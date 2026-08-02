import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import SidePanel from "../../src/components/SidePanel.vue";

const stubs = {
  SessionList: { template: "<div data-testid='session-child' />" },
  KeyManagerPanel: { template: "<div data-testid='key-child' />" },
  QuickCommandPanel: { template: "<div data-testid='quick-child' />" },
  TriggerEditor: { template: "<div data-testid='trigger-child' />" },
  TunnelPanel: { template: "<div data-testid='tunnel-child' />" },
  ThemePanel: { template: "<div data-testid='theme-child' />" },
  PluginPanel: { template: "<div data-testid='plugin-child' />" },
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
});