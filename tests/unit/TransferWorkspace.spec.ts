import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import TransferWorkspace from "../../src/components/transfer/TransferWorkspace.vue";
import FileBrowserPane from "../../src/components/transfer/FileBrowserPane.vue";
import { enqueueUpload, enqueueDownload, deleteRemoteEntry, getRemoteHomeDir } from "../../src/ipc/client";
import { confirm } from "@tauri-apps/plugin-dialog";

vi.mock("../../src/ipc/client", () => ({
  enqueueUpload: vi.fn().mockResolvedValue(undefined),
  enqueueDownload: vi.fn().mockResolvedValue(undefined),
  createRemoteDirectory: vi.fn().mockResolvedValue(undefined),
  deleteRemoteEntry: vi.fn().mockResolvedValue(undefined),
  getRemoteHomeDir: vi.fn().mockResolvedValue("/"),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
  confirm: vi.fn().mockResolvedValue(false),
}));
vi.mock("element-plus", () => ({
  ElMessage: { error: vi.fn() },
  ElMessageBox: { prompt: vi.fn() },
}));

const paneStub = defineComponent({
  name: "FileBrowserPane",
  props: ["mode"],
  template: '<div :data-mode="mode" />',
  setup(_props, { expose }) {
    expose({ refresh: vi.fn().mockResolvedValue(undefined) });
  },
});

function workspace(localPath = "/Users/test/files", sessionId = "session-1") {
  return mount(TransferWorkspace, {
    props: { localPath, sessionId, connected: true },
    global: { stubs: { FileBrowserPane: paneStub } },
  });
}

describe("TransferWorkspace", () => {
  beforeEach(() => vi.clearAllMocks());

  it("does not enqueue when a file is merely selected", async () => {
    const wrapper = workspace();
    const panes = wrapper.findAllComponents(FileBrowserPane);
    panes[0].vm.$emit("selection-change", [{ name: "a.txt", is_dir: false, size: 1, modified: "" }]);
    await flushPromises();
    expect(enqueueUpload).not.toHaveBeenCalled();
    expect(wrapper.emitted("capabilities")?.at(-1)?.[0]).toMatchObject({ upload: true, download: false, delete: false });
  });

  it("disables file actions without a session or chosen local root", async () => {
    const wrapper = mount(TransferWorkspace, { global: { stubs: { FileBrowserPane: paneStub } } });
    await flushPromises();
    expect(wrapper.emitted("capabilities")?.at(-1)?.[0]).toMatchObject({
      upload: false, download: false, createFolder: false, delete: false, sync: false,
    });
    await (wrapper.vm as unknown as { upload(): Promise<void>; deleteSelected(): Promise<void> }).upload();
    await (wrapper.vm as unknown as { deleteSelected(): Promise<void> }).deleteSelected();
    expect(enqueueUpload).not.toHaveBeenCalled();
    expect(deleteRemoteEntry).not.toHaveBeenCalled();
  });

  it("queues one selected regular file only on an explicit upload", async () => {
    const wrapper = workspace();
    wrapper.findAllComponents(FileBrowserPane)[0].vm.$emit("selection-change", [{ name: "a.txt", is_dir: false, size: 1, modified: "" }]);
    await flushPromises();
    await (wrapper.vm as unknown as { upload(): Promise<void> }).upload();
    expect(enqueueUpload).toHaveBeenCalledOnce();
    expect(enqueueUpload).toHaveBeenCalledWith("/Users/test/files/a.txt", "/a.txt", "session-1", "Fail");
  });

  it("does not delete before confirmation or when a directory is selected", async () => {
    const wrapper = workspace();
    const remote = wrapper.findAllComponents(FileBrowserPane)[1];
    remote.vm.$emit("selection-change", [{ name: "folder", is_dir: true, size: 0, modified: "" }]);
    await flushPromises();
    await (wrapper.vm as unknown as { deleteSelected(): Promise<void> }).deleteSelected();
    expect(confirm).not.toHaveBeenCalled();
    remote.vm.$emit("selection-change", [{ name: "a.txt", is_dir: false, size: 1, modified: "" }]);
    await flushPromises();
    await (wrapper.vm as unknown as { deleteSelected(): Promise<void> }).deleteSelected();
    expect(confirm).toHaveBeenCalledOnce();
    expect(deleteRemoteEntry).not.toHaveBeenCalled();
  });
});

