import { afterEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import ElementPlus from "element-plus";
import KeyManagerPanel from "../../src/components/KeyManagerPanel.vue";
import { deleteSshKey, importPrivateKey } from "../../src/ipc/client";

// 选文件走应用内路径选择器（utils/pathPicker），删除确认走 utils/dialog，
// 都不再经过 tauri-plugin-dialog 的原生窗口。
const { pickLocalPath, confirmDialog } = vi.hoisted(() => ({
  pickLocalPath: vi.fn(),
  confirmDialog: vi.fn(),
}));
vi.mock("../../src/utils/pathPicker", () => ({ pickLocalPath }));
vi.mock("../../src/utils/dialog", () => ({ confirmDialog }));
vi.mock("../../src/ipc/client", () => ({
  listKeys: vi.fn().mockResolvedValue([{ id: "key-1", name: "work", key_type: "ED25519", fingerprint: "SHA256:test", has_passphrase: false }]),
  generateSshKey: vi.fn(), importPrivateKey: vi.fn(), deleteSshKey: vi.fn(),
}));

afterEach(() => {
  vi.clearAllMocks();
  document.body.innerHTML = "";
});

type Wrapper = ReturnType<typeof mountPanel>;
const mountPanel = () =>
  mount(KeyManagerPanel, { attachTo: document.body, props: { embedded: true }, global: { plugins: [ElementPlus] } });
const importButton = (wrapper: Wrapper) =>
  wrapper.findAll("button").find(button => button.text() === "导入");

async function openImportDialog(wrapper: Wrapper) {
  pickLocalPath.mockResolvedValue("/tmp/id_ed25519");
  await importButton(wrapper)!.trigger("click");
  await flushPromises();
}

describe("embedded key manager", () => {
  it("exposes the supported private key import action", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    const button = importButton(wrapper);
    expect(button, "import must remain reachable inside the side panel").toBeDefined();
    await openImportDialog(wrapper);
    const input = wrapper.find('.el-dialog input[type="password"]');
    expect(input.exists(), "passphrase dialog must appear after picking a file").toBe(true);
    await input.setValue("test-passphrase");
    await wrapper.find(".el-dialog__footer button.el-button--primary").trigger("click");
    await flushPromises();
    expect(importPrivateKey).toHaveBeenCalledWith("/tmp/id_ed25519", "test-passphrase");
    wrapper.unmount();
  });

  it("keeps an empty passphrase as no-passphrase import and cancel inert", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    await openImportDialog(wrapper);
    await wrapper.findAll(".el-dialog__footer button").find(button => button.text() === "取消")!.trigger("click");
    await flushPromises();
    expect(importPrivateKey).not.toHaveBeenCalled();
    await openImportDialog(wrapper);
    await wrapper.find(".el-dialog__footer button.el-button--primary").trigger("click");
    await flushPromises();
    expect(importPrivateKey).toHaveBeenCalledWith("/tmp/id_ed25519", null);
    wrapper.unmount();
  });

  it("requires confirmation before deleting a key", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    const button = wrapper.findAll("button").find(button => button.text() === "删除")!;
    confirmDialog.mockResolvedValue(false);
    await button.trigger("click");
    await flushPromises();
    expect(deleteSshKey).not.toHaveBeenCalled();
    confirmDialog.mockResolvedValue(true);
    await button.trigger("click");
    await flushPromises();
    expect(deleteSshKey).toHaveBeenCalledWith("key-1");
    wrapper.unmount();
  });
});
