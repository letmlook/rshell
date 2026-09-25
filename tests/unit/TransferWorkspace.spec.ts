import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import TransferWorkspace from "../../src/components/transfer/TransferWorkspace.vue";
import FileBrowserPane from "../../src/components/transfer/FileBrowserPane.vue";
import { enqueueUpload, deleteRemoteEntry } from "../../src/ipc/client";
import { confirm } from "@tauri-apps/plugin-dialog";

vi.mock("../../src/ipc/client", () => ({
  enqueueUpload: vi.fn().mockResolvedValue(undefined),
  enqueueDownload: vi.fn().mockResolvedValue(undefined),
  createRemoteDirectory: vi.fn().mockResolvedValue(undefined),
  deleteRemoteEntry: vi.fn().mockResolvedValue(undefined),
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
    expect(enqueueUpload).toHaveBeenCalledWith("/Users/test/files/a.txt", "/a.txt", "session-1");
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
