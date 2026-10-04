import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import TransferPanel, { type TransferItem } from "../../src/components/TransferPanel.vue";
import type { TransferLogEntry } from "../../src/utils/transferLog";

/**
 * 日志 tab 之前只有一行「传输错误会显示在任务状态中」的占位文案，用户看不到
 * 任何事件；拖拽高度也是新增能力。这两组用例把它们的行为钉住。
 */
const sampleItems: TransferItem[] = [
  { id: "t-active", name: "a.log", phase: "active", progress: 0.4, size: 100, local: "/a", remote: "/b", speed: 1 },
];

const logs: TransferLogEntry[] = [
  { seq: 1, time: new Date(2026, 0, 2, 3, 4, 5).getTime(), level: "info", message: "上传入队 1 个文件", detail: "/var/log" },
  { seq: 2, time: new Date(2026, 0, 2, 3, 4, 6).getTime(), level: "error", message: "传输失败：a.log", detail: "Permission denied" },
];

function mountPanel(propsOverride: Record<string, unknown> = {}) {
  return mount(TransferPanel, {
    props: { expanded: true, items: sampleItems, pendingTaskIds: new Set<string>(), ...propsOverride },
  });
}

describe("TransferPanel 日志页", () => {
  it("切到日志 tab 后逐条渲染真实日志（含时间、等级与明细）", async () => {
    const wrapper = mountPanel({ logs });
    await wrapper.get('[data-test="xfer-tab-log"]').trigger("click");

    const list = wrapper.get('[data-test="xfer-log-list"]');
    expect(list.text()).toContain("上传入队 1 个文件");
    expect(list.text()).toContain("传输失败：a.log");
    expect(list.text()).toContain("Permission denied");
    // 等级标签可区分，失败行不是普通信息
    expect(wrapper.get('[data-test="xfer-log-2"]').classes()).toContain("is-error");
    expect(wrapper.get('[data-test="xfer-log-1"]').text()).toContain("03:04:05");
  });

  it("没有任何日志时显示空状态说明，不用占位文案假装有内容", async () => {
    const wrapper = mountPanel({ logs: [] });
    await wrapper.get('[data-test="xfer-tab-log"]').trigger("click");

    expect(wrapper.find('[data-test="xfer-log-list"]').exists()).toBe(false);
    expect(wrapper.get('[data-test="xfer-log-empty"]').text()).toContain("暂无日志记录");
  });

  it("日志数在 tab 上可见，并如实提示被丢弃的更早记录", async () => {
    const wrapper = mountPanel({ logs, droppedLogs: 12 });
    expect(wrapper.get('[data-test="xfer-tab-log"]').text()).toContain("日志 (2)");

    await wrapper.get('[data-test="xfer-tab-log"]').trigger("click");
    expect(wrapper.get('[data-test="xfer-log-dropped"]').text()).toContain("12");
  });

  it("清空按钮发出 clear-log，不自己改数据", async () => {
    const wrapper = mountPanel({ logs });
    await wrapper.get('[data-test="xfer-tab-log"]').trigger("click");
    await wrapper.get('[data-test="xfer-log-clear"]').trigger("click");

    expect(wrapper.emitted("clear-log")).toHaveLength(1);
    // 数据仍由父组件掌握
    expect(wrapper.get('[data-test="xfer-log-list"]').text()).toContain("上传入队 1 个文件");
  });
});

describe("TransferPanel 顶部边缘拖动高度", () => {
  it("向上拖动把面板变高并回写 update:height", async () => {
    const wrapper = mountPanel({ height: 220 });
    await wrapper.get('[data-test="xfer-resize"]').trigger("mousedown", { clientY: 500 });
    window.dispatchEvent(new MouseEvent("mousemove", { clientY: 460 }));
    window.dispatchEvent(new MouseEvent("mouseup"));

    expect(wrapper.emitted("update:height")?.at(-1)).toEqual([260]);
  });

  it("拖动有下限，不会把面板压到读不出操作按钮", async () => {
    const wrapper = mountPanel({ height: 220 });
    await wrapper.get('[data-test="xfer-resize"]').trigger("mousedown", { clientY: 200 });
    window.dispatchEvent(new MouseEvent("mousemove", { clientY: 2000 }));

    expect(wrapper.emitted("update:height")?.at(-1)).toEqual([120]);
    window.dispatchEvent(new MouseEvent("mouseup"));
  });

  it("鼠标在面板外松开也要收尾，不再继续回写高度", async () => {
    const wrapper = mountPanel({ height: 220 });
    await wrapper.get('[data-test="xfer-resize"]').trigger("mousedown", { clientY: 500 });
    window.dispatchEvent(new MouseEvent("mousemove", { clientY: 480 }));
    window.dispatchEvent(new MouseEvent("mouseup"));
    const before = wrapper.emitted("update:height")?.length ?? 0;

    window.dispatchEvent(new MouseEvent("mousemove", { clientY: 300 }));
    expect(wrapper.emitted("update:height")?.length ?? 0).toBe(before);
  });

  it("双击拖动条恢复默认高度", async () => {
    const wrapper = mountPanel({ height: 400 });
    await wrapper.get('[data-test="xfer-resize"]').trigger("dblclick");

    expect(wrapper.emitted("update:height")?.at(-1)).toEqual([220]);
  });

  it("折叠态不渲染拖动条", () => {
    const wrapper = mountPanel({ expanded: false });
    expect(wrapper.find('[data-test="xfer-resize"]').exists()).toBe(false);
  });
});
