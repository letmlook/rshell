import { describe, expect, it, vi } from "vitest";
import { defineComponent, onMounted } from "vue";
import { flushPromises, mount } from "@vue/test-utils";
import App from "../../src/App.vue";
import { subscribeAppEvents } from "../../src/ipc/events";
import { listTransfers, pauseTransfer, resumeTransfer } from "../../src/ipc/client";
import TransferPanel from "../../src/components/TransferPanel.vue";
import ElementPlus from "element-plus";

const { listTransfersMock, pauseTransferMock, resumeTransferMock, cancelTransferMock, removeTransferMock, sessionsStoreMock } = vi.hoisted(() => ({
  listTransfersMock: vi.fn().mockResolvedValue([]),
  pauseTransferMock: vi.fn().mockResolvedValue(undefined),
  resumeTransferMock: vi.fn().mockResolvedValue(undefined),
  cancelTransferMock: vi.fn().mockResolvedValue(undefined),
  removeTransferMock: vi.fn().mockResolvedValue(undefined),
  sessionsStoreMock: { store: null as { items: Array<{ id: string; name?: string }>; [key: string]: unknown } | null },
}));

// PROB-02：dockview-vue 真实实现不渲染插槽，面板只能经 ready 事件给出的
// api.addPanel 创建。桩用这个可检视的假 api 复现该契约；panels 以 getter
// 复现真实 DockviewApi.panels（App.vue 的孤儿面板清理遍历该列表）。
const { dockviewApiMocks } = vi.hoisted(() => {
  const panelMap = new Map<
    string,
    {
      id: string;
      api: {
        setActive: ReturnType<typeof vi.fn>;
        setTitle: ReturnType<typeof vi.fn>;
        close: ReturnType<typeof vi.fn>;
      };
    }
  >();
  const addPanel = vi.fn((options: { id: string }) => {
    const panel = { id: options.id, api: { setActive: vi.fn(), setTitle: vi.fn(), close: vi.fn() } };
    panelMap.set(options.id, panel);
    return panel;
  });
  return {
    dockviewApiMocks: {
      panelMap,
      get panels() {
        return Array.from(panelMap.values());
      },
      addPanel,
      getPanel: vi.fn((id: string) => panelMap.get(id)),
    },
  };
});

vi.mock("../../src/components/TerminalPane.vue", () => ({ default: { name: "TerminalPane", template: "<div />" } }));

