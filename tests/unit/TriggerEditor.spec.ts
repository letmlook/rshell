import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import ElementPlus from "element-plus";
import TriggerEditor from "../../src/components/TriggerEditor.vue";
import { deleteTrigger, listTriggers } from "../../src/ipc/client";

// R2-14：触发器删除是持久数据删除，必须先弹确认、取消则不删除
//（此前是唯一没有确认流的删除入口；对照 SessionList/KeyManagerPanel/QuickCommandPanel）。
// 删除确认走应用内自定义弹窗（utils/dialog），不再用 ElMessageBox。
const { confirmDialogMock, subscribeAppEventsMock, handlers: eventHandlers } = vi.hoisted(() => {
  const handlers: Array<(event: unknown) => void> = [];
  return {
    confirmDialogMock: vi.fn(),
    subscribeAppEventsMock: vi.fn((handler: (event: unknown) => void) => {
      handlers.push(handler);
      return Promise.resolve(() => {
        const i = handlers.indexOf(handler);
        if (i >= 0) handlers.splice(i, 1);
      });
    }),
    handlers,
  };
});

vi.mock("../../src/ipc/client", () => ({
  listTriggers: vi.fn().mockResolvedValue([]),
  createTrigger: vi.fn(),
  toggleTrigger: vi.fn(),
  deleteTrigger: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../../src/ipc/events", () => ({ subscribeAppEvents: subscribeAppEventsMock }));
vi.mock("../../src/utils/dialog", () => ({ confirmDialog: confirmDialogMock }));

const trigger = {
  id: "trigger-1",
  name: "prompt-regex",
  enabled: true,
  condition: { RegexAppear: "^\\$" },
  action: { SendText: "clear" },
};

function mountPanel() {
  // 真实 ElementPlus 渲染表格行，测试直接点击行内「删除」按钮
  return mount(TriggerEditor, { global: { plugins: [ElementPlus] } });
}

async function clickDelete() {
  const wrapper = mountPanel();
  await flushPromises();
  const buttons = wrapper.findAll("button");
  const deleteButton = buttons.find((b) => b.text() === "删除");
  expect(deleteButton, "表格行内应渲染「删除」按钮").toBeDefined();
  await deleteButton!.trigger("click");
  await flushPromises();
  return wrapper;
}

describe("TriggerEditor 删除确认（R2-14）", () => {
  beforeEach(() => {
    confirmDialogMock.mockReset();
    vi.mocked(deleteTrigger).mockReset().mockResolvedValue(undefined);
    vi.mocked(listTriggers).mockReset().mockResolvedValue([trigger]);
    eventHandlers.length = 0;
  });

  it("删除前弹出确认，取消则不删除", async () => {
    confirmDialogMock.mockResolvedValueOnce(false); // 用户点取消
    const wrapper = await clickDelete();

    expect(confirmDialogMock).toHaveBeenCalledTimes(1);
    expect(String(confirmDialogMock.mock.calls[0][0].message)).toContain("prompt-regex");
    expect(vi.mocked(deleteTrigger)).not.toHaveBeenCalled();
    // 取消路径不产生错误提示
    expect(wrapper.text()).not.toContain("删除失败");
    wrapper.unmount();
  });

  it("确认后调用 deleteTrigger 并刷新列表", async () => {
    confirmDialogMock.mockResolvedValueOnce(true); // 用户确认
    const wrapper = await clickDelete();

    expect(confirmDialogMock).toHaveBeenCalledTimes(1);
    expect(vi.mocked(deleteTrigger)).toHaveBeenCalledWith("trigger-1");
    expect(vi.mocked(listTriggers).mock.calls.length).toBeGreaterThanOrEqual(2);
    wrapper.unmount();
  });

  it("事件订阅经 unmount 释放", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    expect(eventHandlers.length).toBe(1);
    wrapper.unmount();
    expect(eventHandlers.length).toBe(0);
  });
});
