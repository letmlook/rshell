import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import ScriptPanel from "../../src/components/ScriptPanel.vue";
import { executeScript } from "../../src/ipc/client";
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
  executeScript: vi.fn(),
  ipcErrorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));
vi.mock("../../src/ipc/events", () => ({ subscribeAppEvents: subscribeAppEventsMock }));
vi.mock("../../src/stores/sessions", () => ({
  useSessionsStore: () => ({
    items: [{ id: "session-1", name: "alpha", host: "10.0.0.1", port: 22 }],
    currentId: "session-1",
    current: null,
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
  "el-button": { template: '<button type="button"><slot /></button>' },
};

describe("ScriptPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    eventHandlers.length = 0;
    vi.mocked(executeScript).mockResolvedValue(undefined);
  });

  it("runs the script against the selected session", async () => {
    const wrapper = mount(ScriptPanel, { global: { stubs } });
    await flushPromises();

    await wrapper.get('[data-test="script-code"]').setValue('rshell_log("hi");');
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(executeScript).toHaveBeenCalledWith('rshell_log("hi");', "session-1");
    wrapper.unmount();
  });

  it("refuses to run an empty script", async () => {
    const wrapper = mount(ScriptPanel, { global: { stubs } });
    await flushPromises();

    await wrapper.get('[data-test="script-code"]').setValue("   ");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(executeScript).not.toHaveBeenCalled();
    expect(wrapper.get('[data-test="script-error"]').text()).toContain("脚本内容为空");
    wrapper.unmount();
  });

  it("shows the script output published by the backend", async () => {
    const wrapper = mount(ScriptPanel, { global: { stubs } });
    await flushPromises();

    deliverEvent({
      ScriptFinished: {
        session_id: "session-1",
        result: { success: true, output: "会话数: 2", error: null },
      },
    });
    await flushPromises();

    expect(wrapper.get('[data-test="script-result"]').text()).toContain("执行完成");
    expect(wrapper.get('[data-test="script-result"]').text()).toContain("会话数: 2");
    wrapper.unmount();
  });

  it("reports a failed script with the real reason", async () => {
    vi.mocked(executeScript).mockRejectedValue(new Error("Unknown function: rshell_nope"));
    const wrapper = mount(ScriptPanel, { global: { stubs } });
    await flushPromises();

    await wrapper.get('[data-test="script-code"]').setValue("rshell_nope();");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(wrapper.get('[data-test="script-error"]').text()).toContain("Unknown function");
    wrapper.unmount();
  });

  it("states the host API limits instead of implying a full scripting IDE", async () => {
    const wrapper = mount(ScriptPanel, { global: { stubs } });
    await flushPromises();
    const text = wrapper.text();
    expect(text).toContain("rshell_send");
    expect(text).toContain("没有文件读写");
    wrapper.unmount();
  });

  it("releases the event subscription on unmount", async () => {
    const wrapper = mount(ScriptPanel, { global: { stubs } });
    await flushPromises();
    wrapper.unmount();
    expect(eventHandlers.length).toBe(0);
  });
});
