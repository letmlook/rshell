import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import TransferWorkspace from "../../src/components/transfer/TransferWorkspace.vue";
import FileBrowserPane from "../../src/components/transfer/FileBrowserPane.vue";
import { browseRemoteDir, enqueueUpload, enqueueDownload } from "../../src/ipc/client";
import { readDir } from "@tauri-apps/plugin-fs";
import { active, closeDialog, resetDialogs } from "../../src/utils/dialog";

vi.mock("../../src/ipc/client", () => ({
  enqueueUpload: vi.fn().mockResolvedValue(undefined),
  enqueueDownload: vi.fn().mockResolvedValue(undefined),
  createRemoteDirectory: vi.fn().mockResolvedValue(undefined),
  deleteRemoteEntry: vi.fn().mockResolvedValue(undefined),
  getRemoteHomeDir: vi.fn().mockResolvedValue("/remote"),
  browseRemoteDir: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-fs", () => ({ readDir: vi.fn() }));
vi.mock("../../src/utils/pathPicker", () => ({ pickLocalPath: vi.fn().mockResolvedValue(null) }));

const paneStub = defineComponent({
  name: "FileBrowserPane",
  props: ["mode"],
  template: '<div :data-mode="mode" />',
});

const file = (name: string) => ({ name, is_dir: false, size: 1, modified: "" });
const remoteFile = (name: string) => ({ name, size: 1, file_type: "File", modified: "0", permissions: null, owner: null });

function workspace() {
  return mount(TransferWorkspace, {
    props: { sessionId: "session-1", connected: true, localPath: "/local", remotePath: "/remote" },
    global: { stubs: { FileBrowserPane: paneStub } },
  });
}

type Vm = { upload(): Promise<void>; download(): Promise<void> };

/** 当前冲突窗上的按钮文案，用来点「覆盖」/「跳过」而不是硬编码内部 value */
function clickConflictButton(label: string) {
  const button = active.value?.buttons.find((b) => b.label === label);
  if (!button) throw new Error(`冲突窗没有「${label}」按钮`);
  closeDialog(button.value);
}

/**
 * 右键上传/下载时，若目标目录已有同名文件，要**当场**弹覆盖/重命名/跳过窗体，
 * 而不是先让后端拒绝一次再来问。
 */
describe("TransferWorkspace 传输冲突预检", () => {
  beforeEach(() => {
    resetDialogs();
    vi.clearAllMocks();
    vi.mocked(enqueueUpload).mockResolvedValue(undefined);
    vi.mocked(enqueueDownload).mockResolvedValue(undefined);
  });

  it("远端已有同名文件：入队前先弹窗，问之前不发生任何传输", async () => {
    vi.mocked(browseRemoteDir).mockResolvedValue({ entries: [remoteFile("a.txt")] } as never);
    const wrapper = workspace();
    wrapper.findAllComponents(FileBrowserPane)[0].vm.$emit("selection-change", [file("a.txt")]);
    await flushPromises();

    const uploading = (wrapper.vm as unknown as Vm).upload();
    await flushPromises();

    // 冲突窗已开，尚未入队
    expect(active.value?.title).toBe("目标已存在同名文件");
    expect(active.value?.buttons.map((b) => b.label)).toEqual(["跳过", "重命名…", "覆盖"]);
    expect(enqueueUpload).not.toHaveBeenCalled();

    clickConflictButton("覆盖");
    await uploading;

    expect(enqueueUpload).toHaveBeenCalledWith("/local/a.txt", "/remote/a.txt", "session-1", "Overwrite");
    wrapper.unmount();
  });

  it("远端无同名文件：直接入队，不打扰用户", async () => {
    vi.mocked(browseRemoteDir).mockResolvedValue({ entries: [remoteFile("other.txt")] } as never);
    const wrapper = workspace();
    wrapper.findAllComponents(FileBrowserPane)[0].vm.$emit("selection-change", [file("a.txt")]);
    await flushPromises();

    await (wrapper.vm as unknown as Vm).upload();

    expect(active.value).toBeNull();
    expect(enqueueUpload).toHaveBeenCalledWith("/local/a.txt", "/remote/a.txt", "session-1", "Fail");
    wrapper.unmount();
  });

  it("本地下载目标已有同名文件：同样先问，跳过则这一条不传", async () => {
    vi.mocked(readDir).mockResolvedValue([{ name: "a.txt", isDirectory: false }] as never);
    const wrapper = workspace();
    wrapper.findAllComponents(FileBrowserPane)[1].vm.$emit("selection-change", [file("a.txt")]);
    await flushPromises();

    const downloading = (wrapper.vm as unknown as Vm).download();
    await flushPromises();
    expect(active.value?.title).toBe("目标已存在同名文件");
    expect(enqueueDownload).not.toHaveBeenCalled();

    clickConflictButton("跳过");
    await downloading;

    expect(enqueueDownload).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("重命名：第二个窗体输入新文件名后按 Rename 入队到新目标", async () => {
    vi.mocked(browseRemoteDir).mockResolvedValue({ entries: [remoteFile("a.txt")] } as never);
    const wrapper = workspace();
    wrapper.findAllComponents(FileBrowserPane)[0].vm.$emit("selection-change", [file("a.txt")]);
    await flushPromises();

    const uploading = (wrapper.vm as unknown as Vm).upload();
    await flushPromises();

    clickConflictButton("重命名…");
    await flushPromises();
    // 第二个窗体是输入窗，且预填原名
    expect(active.value?.title).toBe("重命名后传输");
    expect(active.value?.input?.value).toBe("a.txt");

    const { setActiveDialogInput, submitActiveDialog } = await import("../../src/utils/dialog");
    setActiveDialogInput("a-1.txt");
    submitActiveDialog();
    await uploading;

    expect(enqueueUpload).toHaveBeenCalledWith("/local/a.txt", "/remote/a-1.txt", "session-1", { Rename: "a-1.txt" });
    wrapper.unmount();
  });

  it("目录读取失败时不假装无冲突：交给入队时的 target_exists 兜底", async () => {
    vi.mocked(browseRemoteDir).mockRejectedValue(new Error("SFTP unavailable"));
    const wrapper = workspace();
    wrapper.findAllComponents(FileBrowserPane)[0].vm.$emit("selection-change", [file("a.txt")]);
    await flushPromises();

    await (wrapper.vm as unknown as Vm).upload();

    // 读不到就不知道有没有冲突：不弹窗，但仍然尝试入队（后端会据实拒绝）
    expect(active.value).toBeNull();
    expect(enqueueUpload).toHaveBeenCalledWith("/local/a.txt", "/remote/a.txt", "session-1", "Fail");
    wrapper.unmount();
  });
});
