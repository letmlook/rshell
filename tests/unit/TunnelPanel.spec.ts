import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import TunnelPanel from "../../src/components/TunnelPanel.vue";
import {
  listTunnels,
  listPendingTunnels,
  createTunnel,
  suspendTunnel,
  resumeTunnel,
  restoreTunnel,
} from "../../src/ipc/client";
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
  suspendTunnel: vi.fn(),
  resumeTunnel: vi.fn(),
  restoreTunnel: vi.fn(),
}));

// 面板的「连接」下拉来自会话 store：这里用假 store 顶掉，
// 避免测试去依赖真实的 Pinia 安装与会话列表 IPC。
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
    "el-select": {
      props: ["modelValue"],
      emits: ["update:modelValue"],
      template:
        '<select :value="modelValue" @change="$emit(\'update:modelValue\', $event.target.value)"><slot /></select>',
    },
    "el-form-item": shell,
    "el-form": shell,
    "el-input": { template: "<input />" },
    "el-empty": { template: "<div />" },
  };

  beforeEach(() => {
    vi.mocked(listPendingTunnels).mockResolvedValue({ rules: [], unsupported: [] });
    vi.mocked(listTunnels).mockReset().mockResolvedValue([]);
    eventHandlers.length = 0;
  });

  it("offers only forwarding modes backed by SSH", () => {
    const wrapper = mount(TunnelPanel, { global: { stubs } });
    const choices = wrapper
      .get('[data-test="tunnel-type"]')
      .findAll("option")
      .map((option) => option.attributes("value"));
    expect(choices).toEqual(["Local", "Dynamic"]);
    wrapper.unmount();
  });

  // 连接选择器此前是裸 uuid 输入框：用户得自己抄 id。
  // 现在是从连接列表里选，选项与真实连接一一对应。
  it("picks the session from the connection list instead of a raw uuid field", () => {
    const wrapper = mount(TunnelPanel, { global: { stubs } });
    const options = wrapper
      .get('[data-test="tunnel-session"]')
      .findAll("option")
      .map((option) => option.attributes("value"));
    expect(options).toEqual(["session-1", "session-2"]);
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
    // 断言用户真正看到的文案（此前这条断言落在 el-table 桩打印的行 JSON 上，
    // 是桩的副产品而不是界面行为）
    expect(wrapper.text()).toContain("运行中");
    expect(wrapper.text()).not.toContain("错误");
    expect(vi.mocked(listTunnels).mock.calls.length).toBe(1);

    // 模拟后端断开会话：列表状态已变 Error，并广播 ActiveTunnelsChanged
    vi.mocked(listTunnels).mockResolvedValue([
      { ...activeTunnel, state: { Error: "session disconnected" } },
    ]);
    deliverEvent("ActiveTunnelsChanged");
    await flushPromises();

    expect(vi.mocked(listTunnels).mock.calls.length).toBeGreaterThanOrEqual(2);
    expect(wrapper.text()).toContain("错误");
    expect(wrapper.text()).toContain("session disconnected");
    expect(wrapper.text()).not.toContain("运行中");
    wrapper.unmount();
    expect(eventHandlers.length).toBe(0);
  });

  // 挂起/恢复此前没有界面入口：后端 R3-02 把「假挂起」修成了真实现，
  // 但界面只有「关闭」，用户根本够不到这两个能力。
  it("suspends an active tunnel", async () => {
    vi.mocked(listTunnels).mockResolvedValue([activeTunnel]);
    const wrapper = mount(TunnelPanel, { global: { stubs } });
    await flushPromises();

    const suspend = wrapper.findAll('[data-test="tunnel-suspend"]').find(
      (button) => button.attributes("disabled") === undefined,
    );
    expect(suspend, "运行中的隧道必须可以挂起").toBeDefined();
    await suspend!.trigger("click");
    await flushPromises();

    expect(suspendTunnel).toHaveBeenCalledWith("tunnel-1");
    wrapper.unmount();
  });

  it("resumes a suspended tunnel and only offers resume in that state", async () => {
    vi.mocked(listTunnels).mockResolvedValue([
      { ...activeTunnel, state: "Suspended" as const },
    ]);
    const wrapper = mount(TunnelPanel, { global: { stubs } });
    await flushPromises();

    const resume = wrapper.findAll('[data-test="tunnel-resume"]').find(
      (button) => button.attributes("disabled") === undefined,
    );
    expect(resume, "挂起的隧道必须可以恢复").toBeDefined();
    await resume!.trigger("click");
    await flushPromises();

    expect(resumeTunnel).toHaveBeenCalledWith("tunnel-1");
    wrapper.unmount();
  });

  it("隧道列表用卡片渲染：三个操作按钮与状态同处一个条目，不依赖表格列宽", async () => {
    // 表格三列合计 430px 放不进默认侧栏（280px），「挂起/恢复/关闭」会被裁在
    // 视口外——用户必须先发现横向滚动条才点得到操作。列表化后不再有列宽约束。
    vi.mocked(listTunnels).mockResolvedValue([activeTunnel]);
    const wrapper = mount(TunnelPanel, { global: { stubs } });
    await flushPromises();

    expect(wrapper.find('[data-test="tunnel-list"]').exists()).toBe(true);
    expect(wrapper.find("table").exists(), "隧道列表不得再渲染成表格").toBe(false);

    const item = wrapper.findAll('[data-test="tunnel-list"] > li');
    expect(item).toHaveLength(1);
    const entry = item[0];
    // 状态、规则、三个操作按钮都在同一条目里
    expect(entry.text()).toContain("运行中");
    expect(entry.text()).toContain("127.0.0.1:8080");
    expect(entry.find('[data-test="tunnel-suspend"]').exists()).toBe(true);
    expect(entry.find('[data-test="tunnel-resume"]').exists()).toBe(true);
    expect(entry.find('[data-test="tunnel-close"]').exists()).toBe(true);
    wrapper.unmount();
  });

  it("shows the error reason of a broken tunnel instead of a bare variant name", async () => {
    vi.mocked(listTunnels).mockResolvedValue([
      { ...activeTunnel, state: { Error: "session disconnected" } },
    ]);
    const wrapper = mount(TunnelPanel, { global: { stubs } });
    await flushPromises();

    expect(wrapper.text()).toContain("session disconnected");
    wrapper.unmount();
  });

  // 重启后不自动抢占端口：可恢复的规则被记在 tunnels.toml 里。
  // 此前界面只显示被跳过的旧规则，可恢复的规则从不显示 —— 用户没有任何入口。
  it("lists restorable pending rules and restores one on request", async () => {
    vi.mocked(listPendingTunnels).mockResolvedValue({
      rules: [
        [
          "session-1",
          {
            bind_address: "127.0.0.1",
            bind_port: 8080,
            remote_host: "localhost",
            remote_port: 80,
            direction: "Local",
            allow_non_loopback: false,
          },
        ],
      ],
      unsupported: [],
    });
    const wrapper = mount(TunnelPanel, { global: { stubs } });
    await flushPromises();

    expect(wrapper.get('[data-test="tunnel-pending"]').text()).toContain("127.0.0.1:8080");
    await wrapper.get('[data-test="tunnel-restore"]').trigger("click");
    await flushPromises();

    expect(restoreTunnel).toHaveBeenCalledWith(
      "session-1",
      expect.objectContaining({ bind_port: 8080 }),
    );
    wrapper.unmount();
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

// R3-16：`:8080` 解析出 host: ""，前端 `host !== ""` 守卫使暴露确认不出现、
// allow_non_loopback 永远为 false，而后端 is_loopback_bind_address("") 判 false
// 并要求该标志 —— 这条输入永远建不出隧道，提示还指向界面上不存在的选项。
// 现在提交前就要求写明主机名。保守判定不变：`""` 既不算回环，也绝不放行到 bind
// （`TcpListener::bind(":port")` 会绑定所有网卡，正是后端要挡的）。
describe("TunnelPanel 监听地址校验", () => {
  const stubs = {
    // 真实 <form> 才能把 @submit 送回 add()
    "el-form": { emits: ["submit"], template: '<form @submit.prevent="$emit(\'submit\', $event)"><slot /></form>' },
    "el-form-item": { props: ["label"], template: '<div :data-test="\'item-\' + label"><slot /></div>' },
    "el-input": {
      props: ["modelValue"],
      emits: ["update:modelValue"],
      template: '<input :value="modelValue" @input="$emit(\'update:modelValue\', $event.target.value)" />',
    },
    "el-select": {
      props: ["modelValue"],
      emits: ["update:modelValue"],
      template:
        '<select :value="modelValue" @change="$emit(\'update:modelValue\', $event.target.value)"><slot /></select>',
    },
    "el-option": { props: ["label", "value"], template: '<option :value="value">{{ label }}</option>' },
    "el-button": { template: "<button><slot /></button>" },
    "el-empty": { template: "<div />" },
    "el-table": { props: ["data"], template: "<div />" },
    "el-table-column": { template: "<div />" },
  };

  beforeEach(() => {
    vi.mocked(listPendingTunnels).mockResolvedValue({ rules: [], unsupported: [] });
    vi.mocked(listTunnels).mockReset().mockResolvedValue([]);
    vi.mocked(createTunnel).mockReset().mockResolvedValue(undefined);
    eventHandlers.length = 0;
  });

  async function submitBind(bind: string) {
    const wrapper = mount(TunnelPanel, { global: { stubs } });
    await flushPromises();
    // 连接也要有，否则 add() 会先因「请先在主视图选择会话」返回
    await wrapper.get('[data-test="tunnel-session"]').setValue("session-1");
    await wrapper.get('[data-test="item-监听"]').get("input").setValue(bind);
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    return wrapper;
  }

  it(":8080（空 host）给出行内错误且不发起 createTunnel", async () => {
    const wrapper = await submitBind(":8080");
    const message = wrapper.get('[data-test="tunnel-bind-error"]').text();
    expect(message).toContain("监听地址必须写明主机名");
    expect(createTunnel).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("提交前不显示行内错误，避免面板刚打开就飘红字", async () => {
    const wrapper = mount(TunnelPanel, { global: { stubs } });
    await flushPromises();
    expect(wrapper.find('[data-test="tunnel-bind-error"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("写明主机名后不再报错，正常的回环监听照常创建", async () => {
    const wrapper = await submitBind("127.0.0.1:8080");
    expect(wrapper.find('[data-test="tunnel-bind-error"]').exists()).toBe(false);
    expect(createTunnel).toHaveBeenCalledTimes(1);
    expect(vi.mocked(createTunnel).mock.calls[0][0]).toBe("session-1");
    expect(vi.mocked(createTunnel).mock.calls[0][1]).toMatchObject({
      bind_address: "127.0.0.1",
      bind_port: 8080,
      allow_non_loopback: false,
    });
    wrapper.unmount();
  });

  it("补上主机名后行内错误自动消失（错误跟着输入走，不粘住）", async () => {
    const wrapper = await submitBind(":8080");
    expect(wrapper.find('[data-test="tunnel-bind-error"]').exists()).toBe(true);
    await wrapper.get('[data-test="item-监听"]').get("input").setValue("127.0.0.1:8080");
    await flushPromises();
    expect(wrapper.find('[data-test="tunnel-bind-error"]').exists()).toBe(false);
    wrapper.unmount();
  });
});
