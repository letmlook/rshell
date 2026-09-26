import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import TransferPanel from "../../src/components/TransferPanel.vue";

const tableStub = {
  props: ["data", "emptyText"],
  template: '<div data-stub="table">{{ data.length }}|{{ emptyText }}</div>',
};

function panel(props: Record<string, unknown> = {}) {
  return mount(TransferPanel, {
    props: { expanded: true, items: [], ...props },
    global: { stubs: { "el-table": tableStub, "el-table-column": true } },
  });
}

describe("TransferPanel", () => {
  it("shows an alert instead of a silent empty state when the queue failed to load", () => {
    const wrapper = panel({ error: "无法读取传输队列: ipc failed" });
    const alert = wrapper.find('[role="alert"]');
    expect(alert.exists()).toBe(true);
    expect(alert.text()).toContain("无法读取传输队列");
  });

  it("renders no alert when the queue loads cleanly", () => {
    expect(panel().find('[role="alert"]').exists()).toBe(false);
  });

  it("still renders the empty text for a clean empty queue", () => {
    expect(panel().text()).toContain("暂无传输任务");
  });
});
