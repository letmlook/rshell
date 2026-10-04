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
  sessionsStoreMock: { store: null as { items: Array<{ id: string; name?: string }>; connectionState: Map<string, string>; duplicate: ReturnType<typeof vi.fn>; connect: ReturnType<typeof vi.fn>; [key: string]: unknown } | null },
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
      // 焦点/关闭事件：App.vue 用它们同步当前会话与多开窗口的标题编号
      onDidActivePanelChange: vi.fn(),
      onDidRemovePanel: vi.fn(),
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
    duplicate: vi.fn(async () => "new-session-id"),
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
    props: ["components", "getTabContextMenuItems"],
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
      params: { sessionId: "session-42", terminalId: "session-42" },
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
      expect.objectContaining({ id: "terminal-session-b", params: { sessionId: "session-b", terminalId: "session-b" } }),
    );
    expect(dockviewApiMocks.panelMap.get("terminal-session-a")?.api.setActive).toHaveBeenCalledOnce();
    // 会话改名后重新激活：标题必须同步刷新，不能停留在旧名
    expect(dockviewApiMocks.panelMap.get("terminal-session-a")?.api.setTitle).toHaveBeenCalled();
    wrapper.unmount();
  });

  // 终端标签用短格式：会话名优先，其次短 id。
  // 回归点是 addPanel 漏传 title —— dockview 会用 `terminal-<uuid>` 当标签。
  // 同一份连接信息可以同时开多个终端窗口：第二个窗口必须是**新面板**
  // （id 带 ~2 序号、标题带 #2），而不是把已有窗口激活掉。
  it("opens an extra terminal window for the same session instead of reusing the panel", async () => {
    dockviewApiMocks.panelMap.clear();
    dockviewApiMocks.addPanel.mockClear();
    if (!sessionsStoreMock.store) throw new Error("sessions store mock missing");
    sessionsStoreMock.store.items = [{ id: "session-42", name: "prod-web-01" }];

    const wrapper = mount(App, { global: { stubs: childStubs } });
    const sidePanel = wrapper.findComponent({ name: "SidePanel" });
    await sidePanel.vm.$emit("select-session", "session-42");
    await flushPromises();
    await sidePanel.vm.$emit("open-terminal-window", "session-42");
    await flushPromises();

    expect(dockviewApiMocks.addPanel).toHaveBeenCalledTimes(2);
    expect(dockviewApiMocks.addPanel).toHaveBeenLastCalledWith(
      expect.objectContaining({ id: "terminal-session-42~2", params: expect.objectContaining({ sessionId: "session-42" }) }),
    );
    // 附加窗口的编号来自「已打开窗口的顺序」，不写死成 2
    const second = dockviewApiMocks.addPanel.mock.calls[1][0] as unknown as { title: string };
    expect(second.title).toBe("prod-web-01 #2");
    // 主窗口仍在，不被新窗口顶掉
    expect(dockviewApiMocks.panelMap.has("terminal-session-42")).toBe(true);
    wrapper.unmount();
  });

  // 关闭中间窗口后编号补齐，不能留下 #2 #4 这种跳号标签
  it("renumbers the remaining windows after one of them closes", async () => {
    dockviewApiMocks.panelMap.clear();
    dockviewApiMocks.addPanel.mockClear();
    if (!sessionsStoreMock.store) throw new Error("sessions store mock missing");
    sessionsStoreMock.store.items = [{ id: "session-42", name: "prod-web-01" }];

    const wrapper = mount(App, { global: { stubs: childStubs } });
    const sidePanel = wrapper.findComponent({ name: "SidePanel" });
    await sidePanel.vm.$emit("select-session", "session-42");
    await flushPromises();
    await sidePanel.vm.$emit("open-terminal-window", "session-42");
    await sidePanel.vm.$emit("open-terminal-window", "session-42");
    await flushPromises();
    expect(dockviewApiMocks.addPanel).toHaveBeenCalledTimes(3);
    expect(dockviewApiMocks.addPanel).toHaveBeenLastCalledWith(
      expect.objectContaining({ id: "terminal-session-42~3" }),
    );

    // 用户关掉 #2（dockview 会回调 onDidRemovePanel）
    dockviewApiMocks.panelMap.delete("terminal-session-42~2");
    const removeHandler = dockviewApiMocks.onDidRemovePanel.mock.calls.at(-1)?.[0] as
      | ((event: { id: string }) => void)
      | undefined;
    expect(typeof removeHandler).toBe("function");
    removeHandler?.({ id: "terminal-session-42~2" });

    const remaining = dockviewApiMocks.panelMap.get("terminal-session-42~3");
    expect(remaining?.api.setTitle).toHaveBeenLastCalledWith("prod-web-01 #2");
    wrapper.unmount();
  });

  // 终端标签右键：三项都以连接信息为操作对象，但结果不同——
  // 新建标签用当前聚焦标签的连接，复制标签用右键所在标签的连接，关闭只关这一个。
  // 菜单由 dockview 的 getTabContextMenuItems 提供（省略该选项 dockview 不弹菜单）。
  it("offers 新建标签 / 复制标签 / 关闭标签 on the terminal tab context menu", async () => {
    dockviewApiMocks.panelMap.clear();
    dockviewApiMocks.addPanel.mockClear();
    if (!sessionsStoreMock.store) throw new Error("sessions store mock missing");
    sessionsStoreMock.store.items = [{ id: "session-42", name: "dev-ubuntu" }];
    sessionsStoreMock.store.duplicate.mockClear();
    sessionsStoreMock.store.connectionState.set("session-42", "connected");

    const wrapper = mount(App, { global: { stubs: childStubs } });
    const sidePanel = wrapper.findComponent({ name: "SidePanel" });
    await sidePanel.vm.$emit("select-session", "session-42");
    await flushPromises();

    const dockview = wrapper.findComponent({ name: "DockviewVue" });
    const getItems = dockview.props("getTabContextMenuItems") as
      | ((params: { panel: { id: string; api: { close: ReturnType<typeof vi.fn> } } }) => unknown[])
      | undefined;
    expect(typeof getItems).toBe("function");

    const panel = dockviewApiMocks.panelMap.get("terminal-session-42")!;
    const items = getItems!({ panel }) as Array<{ label?: string; action?: () => void } | string>;
    const labels = items.filter((i) => typeof i === "object").map((i) => (i as { label: string }).label);
    expect(labels).toEqual(["新建标签", "复制标签", "关闭标签"]);

    // 「新建标签」：为当前聚焦的连接再建一个面板，而不是激活原面板
    const newTab = items[0] as { action: () => void };
    newTab.action();
    await flushPromises();
    expect(dockviewApiMocks.addPanel).toHaveBeenLastCalledWith(
      expect.objectContaining({ id: "terminal-session-42~2" }),
    );

    // 「复制标签」：同样只是多开一个独立标签会话，不复制连接信息本身
    const copyTab = items[1] as { action: () => void };
    copyTab.action();
    await flushPromises();
    expect(dockviewApiMocks.addPanel).toHaveBeenLastCalledWith(
      expect.objectContaining({ id: "terminal-session-42~3" }),
    );
    // 标签操作不碰连接信息条目（复制连接信息是左侧列表里的独立动作）
    expect(sessionsStoreMock.store.duplicate).not.toHaveBeenCalled();

    // 「关闭标签」：仍能关掉面板（没有因为换掉内置项而丢失关闭能力）
    const close = items[3] as { action: () => void };
    close.action();
    expect(panel.api.close).toHaveBeenCalled();

    wrapper.unmount();
  });

  it("suppresses the tab context menu for panels that are not terminal sessions", async () => {
    dockviewApiMocks.panelMap.clear();
    if (!sessionsStoreMock.store) throw new Error("sessions store mock missing");
    sessionsStoreMock.store.items = [{ id: "session-42", name: "dev-ubuntu" }];
    const wrapper = mount(App, { global: { stubs: childStubs } });
    await wrapper.findComponent({ name: "SidePanel" }).vm.$emit("select-session", "session-42");
    await flushPromises();

    const getItems = wrapper.findComponent({ name: "DockviewVue" }).props("getTabContextMenuItems") as
      | ((params: { panel: { id: string } }) => unknown[])
      | undefined;
    expect(getItems!({ panel: { id: "some-other-panel" } })).toEqual([]);
    wrapper.unmount();
  });

  it("duplicates the session from the sidebar without auto-connecting it", async () => {
    dockviewApiMocks.panelMap.clear();
    dockviewApiMocks.addPanel.mockClear();
    if (!sessionsStoreMock.store) throw new Error("sessions store mock missing");
    sessionsStoreMock.store.items = [{ id: "session-42", name: "dev-ubuntu" }];
    sessionsStoreMock.store.duplicate.mockClear();
    sessionsStoreMock.store.connect.mockClear();

    const wrapper = mount(App, { global: { stubs: childStubs } });
    await wrapper.findComponent({ name: "SidePanel" }).vm.$emit("duplicate-session", "session-42");
    await flushPromises();

    expect(sessionsStoreMock.store.duplicate).toHaveBeenCalledWith("session-42");
    // 复制不自动连接：新条目没有凭据，自动连只会换来一次注定失败的认证
    expect(sessionsStoreMock.store.connect).not.toHaveBeenCalled();
    wrapper.unmount();
  });

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
      params: { sessionId: "session-a", terminalId: "session-a" },
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

  // 复制粘贴走 Ctrl+Shift+C / Ctrl+Shift+V（Xterm.js 惯例）。裸 Ctrl+C 是 SIGINT、
  // Ctrl+V 是 quoted-insert，劫持它们会破坏终端语义，因此必须原样透传给远端。
  it("routes Ctrl+Shift+C/V to the active terminal and leaves bare Ctrl+C/V alone", async () => {
    const wrapper = mount(App, { global: { stubs: childStubs } });
    const seen: Array<{ sessionId: string; action: string }> = [];
    const handler = (e: Event) => seen.push((e as CustomEvent<{ sessionId: string; action: string }>).detail);
    window.addEventListener("rshell:terminal-action", handler);

    await wrapper.findComponent({ name: "SidePanel" }).vm.$emit("select-session", "session-a");
    await flushPromises();

    window.dispatchEvent(new KeyboardEvent("keydown", { ctrlKey: true, shiftKey: true, key: "C" }));
    window.dispatchEvent(new KeyboardEvent("keydown", { ctrlKey: true, shiftKey: true, key: "v" }));
    await flushPromises();
    expect(seen).toEqual([
      { sessionId: "session-a", action: "copy" },
      { sessionId: "session-a", action: "paste" },
    ]);

    // 裸 Ctrl+C / Ctrl+V 不得被当作剪贴板操作
    seen.length = 0;
    window.dispatchEvent(new KeyboardEvent("keydown", { ctrlKey: true, key: "c" }));
    window.dispatchEvent(new KeyboardEvent("keydown", { ctrlKey: true, key: "v" }));
    await flushPromises();
    expect(seen).toEqual([]);

    window.removeEventListener("rshell:terminal-action", handler);
    wrapper.unmount();
  });

  it("also accepts the macOS meta key for copy and paste", async () => {
    const wrapper = mount(App, { global: { stubs: childStubs } });
    const seen: Array<{ sessionId: string; action: string }> = [];
    const handler = (e: Event) => seen.push((e as CustomEvent<{ sessionId: string; action: string }>).detail);
    window.addEventListener("rshell:terminal-action", handler);

    await wrapper.findComponent({ name: "SidePanel" }).vm.$emit("select-session", "session-a");
    await flushPromises();

    window.dispatchEvent(new KeyboardEvent("keydown", { metaKey: true, shiftKey: true, key: "c" }));
    window.dispatchEvent(new KeyboardEvent("keydown", { metaKey: true, shiftKey: true, key: "V" }));
    await flushPromises();
    expect(seen.map((e) => e.action)).toEqual(["copy", "paste"]);

    window.removeEventListener("rshell:terminal-action", handler);
    wrapper.unmount();
  });

  it("does not dispatch copy/paste when no terminal is active", async () => {
    const wrapper = mount(App, { global: { stubs: childStubs } });
    const seen: string[] = [];
    const handler = (e: Event) => seen.push((e as CustomEvent<{ action: string }>).detail.action);
    window.addEventListener("rshell:terminal-action", handler);

    window.dispatchEvent(new KeyboardEvent("keydown", { ctrlKey: true, shiftKey: true, key: "c" }));
    window.dispatchEvent(new KeyboardEvent("keydown", { ctrlKey: true, shiftKey: true, key: "v" }));
    await flushPromises();
    expect(seen).toEqual([]);

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