describe("TransferWorkspace 批量传输", () => {
  beforeEach(() => vi.clearAllMocks());

  it("queues every selected regular file, not just the first", async () => {
    const wrapper = workspace();
    wrapper.findAllComponents(FileBrowserPane)[0].vm.$emit("selection-change", [
      { name: "a.txt", is_dir: false, size: 1, modified: "" },
      { name: "b.txt", is_dir: false, size: 1, modified: "" },
    ]);
    await flushPromises();
    await (wrapper.vm as unknown as { upload(): Promise<void> }).upload();
    expect(enqueueUpload).toHaveBeenCalledTimes(2);
    expect(enqueueUpload).toHaveBeenCalledWith("/Users/test/files/a.txt", "/a.txt", "session-1", "Fail");
    expect(enqueueUpload).toHaveBeenCalledWith("/Users/test/files/b.txt", "/b.txt", "session-1", "Fail");
  });

  // 目录没有可传输的内容，且不安全名称（. / .. / 含分隔符）必须被剔除，
  // 否则会拼出越界或逃出目标目录的路径
  it("skips directories and unsafe names inside a multi-selection", async () => {
    const wrapper = workspace();
    wrapper.findAllComponents(FileBrowserPane)[0].vm.$emit("selection-change", [
      { name: "ok.txt", is_dir: false, size: 1, modified: "" },
      { name: "folder", is_dir: true, size: 0, modified: "" },
      { name: "..", is_dir: false, size: 0, modified: "" },
      { name: "a/b.txt", is_dir: false, size: 0, modified: "" },
    ]);
    await flushPromises();
    await (wrapper.vm as unknown as { upload(): Promise<void> }).upload();
    expect(enqueueUpload).toHaveBeenCalledOnce();
    expect(enqueueUpload).toHaveBeenCalledWith("/Users/test/files/ok.txt", "/ok.txt", "session-1", "Fail");
  });

  it("keeps queueing the rest when one file fails", async () => {
    vi.mocked(enqueueUpload)
      .mockRejectedValueOnce(new Error("Permission denied"))
      .mockResolvedValueOnce(undefined);
    const wrapper = workspace();
    wrapper.findAllComponents(FileBrowserPane)[0].vm.$emit("selection-change", [
      { name: "a.txt", is_dir: false, size: 1, modified: "" },
      { name: "b.txt", is_dir: false, size: 1, modified: "" },
    ]);
    await flushPromises();
    await (wrapper.vm as unknown as { upload(): Promise<void> }).upload();
    expect(enqueueUpload).toHaveBeenCalledTimes(2);
    const queued = wrapper.emitted("upload-queued")?.at(-1)?.[0];
    expect(queued).toBe(1);
  });

  it("queues multi-selected remote files for download", async () => {
    const wrapper = workspace();
    wrapper.findAllComponents(FileBrowserPane)[1].vm.$emit("selection-change", [
      { name: "a.txt", is_dir: false, size: 1, modified: "" },
      { name: "b.log", is_dir: false, size: 1, modified: "" },
    ]);
    await flushPromises();
    await (wrapper.vm as unknown as { download(): Promise<void> }).download();
    expect(enqueueDownload).toHaveBeenCalledTimes(2);
    expect(enqueueDownload).toHaveBeenCalledWith("/a.txt", "/Users/test/files/a.txt", "session-1", "Fail");
  });
});

describe("TransferWorkspace 右键菜单", () => {
  beforeEach(() => vi.clearAllMocks());

  it("offers upload on a selected local file and performs it on click", async () => {
    const wrapper = workspace();
    const local = wrapper.findAllComponents(FileBrowserPane)[0];
    local.vm.$emit("selection-change", [{ name: "a.txt", is_dir: false, size: 1, modified: "" }]);
    await flushPromises();
    local.vm.$emit("row-context-menu", {
      entries: [{ name: "a.txt", is_dir: false, size: 1, modified: "" }],
      x: 100,
      y: 120,
    });
    await flushPromises();

    const menu = wrapper.find('[data-test="context-menu"]');
    expect(menu.exists()).toBe(true);
    const uploadItem = wrapper.find('[data-test="context-upload"]');
    expect(uploadItem.exists()).toBe(true);
    await uploadItem.trigger("click");
    await flushPromises();
    expect(enqueueUpload).toHaveBeenCalledOnce();
    // 菜单在动作后必须关闭
    expect(wrapper.find('[data-test="context-menu"]').exists()).toBe(false);
  });

  it("offers download plus delete for a selected remote file", async () => {
    const wrapper = workspace();
    const remote = wrapper.findAllComponents(FileBrowserPane)[1];
    const entry = { name: "a.txt", is_dir: false, size: 1, modified: "" };
    remote.vm.$emit("selection-change", [entry]);
    await flushPromises();
    remote.vm.$emit("row-context-menu", { entries: [entry], x: 10, y: 20 });
    await flushPromises();

    expect(wrapper.find('[data-test="context-download"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="context-createFolder"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="context-delete"]').exists()).toBe(true);
  });

  // 批量删除的确认语义与单条不同，因此只对单个普通文件开放删除
  it("hides delete for a multi-file remote selection", async () => {
    const wrapper = workspace();
    const remote = wrapper.findAllComponents(FileBrowserPane)[1];
    remote.vm.$emit("selection-change", [
      { name: "a.txt", is_dir: false, size: 1, modified: "" },
      { name: "b.txt", is_dir: false, size: 1, modified: "" },
    ]);
    await flushPromises();
    remote.vm.$emit("row-context-menu", {
      entries: [
        { name: "a.txt", is_dir: false, size: 1, modified: "" },
        { name: "b.txt", is_dir: false, size: 1, modified: "" },
      ],
      x: 10,
      y: 20,
    });
    await flushPromises();
    expect(wrapper.find('[data-test="context-download"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="context-delete"]').exists()).toBe(false);
  });

  it("falls back to pane-level actions when the menu is opened on empty space", async () => {
    const wrapper = workspace();
    const local = wrapper.findAllComponents(FileBrowserPane)[0];
    local.vm.$emit("row-context-menu", { entries: [], x: 5, y: 6 });
    await flushPromises();
    expect(wrapper.find('[data-test="context-upload"]').exists()).toBe(false);
    expect(wrapper.find('[data-test="context-refresh"]').exists()).toBe(true);
  });

  it("closes when the backdrop is clicked", async () => {
    const wrapper = workspace();
    wrapper.findAllComponents(FileBrowserPane)[0].vm.$emit("row-context-menu", {
      entries: [], x: 5, y: 6,
    });
    await flushPromises();
    expect(wrapper.find('[data-test="context-menu"]').exists()).toBe(true);
    await wrapper.find('[data-test="context-backdrop"]').trigger("click");
    expect(wrapper.find('[data-test="context-menu"]').exists()).toBe(false);
  });
});

