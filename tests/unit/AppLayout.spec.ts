import { describe, expect, it, vi } from "vitest";
import { defineComponent, onMounted } from "vue";
import { flushPromises, mount } from "@vue/test-utils";
import App from "../../src/App.vue";
import { subscribeAppEvents } from "../../src/ipc/events";
import { listTransfers, pauseTransfer, resumeTransfer } from "../../src/ipc/client";
import TransferPanel from "../../src/components/TransferPanel.vue";
import ElementPlus from "element-plus";

const { listTransfersMock, pauseTransferMock, resumeTransferMock } = vi.hoisted(() => ({
  listTransfersMock: vi.fn().mockResolvedValue([]),
  pauseTransferMock: vi.fn().mockResolvedValue(undefined),
  resumeTransferMock: vi.fn().mockResolvedValue(undefined),
}));

// PROB-02：dockview-vue 真实实现不渲染插槽，面板只能经 ready 事件给出的
// api.addPanel 创建。桩用这个可检视的假 api 复现该契约。
const { dockviewApiMocks } = vi.hoisted(() => {
  const panels = new Map<string, { id: string; api: { setActive: ReturnType<typeof vi.fn> } }>();
  const addPanel = vi.fn((options: { id: string }) => {
    const panel = { id: options.id, api: { setActive: vi.fn() } };
    panels.set(options.id, panel);
    return panel;
  });
  return { dockviewApiMocks: { panels, addPanel, getPanel: vi.fn((id: string) => panels.get(id)) } };
});

vi.mock("../../src/components/TerminalPane.vue", () => ({ default: { name: "TerminalPane", template: "<div />" } }));

vi.mock("../../src/stores/sessions", () => ({
  useSessionsStore: () => ({
    currentId: null,
    current: null,
    items: [],
    connectionState: new Map(),
    refresh: vi.fn().mockResolvedValue(undefined),
    subscribeEvents: vi.fn().mockResolvedValue(undefined),
    disposeEvents: vi.fn(),
    connect: vi.fn().mockResolvedValue(undefined),
  }),
}));
vi.mock("../../src/stores/hostKey", () => ({
  useHostKeyStore: () => ({ subscribeEvents: vi.fn().mockResolvedValue(undefined), disposeEvents: vi.fn() }),
}));
vi.mock("../../src/stores/theme", () => ({
  useThemeStore: () => ({ refresh: vi.fn().mockResolvedValue(undefined), subscribeEvents: vi.fn().mockResolvedValue(undefined), disposeEvents: vi.fn() }),
}));
vi.mock("../../src/ipc/client", () => ({
  listTransfers: listTransfersMock,
  pauseTransfer: pauseTransferMock,
  resumeTransfer: resumeTransferMock,
}));
vi.mock("../../src/ipc/events", () => ({ subscribeAppEvents: vi.fn().mockResolvedValue(vi.fn()) }));