vi.mock("../../src/stores/sessions", async () => {
  // 共享 reactive 实例：测试可以直接改 items 触发 App.vue 的会话列表 watch
  const { reactive } = await import("vue");
  const store = reactive({
    currentId: null as string | null,
    current: null,
    items: [] as Array<{ id: string; name?: string }>,
    connectionState: new Map<string, string>(),
    refresh: vi.fn(async () => {}),
    subscribeEvents: vi.fn(async () => {}),
    disposeEvents: vi.fn(),
    connect: vi.fn(async () => {}),
    delete: vi.fn(async () => {}),
  });
  sessionsStoreMock.store = store;
  return { useSessionsStore: () => store };
});
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
  cancelTransfer: cancelTransferMock,
  removeTransfer: removeTransferMock,
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
    dockviewApiMocks.panelMap.clear();
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
    // 回归：不传 title 时 dockview 用面板 id 当标签（`terminal-<uuid>`，45 字符）
    expect(dockviewApiMocks.addPanel).toHaveBeenCalledWith({
      id: "terminal-session-42",
      component: "terminal",
      title: "session-",
      params: { sessionId: "session-42" },
    });
    wrapper.unmount();
  });

  it("re-activates the existing panel instead of duplicating it when switching back", async () => {
    dockviewApiMocks.panelMap.clear();
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
    expect(dockviewApiMocks.panelMap.get("terminal-session-a")?.api.setActive).toHaveBeenCalledOnce();
    // 会话改名后重新激活：标题必须同步刷新，不能停留在旧名
    expect(dockviewApiMocks.panelMap.get("terminal-session-a")?.api.setTitle).toHaveBeenCalled();
    wrapper.unmount();
  });

  // 终端标签用短格式：会话名优先，其次短 id。
  // 回归点是 addPanel 漏传 title —— dockview 会用 `terminal-<uuid>` 当标签。
  it("titles the terminal panel with the short session name instead of the panel id", async () => {
    dockviewApiMocks.panelMap.clear();
    dockviewApiMocks.addPanel.mockClear();
    if (!sessionsStoreMock.store) throw new Error("sessions store mock missing");
    sessionsStoreMock.store.items = [{ id: "session-42", name: "prod-web-01" }];

    const wrapper = mount(App, { global: { stubs: childStubs } });
    await wrapper.findComponent({ name: "SidePanel" }).vm.$emit("select-session", "session-42");
    await flushPromises();

    expect(dockviewApiMocks.addPanel).toHaveBeenCalledWith(
      expect.objectContaining({ id: "terminal-session-42", title: "prod-web-01" }),
    );
    const title = dockviewApiMocks.addPanel.mock.calls[0][0] as unknown as { title: string };
    expect(title.title).not.toContain("session-42");
    wrapper.unmount();
  });

  // R2-03：dockview 默认标签自带关闭按钮，面板被关闭后 App 无回调可同步；
  // 重复点击同一会话时 watch(activeTerminal) 因 Object.is 相等不触发，
  // selectSession 必须直接确保面板重建，否则单会话下终端区域永久空白。
  it("rebuilds the terminal panel after the user closes the tab and re-clicks the same session", async () => {
    dockviewApiMocks.panelMap.clear();
    dockviewApiMocks.addPanel.mockClear();
    const wrapper = mount(App, { global: { stubs: childStubs } });
    const sidePanel = wrapper.findComponent({ name: "SidePanel" });
    await sidePanel.vm.$emit("select-session", "session-a");
    await flushPromises();
    expect(dockviewApiMocks.addPanel).toHaveBeenCalledTimes(1);
    expect(dockviewApiMocks.panelMap.get("terminal-session-a")).toBeDefined();

    // 模拟用户点击标签关闭按钮：面板从 dockview 注册表移除，App 侧无回调
    dockviewApiMocks.panelMap.delete("terminal-session-a");

    // 重新点击同一会话：面板必须被重建（回归时 watch 不触发，addPanel 仍为 1 次）
    await sidePanel.vm.$emit("select-session", "session-a");
    await flushPromises();
    expect(dockviewApiMocks.addPanel).toHaveBeenCalledTimes(2);
    expect(dockviewApiMocks.addPanel).toHaveBeenLastCalledWith({
      id: "terminal-session-a",
      component: "terminal",
      title: "session-",
      params: { sessionId: "session-a" },
    });
    expect(dockviewApiMocks.panelMap.get("terminal-session-a")).toBeDefined();
    wrapper.unmount();
  });

  // R2-12：删除已打开终端的会话后，对应面板必须同步关闭——否则残留指向
  // 已删除会话的僵尸面板，键入只会触发 IO 失败提示。
  it("closes the terminal panel of a session after it is deleted", async () => {
    dockviewApiMocks.panelMap.clear();
    dockviewApiMocks.addPanel.mockClear();
    const wrapper = mount(App, { global: { stubs: childStubs } });
    const sidePanel = wrapper.findComponent({ name: "SidePanel" });
    await sidePanel.vm.$emit("select-session", "session-a");
    await flushPromises();
    expect(dockviewApiMocks.panelMap.get("terminal-session-a")).toBeDefined();

    // 会话删除：真实路径 SessionListChanged → store.refresh 更新 items（不再含 session-a）。
    // 回归时 close 不会被调用，面板残留。
    if (!sessionsStoreMock.store) throw new Error("sessions store mock missing");
    sessionsStoreMock.store.items = [{ id: "session-b" }];
    await flushPromises();

    expect(dockviewApiMocks.panelMap.get("terminal-session-a")?.api.close).toHaveBeenCalled();
    wrapper.unmount();
  });

  // R2-13：window 级 Ctrl+F/Escape 由 App.vue 统一拦截，仅路由到当前激活终端
  //（sessionId 过滤在 TerminalPane 内完成）——回归时每个面板各自响应会多播。
  it("routes window Ctrl+F/Escape to the active terminal only", async () => {
    const wrapper = mount(App, { global: { stubs: childStubs } });
    const seen: Array<{ sessionId: string; action: string }> = [];
    const handler = (e: Event) => seen.push((e as CustomEvent<{ sessionId: string; action: string }>).detail);
    window.addEventListener("rshell:terminal-action", handler);

    // 无激活终端：快捷键不分发
    window.dispatchEvent(new KeyboardEvent("keydown", { ctrlKey: true, key: "f" }));
    await flushPromises();
    expect(seen).toEqual([]);

    await wrapper.findComponent({ name: "SidePanel" }).vm.$emit("select-session", "session-a");
    await flushPromises();
    window.dispatchEvent(new KeyboardEvent("keydown", { ctrlKey: true, key: "f" }));
    await flushPromises();
    expect(seen).toEqual([{ sessionId: "session-a", action: "find" }]);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flushPromises();
    expect(seen).toEqual([
      { sessionId: "session-a", action: "find" },
      { sessionId: "session-a", action: "closeFind" },
    ]);

    window.removeEventListener("rshell:terminal-action", handler);
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
