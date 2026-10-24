import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent, h, inject, provide } from "vue";
import QuickCommandPanel from "../../src/components/QuickCommandPanel.vue";
import {
  createQuickCommand,
  executeQuickCommand,
  listQuickCommands,
} from "../../src/ipc/client";

const { subscribeAppEventsMock, handlers: eventHandlers } = vi.hoisted(() => {
  const handlers: Array<(event: unknown) => void> = [];
  return {
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
  listQuickCommands: vi.fn().mockResolvedValue([]),
  createQuickCommand: vi.fn(),
  deleteQuickCommand: vi.fn(),
  executeQuickCommand: vi.fn(),
}));
vi.mock("../../src/ipc/events", () => ({ subscribeAppEvents: subscribeAppEventsMock }));
vi.mock("../../src/utils/dialog", () => ({ confirmDialog: vi.fn() }));
vi.mock("../../src/stores/sessions", () => ({
  useSessionsStore: () => ({
    items: [
      { id: "session-1", name: "alpha", host: "10.0.0.1", port: 22 },
      { id: "session-2", name: "beta", host: "10.0.0.2", port: 22 },
    ],
    currentId: "session-1",
    current: null,
    connectionState: new Map([
      ["session-1", "connected"],
      ["session-2", "connected"],
    ]),
    refresh: vi.fn().mockResolvedValue(undefined),
  }),
}));

/** 下拉需要真的能改 modelValue，真实 Element Plus 的 popper 在测试里点不到 */
const stubs = {
  "el-input": {
    props: ["modelValue"],
    emits: ["update:modelValue"],
    template:
      '<input :value="modelValue" @input="$emit(\'update:modelValue\', $event.target.value)" />',
  },
  "el-select": defineComponent({
    name: "el-select",
    // multiple 必须是 Boolean 类型：数组式 props 声明下裸属性会变成空串（falsy）
    props: { modelValue: null, multiple: Boolean },
    emits: ["update:modelValue"],
    setup(props, { emit, slots }) {
      provide("rs-select", {
        choose: (value: string) => {
          // 多选：追加而不是替换
          if (props.multiple) {
            const current = Array.isArray(props.modelValue) ? props.modelValue : [];
            emit("update:modelValue", [...current, value]);
            return;
          }
          emit("update:modelValue", value);
        },
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
      return () =>
        h(
          "button",
          {
            type: "button",
            class: "el-option",
            onClick: () => select?.choose(props.value as string),
          },
          [slots.default?.(), props.label],
        );
    },
  }),
  "el-checkbox": { template: "<span />" },
  "el-button": { template: "<button><slot /></button>" },
  "el-empty": { props: ["description"], template: '<div>{{ description }}</div>' },
};

function mountPanel() {
  return mount(QuickCommandPanel, { global: { stubs } });
}

describe("QuickCommandPanel 作用范围", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    eventHandlers.length = 0;
    vi.mocked(createQuickCommand).mockResolvedValue(undefined);
    vi.mocked(executeQuickCommand).mockResolvedValue(undefined);
  });

  async function fillForm(wrapper: ReturnType<typeof mountPanel>) {
    await wrapper.findAll("input")[0].setValue("uptime");
    await wrapper.findAll("input")[1].setValue("uptime -p");
  }

  it("默认用当前会话作为作用范围", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    await fillForm(wrapper);
    const add = wrapper.findAll("button").find((button) => button.text() === "添加");
    await add!.trigger("click");
    await flushPromises();

    expect(vi.mocked(createQuickCommand)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(createQuickCommand).mock.calls[0][0].scope).toBe("CurrentSession");
    wrapper.unmount();
  });

  it("可以创建「所有已连接会话」范围", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    await fillForm(wrapper);
    const scopeSelect = wrapper.get('[data-test="quick-command-scope"]');
    const allOption = scopeSelect
      .findAll("button.el-option")
      .find((button) => button.text().includes("所有已连接会话"));
    await allOption!.trigger("click");
    await flushPromises();

    const add = wrapper.findAll("button").find((button) => button.text() === "添加");
    await add!.trigger("click");
    await flushPromises();

    expect(vi.mocked(createQuickCommand).mock.calls[0][0].scope).toBe("AllSessions");
    wrapper.unmount();
  });

  it("可以创建「选定会话」范围并带上勾选的会话", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    await fillForm(wrapper);
    const scopeSelect = wrapper.get('[data-test="quick-command-scope"]');
    const selectedOption = scopeSelect
      .findAll("button.el-option")
      .find((button) => button.text().includes("选定会话"));
    await selectedOption!.trigger("click");
    await flushPromises();

    // 选定会话时会出现目标会话下拉
    const targets = wrapper.find('[data-test="quick-command-targets"]');
    expect(targets.exists()).toBe(true);
    const beta = targets
      .findAll("button.el-option")
      .find((button) => button.text().includes("beta"));
    await beta!.trigger("click");
    await flushPromises();

    const add = wrapper.findAll("button").find((button) => button.text() === "添加");
    await add!.trigger("click");
    await flushPromises();

    expect(vi.mocked(createQuickCommand).mock.calls[0][0].scope).toEqual({
      SelectedSessions: ["session-2"],
    });
    wrapper.unmount();
  });

  it("「选定会话」范围下没选会话时不创建", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    await fillForm(wrapper);
    const scopeSelect = wrapper.get('[data-test="quick-command-scope"]');
    const selectedOption = scopeSelect
      .findAll("button.el-option")
      .find((button) => button.text().includes("选定会话"));
    await selectedOption!.trigger("click");
    await flushPromises();

    const add = wrapper.findAll("button").find((button) => button.text() === "添加");
    await add!.trigger("click");
    await flushPromises();

    expect(vi.mocked(createQuickCommand)).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("请至少选择一个目标会话");
    wrapper.unmount();
  });
});

