import { describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import App from "../../src/App.vue";
import { subscribeAppEvents } from "../../src/ipc/events";
import { listTransfers } from "../../src/ipc/client";
import TransferPanel from "../../src/components/TransferPanel.vue";
import ElementPlus from "element-plus";

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
vi.mock("../../src/ipc/client", () => ({ listTransfers: vi.fn().mockResolvedValue([]) }));
vi.mock("../../src/ipc/events", () => ({ subscribeAppEvents: vi.fn().mockResolvedValue(vi.fn()) }));

describe("App layout", () => {
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
    DockviewVue: { name: "DockviewVue", template: "<div data-testid='dockview'><slot name='terminal' /></div>" },
    TerminalPane: { name: "TerminalPane", template: "<div data-testid='terminal-pane' />" },
    TransferWorkspace: { name: "TransferWorkspace", template: "<div data-testid='transfer-workspace' />" },
    TransferPanel: { name: "TransferPanel", template: "<div data-testid='transfer-panel' />" },
    SessionCreateDialog: { name: "SessionCreateDialog", template: "<div />" },
    HostKeyMismatchDialog: { name: "HostKeyMismatchDialog", template: "<div />" },
    TransferQueue: { name: "TransferQueue", template: "<div />" },
    MasterPasswordDialog: { name: "MasterPasswordDialog", template: "<div />" },
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
});
