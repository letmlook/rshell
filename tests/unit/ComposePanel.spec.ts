import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent, h, inject, provide } from "vue";
import ComposePanel from "../../src/components/ComposePanel.vue";
import { sendComposeText } from "../../src/ipc/client";
import type { AppEvent } from "../../src/ipc/types";

const { subscribeAppEventsMock, handlers: eventHandlers } = vi.hoisted(() => {
  const handlers: Array<(event: AppEvent) => void> = [];
  return {
    subscribeAppEventsMock: vi.fn((handler: (event: AppEvent) => void) => {
      handlers.push(handler);
      return Promise.resolve(() => {
        const i = handlers.indexOf(handler);
        if (i >= 0) handlers.splice(i, 1);
      });
    }),
    handlers,
  };
});
const deliverEvent = (event: AppEvent) => {
  for (const handler of [...eventHandlers]) handler(event);
};

vi.mock("../../src/ipc/client", () => ({
  sendComposeText: vi.fn(),
  ipcErrorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));
vi.mock("../../src/ipc/events", () => ({ subscribeAppEvents: subscribeAppEventsMock }));
vi.mock("../../src/stores/sessions", () => ({
  useSessionsStore: () => ({
    items: [
      { id: "session-1", name: "alpha", host: "10.0.0.1", port: 22 },
      { id: "session-2", name: "beta", host: "10.0.0.2", port: 22 },
    ],
    currentId: "session-1",
    current: { id: "session-1", name: "alpha", host: "10.0.0.1", port: 22 },
    connectionState: new Map(),
    refresh: vi.fn().mockResolvedValue(undefined),
  }),
}));

const stubs = {
  "el-form": {
    emits: ["submit"],
    template: '<form @submit.prevent="$emit(\'submit\', $event)"><slot /></form>',
  },
  "el-form-item": { props: ["label"], template: '<div><slot /></div>' },
  "el-input": {
    props: ["modelValue", "type"],
    emits: ["update:modelValue"],
    template:
      '<textarea :value="modelValue" @input="$emit(\'update:modelValue\', $event.target.value)" />',
  },
  "el-select": { template: "<select><slot /></select>" },
  "el-option": { props: ["label", "value"], template: '<option :value="value">{{ label }}</option>' },
  // 单选组要真的能改 modelValue，否则「全部会话」永远选不上。
  // 真实 Element Plus 靠 provide/inject 在 group 与 radio 之间通信，
  // 这里用同样的机制还原，避免测试点不动 radio。
  "el-radio-group": defineComponent({
    name: "el-radio-group",
    props: ["modelValue"],
    emits: ["update:modelValue"],
    setup(_props, { emit, slots }) {
      provide("rs-radio-group", {
        select: (value: string) => emit("update:modelValue", value),
      });
      return () => h("div", slots.default?.());
    },
  }),
  "el-radio": defineComponent({
    name: "el-radio",
    props: ["value"],
    setup(props, { slots }) {
      const group = inject<{ select: (value: string) => void } | null>(
        "rs-radio-group",
        null,
      );
      return () =>
        h(
          "label",
          {
            onClick: () => group?.select(props.value as string),
          },
          [slots.default?.(), h("input", { type: "radio", value: props.value })],
        );
    },
  }),
  "el-button": { template: '<button type="button"><slot /></button>' },
};

describe("ComposePanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    eventHandlers.length = 0;
    vi.mocked(sendComposeText).mockResolvedValue(undefined);
  });

  it("sends to the current session and passes the active session id", async () => {
    const wrapper = mount(ComposePanel, { global: { stubs } });
    await flushPromises();

    await wrapper.get('[data-test="compose-content"]').setValue("uptime");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(sendComposeText).toHaveBeenCalledWith("uptime", "CurrentSession", "session-1");
    expect(wrapper.get('[data-test="compose-sent"]').text()).toContain("已发送");
    wrapper.unmount();
  });

  it("sends to all sessions", async () => {
    const wrapper = mount(ComposePanel, { global: { stubs } });
    await flushPromises();

    await wrapper.get('[data-test="compose-target-all"]').trigger("click");
    await wrapper.get('[data-test="compose-content"]').setValue("uptime");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(sendComposeText).toHaveBeenCalledWith("uptime", "AllSessions", null);
    wrapper.unmount();
  });

  it("refuses to send empty content", async () => {
    const wrapper = mount(ComposePanel, { global: { stubs } });
    await flushPromises();

    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(sendComposeText).not.toHaveBeenCalled();
    expect(wrapper.get('[data-test="compose-error"]').text()).toContain("请先输入");
    wrapper.unmount();
  });

  it("reports the backend failure instead of claiming success", async () => {
    vi.mocked(sendComposeText).mockRejectedValue(
      new Error("Compose send failed for all 1 target session(s)"),
    );
    const wrapper = mount(ComposePanel, { global: { stubs } });
    await flushPromises();

    await wrapper.get('[data-test="compose-content"]').setValue("uptime");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(wrapper.find('[data-test="compose-sent"]').exists()).toBe(false);
    expect(wrapper.get('[data-test="compose-error"]').text()).toContain("Compose send failed");
    wrapper.unmount();
  });

  it("lists per-session failures published by the backend", async () => {
    const wrapper = mount(ComposePanel, { global: { stubs } });
    await flushPromises();

    deliverEvent({ ComposeSendFailed: { session_id: "session-2", error: "连接已断开" } });
    await flushPromises();

    expect(wrapper.get('[data-test="compose-failures"]').text()).toContain("beta");
    expect(wrapper.get('[data-test="compose-failures"]').text()).toContain("连接已断开");
    wrapper.unmount();
  });

  it("releases the event subscription on unmount", async () => {
    const wrapper = mount(ComposePanel, { global: { stubs } });
    await flushPromises();
    wrapper.unmount();
    expect(eventHandlers.length).toBe(0);
  });
});
