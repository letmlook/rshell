import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import TransferPanel, { type TransferItem } from "../../src/components/TransferPanel.vue";

const sampleItems: TransferItem[] = [
  { id: "t-active", name: "active", phase: "active", progress: 0.4, size: 100, local: "/a", remote: "/b", speed: 1 },
  { id: "t-paused", name: "paused", phase: "paused", progress: 0.2, size: 100, local: "/a", remote: "/b", speed: 0 },
  { id: "t-done", name: "done", phase: "done", progress: 1, size: 100, local: "/a", remote: "/b", speed: 0 },
  { id: "t-failed", name: "failed", phase: "failed", progress: 0.5, size: 100, local: "/a", remote: "/b", speed: 0, error: "boom" },
];

function mountPanel(propsOverride: Record<string, unknown> = {}) {
  return mount(TransferPanel, {
    props: {
      expanded: true,
      items: sampleItems,
      pendingTaskIds: new Set<string>(),
      ...propsOverride,
    },
  });
}

describe("TransferPanel pause/resume controls", () => {
  it("shows a 暂停按钮 only for active tasks and a 继续按钮 only for paused tasks", () => {
    const wrapper = mountPanel();
    const pauseButtons = wrapper.findAll('[data-test="xfer-pause"]');
    const resumeButtons = wrapper.findAll('[data-test="xfer-resume"]');
    expect(pauseButtons).toHaveLength(1);
    expect(resumeButtons).toHaveLength(1);
    expect((pauseButtons[0].element as HTMLButtonElement).getAttribute("aria-label")).toBe("暂停传输");
    expect((resumeButtons[0].element as HTMLButtonElement).getAttribute("aria-label")).toBe("继续传输");
    const terminalIds = ["t-done", "t-failed"];
    for (const id of terminalIds) {
      const row = wrapper.find(`[data-row-id="${id}"]`);
      expect(row.find('[data-test="xfer-pause"]').exists()).toBe(false);
      expect(row.find('[data-test="xfer-resume"]').exists()).toBe(false);
    }
  });

  it("emits pause(taskId) with the row id when its pause button is clicked", async () => {
    const wrapper = mountPanel();
    const button = wrapper.find('[data-row-id="t-active"] [data-test="xfer-pause"]');
    await button.trigger("click");
    expect(wrapper.emitted("pause")).toEqual([["t-active"]]);
    expect(wrapper.emitted("resume")).toBeUndefined();
  });

  it("emits resume(taskId) with the row id when its resume button is clicked", async () => {
    const wrapper = mountPanel();
    const button = wrapper.find('[data-row-id="t-paused"] [data-test="xfer-resume"]');
    await button.trigger("click");
    expect(wrapper.emitted("resume")).toEqual([["t-paused"]]);
    expect(wrapper.emitted("pause")).toBeUndefined();
  });

  it("disables only the buttons belonging to the pending task ids", () => {
    const wrapper = mountPanel({ pendingTaskIds: new Set(["t-active"]) });
    const activePause = wrapper.find('[data-row-id="t-active"] [data-test="xfer-pause"]')
      .element as HTMLButtonElement;
    const pausedResume = wrapper.find('[data-row-id="t-paused"] [data-test="xfer-resume"]')
      .element as HTMLButtonElement;
    expect(activePause.disabled).toBe(true);
    expect(pausedResume.disabled).toBe(false);
  });

  it("ignores repeated clicks while an action is pending and surfaces a one-line error banner", async () => {
    const wrapper = mountPanel({
      pendingTaskIds: new Set(["t-active"]),
      actionError: "暂停失败：传输已结束",
    });
    const button = wrapper.find('[data-row-id="t-active"] [data-test="xfer-pause"]');
    await button.trigger("click");
    expect(wrapper.emitted("pause")).toBeUndefined();
    expect(wrapper.find('[data-test="xfer-action-error"]').text()).toContain("暂停失败");
  });
});

describe("TransferPanel cancel/remove controls", () => {
  const cancelledItem: TransferItem = {
    id: "t-cancelled", name: "cancelled", phase: "cancelled",
    progress: 0.3, size: 100, local: "/a", remote: "/b", speed: 0,
  };

  it("shows 取消 for active and paused tasks only", () => {
    const wrapper = mountPanel();
    const cancels = wrapper.findAll('[data-test="xfer-cancel"]');
    expect(cancels).toHaveLength(2);
    expect(cancels.map((b) => b.attributes("data-task-id")).sort()).toEqual(["t-active", "t-paused"]);
    for (const id of ["t-done", "t-failed"]) {
      expect(wrapper.find(`[data-row-id="${id}"] [data-test="xfer-cancel"]`).exists()).toBe(false);
    }
  });

  it("shows 删除 only for terminal tasks (done/failed/cancelled)", () => {
    const wrapper = mountPanel({ items: [...sampleItems, cancelledItem] });
    const removes = wrapper.findAll('[data-test="xfer-remove"]');
    expect(removes.map((b) => b.attributes("data-task-id")).sort())
      .toEqual(["t-cancelled", "t-done", "t-failed"]);
    // 活跃任务仍由 CancelTransfer 收尾，不提供直接移除
    for (const id of ["t-active", "t-paused"]) {
      expect(wrapper.find(`[data-row-id="${id}"] [data-test="xfer-remove"]`).exists()).toBe(false);
    }
  });

  it("emits cancel(taskId) from the active row", async () => {
    const wrapper = mountPanel();
    await wrapper.find('[data-row-id="t-active"] [data-test="xfer-cancel"]').trigger("click");
    expect(wrapper.emitted("cancel")).toEqual([["t-active"]]);
    expect(wrapper.emitted("remove")).toBeUndefined();
  });

  it("emits remove(taskId) from a terminal row", async () => {
    const wrapper = mountPanel();
    await wrapper.find('[data-row-id="t-failed"] [data-test="xfer-remove"]').trigger("click");
    expect(wrapper.emitted("remove")).toEqual([["t-failed"]]);
    expect(wrapper.emitted("cancel")).toBeUndefined();
  });

  it("disables 取消/删除 while that task's action is pending", () => {
    const wrapper = mountPanel({
      items: [...sampleItems, cancelledItem],
      pendingTaskIds: new Set(["t-active", "t-done"]),
    });
    const activeCancel = wrapper.find('[data-row-id="t-active"] [data-test="xfer-cancel"]')
      .element as HTMLButtonElement;
    const doneRemove = wrapper.find('[data-row-id="t-done"] [data-test="xfer-remove"]')
      .element as HTMLButtonElement;
    const pausedCancel = wrapper.find('[data-row-id="t-paused"] [data-test="xfer-cancel"]')
      .element as HTMLButtonElement;
    expect(activeCancel.disabled).toBe(true);
    expect(doneRemove.disabled).toBe(true);
    expect(pausedCancel.disabled).toBe(false);
  });

  it("ignores remove clicks while pending and reports the failure banner", async () => {
    const wrapper = mountPanel({
      pendingTaskIds: new Set(["t-done"]),
      actionError: "删除失败：Invalid state",
    });
    await wrapper.find('[data-row-id="t-done"] [data-test="xfer-remove"]').trigger("click");
    expect(wrapper.emitted("remove")).toBeUndefined();
    expect(wrapper.find('[data-test="xfer-action-error"]').text()).toContain("删除失败");
  });
});