// 热键：后端存了 hotkey 字段，界面此前连输入框都没有。
describe("QuickCommandPanel 热键", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    eventHandlers.length = 0;
    vi.mocked(createQuickCommand).mockResolvedValue(undefined);
    vi.mocked(executeQuickCommand).mockResolvedValue(undefined);
  });

  it("可以给快速命令绑定热键", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    await wrapper.findAll("input")[0].setValue("uptime");
    await wrapper.findAll("input")[1].setValue("uptime -p");
    await wrapper.get('[data-test="quick-command-hotkey"]').setValue("Ctrl+Shift+1");
    const add = wrapper.findAll("button").find((button) => button.text() === "添加");
    await add!.trigger("click");
    await flushPromises();

    expect(vi.mocked(createQuickCommand).mock.calls[0][0].hotkey).toBe("Ctrl+Shift+1");
    wrapper.unmount();
  });

  it("热键写法无法识别时不创建，并说明原因", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    await wrapper.findAll("input")[0].setValue("uptime");
    await wrapper.findAll("input")[1].setValue("uptime -p");
    await wrapper.get('[data-test="quick-command-hotkey"]').setValue("Ctrl+abc");
    const add = wrapper.findAll("button").find((button) => button.text() === "添加");
    await add!.trigger("click");
    await flushPromises();

    expect(vi.mocked(createQuickCommand)).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("热键写法无法识别");
    wrapper.unmount();
  });

  it("不填热键时存 null（表示不绑定）", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    await wrapper.findAll("input")[0].setValue("uptime");
    await wrapper.findAll("input")[1].setValue("uptime -p");
    const add = wrapper.findAll("button").find((button) => button.text() === "添加");
    await add!.trigger("click");
    await flushPromises();

    expect(vi.mocked(createQuickCommand).mock.calls[0][0].hotkey).toBeNull();
    wrapper.unmount();
  });
});

// 后端 QuickCommand.group / description 此前恒为 null / 空串——
// 字段存在但没有任何界面能设置，等于在声明不存在的能力。
describe("QuickCommandPanel 分组与说明", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    eventHandlers.length = 0;
    vi.mocked(createQuickCommand).mockResolvedValue(undefined);
    vi.mocked(executeQuickCommand).mockResolvedValue(undefined);
  });

  it("可以给快速命令填分组与说明", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    await wrapper.findAll("input")[0].setValue("uptime");
    await wrapper.findAll("input")[1].setValue("uptime -p");
    await wrapper.get('[data-test="quick-command-group"]').setValue("运维");
    await wrapper.get('[data-test="quick-command-description"]').setValue("看机器负载");
    const add = wrapper.findAll("button").find((button) => button.text() === "添加");
    await add!.trigger("click");
    await flushPromises();

    const created = vi.mocked(createQuickCommand).mock.calls[0][0];
    expect(created.group).toBe("运维");
    expect(created.description).toBe("看机器负载");
    wrapper.unmount();
  });

  it("留空时分组为 null、说明为空串", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    await wrapper.findAll("input")[0].setValue("uptime");
    await wrapper.findAll("input")[1].setValue("uptime -p");
    const add = wrapper.findAll("button").find((button) => button.text() === "添加");
    await add!.trigger("click");
    await flushPromises();

    const created = vi.mocked(createQuickCommand).mock.calls[0][0];
    expect(created.group).toBeNull();
    expect(created.description).toBe("");
    wrapper.unmount();
  });

  it("按分组名把命令聚在一起", async () => {
    vi.mocked(listQuickCommands).mockResolvedValue([
      {
        id: "q1",
        name: "uptime",
        command: "uptime",
        send_enter: true,
        description: "",
        scope: "AllSessions",
        hotkey: null,
        group: "运维",
      },
      {
        id: "q2",
        name: "df",
        command: "df -h",
        send_enter: true,
        description: "",
        scope: "AllSessions",
        hotkey: null,
        group: "运维",
      },
      {
        id: "q3",
        name: "who",
        command: "who",
        send_enter: true,
        description: "",
        scope: "AllSessions",
        hotkey: null,
        group: null,
      },
    ]);
    const wrapper = mountPanel();
    await flushPromises();

    const groups = wrapper.get('[data-test="quick-command-groups"]');
    const text = groups.text();
    expect(text).toContain("运维");
    expect(text).toContain("uptime");
    expect(text).toContain("df -h");
    expect(text).toContain("who");
    wrapper.unmount();
  });

  // 侧栏只有 200~360px：表格会把「热键」截断、把「执行 / 删除」整列推出
  // 视口（真机实测）。列表必须用卡片渲染，且命令名与命令本身都要看得见。
  it("快速命令列表用卡片渲染，不渲染表格", async () => {
    vi.mocked(listQuickCommands).mockResolvedValue([
      {
        id: "q1",
        name: "uptime",
        command: "uptime -p",
        send_enter: true,
        description: "看机器负载",
        scope: "AllSessions",
        hotkey: "Ctrl+Shift+9",
        group: "运维",
      },
    ]);
    const wrapper = mountPanel();
    await flushPromises();

    expect(wrapper.find(".el-table").exists()).toBe(false);
    const card = wrapper.get('[data-test="quick-command-card-q1"]');
    expect(card.text()).toContain("uptime");
    expect(card.text()).toContain("uptime -p");
    expect(card.text()).toContain("看机器负载");
    // 热键按物理键位回显
    expect(card.text()).toContain("Ctrl+Shift+9");
    // 操作按钮必须在卡片里（表格版本里它们整列在视口外）
    const actions = card.findAll("button").map((button) => button.text());
    expect(actions).toContain("执行");
    expect(actions).toContain("删除");
    wrapper.unmount();
  });
});
