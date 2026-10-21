import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import SyncInputPanel from "../../src/components/SyncInputPanel.vue";
import { getSyncInputSessions, toggleSyncInput } from "../../src/ipc/client";
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
  getSyncInputSessions: vi.fn(),
  toggleSyncInput: vi.fn(),
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
    current: null,
    connectionState: new Map(),
    refresh: vi.fn().mockResolvedValue(undefined),
  }),
}));

const stubs = {
  "el-button": {
    props: ["disabled"],
    template: '<button :disabled="disabled"><slot /></button>',
  },
  "el-empty": { template: "<div />" },
};

describe("SyncInputPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    eventHandlers.length = 0;
    vi.mocked(getSyncInputSessions).mockResolvedValue([]);
    vi.mocked(toggleSyncInput).mockResolvedValue(undefined);
  });

  it("shows that sync input is off when the backend reports no session group", async () => {
    const wrapper = mount(SyncInputPanel, { global: { stubs } });
    await flushPromises();
    expect(wrapper.get('[data-test="sync-inactive"]').text()).toContain("未开启");
    expect(wrapper.find('[data-test="sync-active"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("enables sync input for the checked sessions", async () => {
    const wrapper = mount(SyncInputPanel, { global: { stubs } });
    await flushPromises();

    await wrapper.get('[data-test="sync-session-session-2"]').setValue(true);
    await wrapper.get('[data-test="sync-enable"]').trigger("click");
    await flushPromises();

    expect(toggleSyncInput).toHaveBeenCalledWith(["session-2"]);
    wrapper.unmount();
  });

  it("keeps the enable action disabled until a session is picked", async () => {
    const wrapper = mount(SyncInputPanel, { global: { stubs } });
    await flushPromises();

    const enable = wrapper.get('[data-test="sync-enable"]');
    expect(enable.attributes("disabled")).toBeDefined();

    await wrapper.get('[data-test="sync-session-session-2"]').setValue(true);
    await flushPromises();
    expect(wrapper.get('[data-test="sync-enable"]').attributes("disabled")).toBeUndefined();
    expect(toggleSyncInput).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("turns sync input off with an empty session group", async () => {
    vi.mocked(getSyncInputSessions).mockResolvedValue(["session-1", "session-2"]);
    const wrapper = mount(SyncInputPanel, { global: { stubs } });
    await flushPromises();
    expect(wrapper.get('[data-test="sync-active"]').text()).toContain("alpha");

    await wrapper.get('[data-test="sync-disable"]').trigger("click");
    await flushPromises();

    expect(toggleSyncInput).toHaveBeenCalledWith([]);
    expect(wrapper.get('[data-test="sync-inactive"]').text()).toContain("未开启");
    wrapper.unmount();
  });

  it("follows the backend when the sync group changes", async () => {
    const wrapper = mount(SyncInputPanel, { global: { stubs } });
    await flushPromises();

    deliverEvent({ SyncInputSessionsChanged: { session_ids: ["session-1"] } });
    await flushPromises();

    expect(wrapper.get('[data-test="sync-active"]').text()).toContain("alpha");
    wrapper.unmount();
  });

  it("surfaces a per-session sync failure instead of swallowing it", async () => {
    const wrapper = mount(SyncInputPanel, { global: { stubs } });
    await flushPromises();

    deliverEvent({ SyncInputSendFailed: { session_id: "session-2", error: "连接已断开" } });
    await flushPromises();

    expect(wrapper.get('[data-test="sync-error"]').text()).toContain("beta");
    expect(wrapper.get('[data-test="sync-error"]').text()).toContain("连接已断开");
    wrapper.unmount();
  });

  it("releases the event subscription on unmount", async () => {
    const wrapper = mount(SyncInputPanel, { global: { stubs } });
    await flushPromises();
    wrapper.unmount();
    expect(eventHandlers.length).toBe(0);
  });
});