describe("TransferWorkspace 远端默认工作目录", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
  });

  it("navigates to the remote home dir instead of the filesystem root", async () => {
    vi.mocked(getRemoteHomeDir).mockResolvedValue("/home/deploy");
    const wrapper = mount(TransferWorkspace, {
      props: { localPath: "/Users/test/files", sessionId: "session-1", connected: true },
      global: { stubs: { FileBrowserPane: paneStub } },
    });
    await flushPromises();
    expect(getRemoteHomeDir).toHaveBeenCalledWith("session-1");
    expect(wrapper.emitted("remote-path")?.at(-1)?.[0]).toBe("/home/deploy");
  });

  it("respects an explicitly supplied remotePath", async () => {
    vi.mocked(getRemoteHomeDir).mockResolvedValue("/home/deploy");
    mount(TransferWorkspace, {
      props: {
        localPath: "/Users/test/files",
        sessionId: "session-1",
        connected: true,
        remotePath: "/srv/app",
      },
      global: { stubs: { FileBrowserPane: paneStub } },
    });
    await flushPromises();
    expect(getRemoteHomeDir).not.toHaveBeenCalled();
  });

  it("does not query before the session is connected", async () => {
    mount(TransferWorkspace, {
      props: { localPath: "/Users/test/files", sessionId: "session-1", connected: false },
      global: { stubs: { FileBrowserPane: paneStub } },
    });
    await flushPromises();
    expect(getRemoteHomeDir).not.toHaveBeenCalled();
  });

  // 解析失败时保持当前目录，不能把远端面板打成空白
  it("keeps the current path when the home dir cannot be resolved", async () => {
    vi.mocked(getRemoteHomeDir).mockRejectedValue(new Error("SFTP init failed"));
    const wrapper = mount(TransferWorkspace, {
      props: { localPath: "/Users/test/files", sessionId: "session-1", connected: true },
      global: { stubs: { FileBrowserPane: paneStub } },
    });
    await flushPromises();
    expect(wrapper.emitted("remote-path")).toBeUndefined();
  });
});

describe("TransferWorkspace 本地目录记忆", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
  });

  it("restores the previously chosen local directory on mount", async () => {
    localStorage.setItem("rshell.transfer.lastLocalDir.v1", "/Users/test/previous");
    const wrapper = mount(TransferWorkspace, {
      props: { sessionId: "session-1", connected: true },
      global: { stubs: { FileBrowserPane: paneStub } },
    });
    await flushPromises();
    expect(wrapper.emitted("local-path")?.at(-1)?.[0]).toBe("/Users/test/previous");
  });

  it("ignores a stored relative path (must not be trusted for restore)", async () => {
    localStorage.setItem("rshell.transfer.lastLocalDir.v1", "relative/path");
    const wrapper = mount(TransferWorkspace, {
      props: { sessionId: "session-1", connected: true },
      global: { stubs: { FileBrowserPane: paneStub } },
    });
    await flushPromises();
    expect(wrapper.emitted("local-path")).toBeUndefined();
  });

  it("an explicit localPath prop wins over the remembered value", async () => {
    localStorage.setItem("rshell.transfer.lastLocalDir.v1", "/Users/test/previous");
    const wrapper = workspace("/Users/test/explicit");
    await flushPromises();
    // 显式传入 localPath 时不应被记忆值覆盖（该场景下也不会 emit local-path）
    const paths = (wrapper.emitted("local-path") ?? []).map((e) => e[0]);
    expect(paths).not.toContain("/Users/test/previous");
  });
});