describe("App layout", () => {
  // dockview 容器桩：经 ready 事件给出与真实库一致的 api 契约（见文件头部 PROB-02 注释）
  const dockviewStub = defineComponent({
    name: "DockviewVue",
    props: ["components"],
    emits: ["ready"],
    template: "<div data-testid='dockview' />",
    setup(_, { emit }) {
      onMounted(() => emit("ready", { api: dockviewApiMocks }));
    },
  });

  const childStubs = {
    CustomTitleBar: { name: "CustomTitleBar", template: "<header data-testid='titlebar' />" },
    WorkspaceToolbar: {
      name: "WorkspaceToolbar",
      props: ["workspace", "connectionState", "activePanel", "sidebarExpanded"],
      emits: ["select-panel", "toggle-sidebar", "change-workspace"],
      template: "<div data-testid='toolbar' />",
    },
    SidePanel: {
      name: "SidePanel",
      props: ["active", "width", "expanded"],
      emits: ["update:width", "select-session", "open-sftp", "open-terminal"],
      template: "<aside data-testid='side-panel' />",
    },
    StatusBar: { name: "StatusBar", template: "<footer data-testid='statusbar' />" },
    // dockview-vue 导出组件的解析名是编译注入的 __name: "dockview"（dist 实测），
    // 不是导入名 DockviewVue；桩若匹配不上会挂载真实 dockview，jsdom 缺
    // ResizeObserver 使 mounted 钩子抛错并毒化 Vue 调度器，拖垮同文件其余用例。
    // 两个键指向同一桩：script setup 模板内联与否都能按解析名命中。
    DockviewVue: dockviewStub,
    dockview: dockviewStub,
    TerminalPane: { name: "TerminalPane", template: "<div data-testid='terminal-pane' />" },
    TransferWorkspace: { name: "TransferWorkspace", template: "<div data-testid='transfer-workspace' />" },
    TransferPanel: { name: "TransferPanel", template: "<div data-testid='transfer-panel' />" },
    SessionCreateDialog: { name: "SessionCreateDialog", template: "<div />" },
    HostKeyMismatchDialog: { name: "HostKeyMismatchDialog", template: "<div />" },
    TransferQueue: { name: "TransferQueue", template: "<div />" },
    "el-button": { template: "<button />" },
  };

  it("opens and hides the mounted sidebar without destroying its subtree", async () => {
    const wrapper = mount(App, { global: { stubs: childStubs } });
    const sidebar = wrapper.findComponent({ name: "SidePanel" });
    expect(sidebar.props("expanded")).toBe(true);
    const toolbarVm = wrapper.findComponent({ name: "WorkspaceToolbar" }).vm;
    await toolbarVm.$emit("toggle-sidebar", false);
    expect(wrapper.findComponent({ name: "SidePanel" }).exists()).toBe(true);
    expect(wrapper.findComponent({ name: "SidePanel" }).props("expanded")).toBe(false);
  });

  it("shows the backend failure reason in the transfer queue", async () => {
    vi.mocked(listTransfers).mockResolvedValueOnce([{
      id: "transfer-1", session_id: "session-1", direction: "Upload", state: "Failed",
      local_path: "/tmp/file", remote_path: "/remote/file", total_bytes: 10,
      bytes_transferred: 0, speed_bps: 0, error_message: "Permission denied: /remote/file",
    }]);
    const wrapper = mount(App, { global: { plugins: [ElementPlus], stubs: { ...childStubs, TransferPanel: false } } });
    await flushPromises();
    expect(wrapper.findComponent(TransferPanel).text()).toContain("Permission denied: /remote/file");
    wrapper.unmount();
  });

  it("renders one sidebar and no ActivityBar", () => {
    const wrapper = mount(App, { global: { stubs: childStubs } });
    expect(wrapper.find('[data-testid="side-panel"]').exists()).toBe(true);
    expect(wrapper.findComponent({ name: "ActivityBar" }).exists()).toBe(false);
  });

  it("creates the terminal panel through api.addPanel when a session is selected", async () => {
    dockviewApiMocks.panels.clear();
    dockviewApiMocks.addPanel.mockClear();
    const wrapper = mount(App, { global: { stubs: childStubs } });
    // 桩不再渲染插槽：选中会话前终端容器为空，也不存在 TerminalPane
    expect(wrapper.find('[data-testid="dockview"]').exists()).toBe(false);
    await wrapper.findComponent({ name: "SidePanel" }).vm.$emit("select-session", "session-42");
    await flushPromises();
    expect(wrapper.find('[data-testid="dockview"]').exists()).toBe(true);
    // 桩不渲染任何插槽：dockview 挂载后 TerminalPane 也不经插槽出现，
    // 面板只能由 addPanel 创建 —— 桩或 App.vue 回归插槽方案时本断言失败。
    expect(wrapper.find('[data-testid="terminal-pane"]').exists()).toBe(false);
    expect(dockviewApiMocks.addPanel).toHaveBeenCalledTimes(1);
    expect(dockviewApiMocks.addPanel).toHaveBeenCalledWith({
      id: "terminal-session-42",
      component: "terminal",
      params: { sessionId: "session-42" },
    });
    wrapper.unmount();
  });

  it("re-activates the existing panel instead of duplicating it when switching back", async () => {
    dockviewApiMocks.panels.clear();
    dockviewApiMocks.addPanel.mockClear();
    const wrapper = mount(App, { global: { stubs: childStubs } });
    const sidePanel = wrapper.findComponent({ name: "SidePanel" });
    await sidePanel.vm.$emit("select-session", "session-a");
    await flushPromises();
    await sidePanel.vm.$emit("select-session", "session-b");
    await flushPromises();
    await sidePanel.vm.$emit("select-session", "session-a");
    await flushPromises();
    expect(dockviewApiMocks.addPanel).toHaveBeenCalledTimes(2);
    expect(dockviewApiMocks.addPanel).toHaveBeenCalledWith(
      expect.objectContaining({ id: "terminal-session-b", params: { sessionId: "session-b" } }),
    );
    expect(dockviewApiMocks.panels.get("terminal-session-a")?.api.setActive).toHaveBeenCalledOnce();
    wrapper.unmount();
  });

  it("opens the sidebar and changes the selected panel from toolbar intent", async () => {
    const wrapper = mount(App, { global: { stubs: childStubs } });
    const toolbar = wrapper.find('[data-testid="toolbar"]');
    await toolbar.trigger("click");
    const toolbarVm = wrapper.findComponent({ name: "WorkspaceToolbar" }).vm;
    await toolbarVm.$emit("toggle-sidebar", false);
    await toolbarVm.$emit("select-panel", "keys");
    expect(wrapper.findComponent({ name: "SidePanel" }).props("active")).toBe("keys");
  });

  it("unsubscribes the App event listener on each unmount", async () => {
    const firstStop = vi.fn();
    const secondStop = vi.fn();
    vi.mocked(subscribeAppEvents).mockResolvedValueOnce(firstStop).mockResolvedValueOnce(secondStop);
    const first = mount(App, { global: { stubs: childStubs } });
    await flushPromises();
    first.unmount();
    expect(firstStop).toHaveBeenCalledOnce();
    const second = mount(App, { global: { stubs: childStubs } });
    await flushPromises();
    second.unmount();
    expect(secondStop).toHaveBeenCalledOnce();
  });

  it("calls pauseTransfer exactly once for repeated pause clicks and refreshes the queue after success", async () => {
    pauseTransferMock.mockClear();
    listTransfersMock.mockReset();
    listTransfersMock.mockResolvedValue([]);
    const wrapper = mount(App, { global: { plugins: [ElementPlus], stubs: childStubs } });
    const panel = wrapper.findComponent(TransferPanel);
    await panel.vm.$emit("pause", "transfer-1");
    await panel.vm.$emit("pause", "transfer-1");
    await flushPromises();
    expect(vi.mocked(pauseTransfer)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(pauseTransfer)).toHaveBeenCalledWith("transfer-1");
    // After the IPC call resolves, App.vue asks the backend for the latest
    // queue snapshot so the panel reflects the real phase.
    expect(vi.mocked(listTransfers).mock.calls.length).toBeGreaterThanOrEqual(2);
    wrapper.unmount();
  });

  it("surfaces a failure notification when pauseTransfer rejects, without mutating phase", async () => {
    pauseTransferMock.mockReset();
    pauseTransferMock.mockRejectedValueOnce(new Error("transmission already finished"));
    listTransfersMock.mockReset();
    listTransfersMock.mockResolvedValue([]);
    const wrapper = mount(App, { global: { plugins: [ElementPlus], stubs: { ...childStubs, TransferPanel: false } } });
    const panel = wrapper.findComponent(TransferPanel);
    await panel.vm.$emit("pause", "transfer-2");
    await flushPromises();
    const errorBanner = wrapper.find('[data-test="xfer-action-error"]');
    expect(errorBanner.exists()).toBe(true);
    expect(errorBanner.text()).toContain("transmission already finished");
    wrapper.unmount();
  });

  it("routes resume events to resumeTransfer and tracks pending task ids independently", async () => {
    pauseTransferMock.mockClear();
    resumeTransferMock.mockClear();
    listTransfersMock.mockReset();
    listTransfersMock.mockResolvedValue([]);
    const wrapper = mount(App, { global: { plugins: [ElementPlus], stubs: childStubs } });
    const panel = wrapper.findComponent(TransferPanel);
    await panel.vm.$emit("resume", "transfer-3");
    await panel.vm.$emit("pause", "transfer-4");
    await flushPromises();
    expect(vi.mocked(resumeTransfer)).toHaveBeenCalledWith("transfer-3");
    expect(vi.mocked(pauseTransfer)).toHaveBeenCalledWith("transfer-4");
    wrapper.unmount();
  });
});
