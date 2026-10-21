import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import ElementPlus from "element-plus";
import { defineComponent, h, inject, provide } from "vue";
import TriggerEditor from "../../src/components/TriggerEditor.vue";
import { createTrigger, deleteTrigger, listTriggers } from "../../src/ipc/client";

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
vi.mock("../../src/utils/pathPicker", () => ({ pickLocalPath: vi.fn() }));

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

// 后端的 TriggerAction 有四种（发送文本 / 显示通知 / 断开连接 / 记录到文件），
// 四种都已实现，但此前界面只能创建「正则 + 发送文本」一种——其余三种没有任何入口。
describe("TriggerEditor 动作与条件覆盖", () => {
  // 真实 ElementPlus 的 el-select 把选项渲染到 teleport 出去的 popper 里，
  // 测试里点不到；这里用受控桩还原 v-model 通道。
  const selectStubs = {
    "el-select": defineComponent({
      name: "el-select",
      props: ["modelValue"],
      emits: ["update:modelValue"],
      setup(_props, { emit, slots }) {
        provide("rs-select", {
          choose: (value: string) => emit("update:modelValue", value),
        });
        return () => h("div", { class: "el-select" }, slots.default?.());
      },
    }),
    "el-option": defineComponent({
      name: "el-option",
      props: ["label", "value"],
      setup(props, { slots }) {
        const select = inject<{ choose: (value: string) => void } | null>(
          "rs-select",
          null,
        );
        // 标签来自 label prop（不是插槽内容），必须渲染出来才能按文本选中
        return () =>
          h(
            "button",
            {
              type: "button",
              class: "el-option",
              "data-value": props.value,
              onClick: () => select?.choose(props.value as string),
            },
            [slots.default?.(), props.label],
          );
      },
    }),
    "el-form": {
      emits: ["submit"],
      template: '<form @submit.prevent="$emit(\'submit\', $event)"><slot /></form>',
    },
  };

  function mountEditor() {
    return mount(TriggerEditor, {
      global: { plugins: [ElementPlus], stubs: selectStubs },
    });
  }

  beforeEach(() => {
    confirmDialogMock.mockReset();
    vi.mocked(listTriggers).mockReset().mockResolvedValue([]);
    vi.mocked(createTrigger).mockReset().mockResolvedValue(undefined);
    eventHandlers.length = 0;
  });

  async function fillPattern(value: string) {
    const wrapper = mountEditor();
    await flushPromises();
    // 名称是必填的（此前恒等于匹配条件，列表里两列内容重复）
    await wrapper.get('[data-test="trigger-name"]').setValue("测试触发器");
    await wrapper.get('[data-test="trigger-pattern"]').setValue(value);
    return wrapper;
  }

  /** 在指定下拉里按可见文本点选一项 */
  async function chooseOption(
    wrapper: ReturnType<typeof mountEditor>,
    selectTest: string,
    text: string,
  ) {
    const option = wrapper
      .get(`[data-test="${selectTest}"]`)
      .findAll("button.el-option")
      .find((button) => button.text().includes(text));
    expect(option, `下拉里应有「${text}」选项`).toBeDefined();
    await option!.trigger("click");
    await flushPromises();
  }

  it("默认创建正则匹配 + 发送文本", async () => {
    const wrapper = await fillPattern("^\\$");
    await wrapper.get('[data-test="trigger-text"]').setValue("clear");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(vi.mocked(createTrigger)).toHaveBeenCalledTimes(1);
    const created = vi.mocked(createTrigger).mock.calls[0][0];
    expect(created.condition).toEqual({ RegexAppear: "^\\$" });
    expect(created.action).toEqual({ SendText: "clear" });
    wrapper.unmount();
  });

  it("可以创建精确匹配条件", async () => {
    const wrapper = await fillPattern("login:");
    await chooseOption(wrapper, "trigger-condition", "包含文本");
    await wrapper.get('[data-test="trigger-text"]').setValue("clear");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    const created = vi.mocked(createTrigger).mock.calls[0][0];
    expect(created.condition).toEqual({ ExactMatch: "login:" });
    wrapper.unmount();
  });

  it("可以创建显示通知动作", async () => {
    const wrapper = await fillPattern("error");
    await chooseOption(wrapper, "trigger-action", "显示通知");
    await wrapper.get('[data-test="trigger-text"]').setValue("出错了");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    const created = vi.mocked(createTrigger).mock.calls[0][0];
    expect(created.action).toEqual({ ShowNotification: "出错了" });
    wrapper.unmount();
  });

  it("可以创建断开连接动作且不需要内容", async () => {
    const wrapper = await fillPattern("bye");
    await chooseOption(wrapper, "trigger-action", "断开连接");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    const created = vi.mocked(createTrigger).mock.calls[0][0];
    expect(created.action).toBe("Disconnect");
    wrapper.unmount();
  });

  it("可以创建记录到文件动作并带上绝对路径", async () => {
    const wrapper = await fillPattern("done");
    await chooseOption(wrapper, "trigger-action", "记录到文件");
    await wrapper.get('[data-test="trigger-log-path"]').setValue("/tmp/rshell-trigger.log");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    const created = vi.mocked(createTrigger).mock.calls[0][0];
    expect(created.action).toEqual({ LogToFile: "/tmp/rshell-trigger.log" });
    wrapper.unmount();
  });

  it("记录到文件必须给出日志文件路径", async () => {
    const wrapper = await fillPattern("done");
    await chooseOption(wrapper, "trigger-action", "记录到文件");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(vi.mocked(createTrigger)).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("请选择要写入的日志文件");
    wrapper.unmount();
  });

  it("匹配条件为空时不创建", async () => {
    const wrapper = mountEditor();
    await flushPromises();
    await wrapper.get('[data-test="trigger-name"]').setValue("测试触发器");
    await wrapper.get('[data-test="trigger-text"]').setValue("clear");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(vi.mocked(createTrigger)).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("匹配条件不能为空");
    wrapper.unmount();
  });

  it("名称留空时回落为匹配条件，不额外报错", async () => {
    const wrapper = mountEditor();
    await flushPromises();
    await wrapper.get('[data-test="trigger-pattern"]').setValue("done");
    await wrapper.get('[data-test="trigger-text"]').setValue("clear");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(vi.mocked(createTrigger)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(createTrigger).mock.calls[0][0].name).toBe("done");
    wrapper.unmount();
  });

  it("填了名称就用名称（不再与正则重复）", async () => {
    const wrapper = mountEditor();
    await flushPromises();
    await wrapper.get('[data-test="trigger-name"]').setValue("登录提示");
    await wrapper.get('[data-test="trigger-pattern"]').setValue("done");
    await wrapper.get('[data-test="trigger-text"]').setValue("clear");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(vi.mocked(createTrigger).mock.calls[0][0].name).toBe("登录提示");
    wrapper.unmount();
  });

  it("动作内容为空时（断开连接除外）不创建", async () => {
    const wrapper = await fillPattern("done");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(vi.mocked(createTrigger)).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("动作内容不能为空");
    wrapper.unmount();
  });
});
