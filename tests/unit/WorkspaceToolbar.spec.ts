import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import WorkspaceToolbar from "../../src/components/WorkspaceToolbar.vue";

const props = {
  workspace: "terminal" as const,
  connectionState: "connected",
  sidebarExpanded: true,
  onNewSession: () => undefined,
};

describe("WorkspaceToolbar context navigation", () => {
  it("renders all five context buttons in the top toolbar", () => {
    const wrapper = mount(WorkspaceToolbar, { props });
    expect(wrapper.get('[data-testid="context-sessions"]').text()).toContain("会话");
    expect(wrapper.get('[data-testid="context-files"]').text()).toContain("文件");
    expect(wrapper.get('[data-testid="context-keys"]').text()).toContain("密钥");
    expect(wrapper.get('[data-testid="context-tools"]').text()).toContain("工具");
    expect(wrapper.get('[data-testid="context-settings"]').text()).toContain("设置");
  });

  it("emits select-panel and expands the sidebar from a context button", async () => {
    const wrapper = mount(WorkspaceToolbar, { props });
    await wrapper.get('[data-testid="context-keys"]').trigger("click");
    expect(wrapper.emitted("select-panel")).toEqual([["keys"]]);
    expect(wrapper.emitted("toggle-sidebar")).toEqual([[true]]);
  });

  it("emits a toggle request and exposes aria-pressed", async () => {
    const wrapper = mount(WorkspaceToolbar, { props: { ...props, sidebarExpanded: false } });
    const button = wrapper.get('[data-testid="toggle-sidebar"]');
    expect(button.attributes("aria-pressed")).toBe("false");
    await button.trigger("click");
    expect(wrapper.emitted("toggle-sidebar")).toEqual([[true]]);
  });
});
