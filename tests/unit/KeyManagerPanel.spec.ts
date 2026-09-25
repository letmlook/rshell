import { afterEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import ElementPlus from "element-plus";
import KeyManagerPanel from "../../src/components/KeyManagerPanel.vue";
import { deleteSshKey, importPrivateKey } from "../../src/ipc/client";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn().mockResolvedValue("/tmp/id_ed25519") }));
vi.mock("../../src/ipc/client", () => ({
  listKeys: vi.fn().mockResolvedValue([{ id: "key-1", name: "work", key_type: "ED25519", fingerprint: "SHA256:test", has_passphrase: false }]),
  generateSshKey: vi.fn(), importPrivateKey: vi.fn(), deleteSshKey: vi.fn(),
}));

afterEach(() => { vi.restoreAllMocks(); vi.clearAllMocks(); });

describe("embedded key manager", () => {
  it("exposes the supported private key import action", async () => {
    vi.spyOn(window, "prompt").mockReturnValue("");
    const wrapper = mount(KeyManagerPanel, { props: { embedded: true }, global: { plugins: [ElementPlus] } });
    await flushPromises();
    const button = wrapper.findAll("button").find(button => button.text() === "导入");
    expect(button, "import must remain reachable inside the side panel").toBeDefined();
    await button!.trigger("click");
    await flushPromises();
    expect(importPrivateKey).toHaveBeenCalledWith("/tmp/id_ed25519", "");
    wrapper.unmount();
  });

  it("requires confirmation before deleting a key", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const wrapper = mount(KeyManagerPanel, { props: { embedded: true }, global: { plugins: [ElementPlus] } });
    await flushPromises();
    const button = wrapper.findAll("button").find(button => button.text() === "删除")!;
    await button.trigger("click");
    await flushPromises();
    expect(deleteSshKey).not.toHaveBeenCalled();
    confirm.mockReturnValue(true);
    await button.trigger("click");
    await flushPromises();
    expect(deleteSshKey).toHaveBeenCalledWith("key-1");
    wrapper.unmount();
  });
});
