import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import TunnelPanel from "../../src/components/TunnelPanel.vue";
import { listTunnels, listPendingTunnels } from "../../src/ipc/client";
import type { ActiveTunnelInfo, AppEvent } from "../../src/ipc/types";

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
  listTunnels: vi.fn().mockResolvedValue([]),
  listPendingTunnels: vi.fn().mockResolvedValue({ rules: [], unsupported: [] }),
  createTunnel: vi.fn(),
  closeTunnel: vi.fn(),
}));
vi.mock("../../src/ipc/events", () => ({ subscribeAppEvents: subscribeAppEventsMock }));

const activeTunnel: ActiveTunnelInfo = {
  id: "tunnel-1",
  session_id: "session-1",
  rule: {
    bind_address: "127.0.0.1",
    bind_port: 8080,
    remote_host: "localhost",
    remote_port: 80,
    direction: "Local",
    allow_non_loopback: false,
  },
  state: "Active" as const,
  bytes_transferred: 0,
  connections_count: 0,
};

describe("TunnelPanel", () => {
  const shell = { template: "<div><slot /></div>" };
  const stubs = {
    "el-button": { template: "<button><slot /></button>" },
    "el-option": {
      props: ["label", "value"],
      template: '<option :value="value">{{ label }}</option>',
    },
    "el-select": { template: "<select><slot /></select>" },
    "el-form-item": shell,
    "el-form": shell,
    "el-input": { template: "<input />" },
    "el-empty": { template: "<div />" },
    // FileBrowserPane.spec.ts 同款：列桩不渲染 #default（避免无 row 上下文的
    // 渲染错误），行数据由 el-table 桩序列化进文本供断言状态列内容
    "el-table-column": { template: "<div />" },
    "el-table": { props: ["data"], template: "<div data-testid='rows'>{{ JSON.stringify(data) }}<slot /></div>" },
  };

  beforeEach(() => {
    vi.mocked(listPendingTunnels).mockResolvedValue({ rules: [], unsupported: [] });
    vi.mocked(listTunnels).mockReset().mockResolvedValue([]);
    eventHandlers.length = 0;
  });

  it("offers only forwarding modes backed by SSH", () => {
    const wrapper = mount(TunnelPanel, { global: { stubs } });
    const choices = wrapper.findAll("option").map((option) => option.attributes("value"));
    expect(choices).toEqual(["Local", "Dynamic"]);
    wrapper.unmount();
  });

  it("shows a skipped legacy Remote rule to the user", async () => {
    vi.mocked(listPendingTunnels).mockResolvedValue({
      rules: [],
      unsupported: [{ session_id: "legacy-session", reason: "Remote forwarding is not supported" }],
    });
    const wrapper = mount(TunnelPanel, { global: { stubs } });
    await flushPromises();
    expect(wrapper.text()).toContain("旧隧道规则已跳过");
    expect(wrapper.text()).toContain("legacy-session");
    wrapper.unmount();
  });

  // R2-04：后端断开会话时把该会话隧道置 Error 并发布
  // TunnelStateChanged/ActiveTunnelsChanged；面板必须订阅并刷新，
  // 否则状态列停留在旧的 Active 文案。
  it("refreshes the tunnel row to the Error state when the session disconnects (ActiveTunnelsChanged)", async () => {
    vi.mocked(listTunnels).mockResolvedValueOnce([activeTunnel]);
    const wrapper = mount(TunnelPanel, { global: { stubs } });
    await flushPromises();
    expect(wrapper.text()).toContain("Active");
    expect(vi.mocked(listTunnels).mock.calls.length).toBe(1);

    // 模拟后端断开会话：列表状态已变 Error，并广播 ActiveTunnelsChanged
    vi.mocked(listTunnels).mockResolvedValue([
      { ...activeTunnel, state: { Error: "session disconnected" } },
    ]);
    deliverEvent("ActiveTunnelsChanged");
    await flushPromises();

    expect(vi.mocked(listTunnels).mock.calls.length).toBeGreaterThanOrEqual(2);
    expect(wrapper.text()).toContain("Error");
    expect(wrapper.text()).not.toContain("Active");
    wrapper.unmount();
    expect(eventHandlers.length).toBe(0);
  });

  it("also refreshes on TunnelStateChanged and releases the subscription on unmount", async () => {
    vi.mocked(listTunnels).mockResolvedValue([activeTunnel]);
    const wrapper = mount(TunnelPanel, { global: { stubs } });
    await flushPromises();
    expect(vi.mocked(listTunnels).mock.calls.length).toBe(1);

    vi.mocked(listTunnels).mockResolvedValue([
      { ...activeTunnel, state: { Error: "listener aborted" } },
    ]);
    deliverEvent({ TunnelStateChanged: { tunnel_id: "tunnel-1", state: { Error: "listener aborted" } } });
    await flushPromises();
    expect(vi.mocked(listTunnels).mock.calls.length).toBeGreaterThanOrEqual(2);
    expect(wrapper.text()).toContain("listener aborted");

    // 订阅必须在 unmount 时释放
    wrapper.unmount();
    expect(eventHandlers.length).toBe(0);
  });
});
