import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import HostKeyPanel from "../../src/components/HostKeyPanel.vue";
import { deleteHostKey, listHostKeys } from "../../src/ipc/client";
import type { HostKeyEntry } from "../../src/ipc/types";

const { confirmDialog } = vi.hoisted(() => ({ confirmDialog: vi.fn() }));

vi.mock("../../src/ipc/client", () => ({
  listHostKeys: vi.fn(),
  deleteHostKey: vi.fn(),
  ipcErrorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));
vi.mock("../../src/utils/dialog", () => ({ confirmDialog }));

const stubs = {
  "el-button": { template: "<button><slot /></button>" },
  // el-empty 的空状态文案要渲染出来，否则「没有已信任的主机密钥」无法断言
  "el-empty": { props: ["description"], template: '<div>{{ description }}</div>' },
};

const entry: HostKeyEntry = {
  host: "10.0.0.1",
  port: 22,
  key_type: "ssh-ed25519",
  fingerprint: "SHA256:abc",
  trust_level: "Trusted",
  first_seen: "2026-10-01T00:00:00Z",
  last_seen: "2026-10-02T00:00:00Z",
};

describe("HostKeyPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(listHostKeys).mockResolvedValue([]);
    vi.mocked(deleteHostKey).mockResolvedValue(undefined);
    confirmDialog.mockResolvedValue(true);
  });

  it("lists the trusted host keys the application knows about", async () => {
    vi.mocked(listHostKeys).mockResolvedValue([entry]);
    const wrapper = mount(HostKeyPanel, { global: { stubs } });
    await flushPromises();

    expect(wrapper.get('[data-test="host-key-list"]').text()).toContain("10.0.0.1:22");
    expect(wrapper.get('[data-test="host-key-list"]').text()).toContain("SHA256:abc");
    wrapper.unmount();
  });

  it("shows both the first and the last seen time", async () => {
    // 后端 sidecar 一直记着首次连接时间，界面上此前只显示「上次连接」——
    // 用户判断一条主机密钥是不是自己很久以前加的，只能靠猜。
    vi.mocked(listHostKeys).mockResolvedValue([entry]);
    const wrapper = mount(HostKeyPanel, { global: { stubs } });
    await flushPromises();

    const text = wrapper.get('[data-test="host-key-list"]').text();
    expect(text).toContain("首次连接");
    expect(text).toContain("上次连接");
    wrapper.unmount();
  });

  it("says so when nothing is trusted yet", async () => {
    const wrapper = mount(HostKeyPanel, { global: { stubs } });
    await flushPromises();
    expect(wrapper.text()).toContain("没有已信任的主机密钥");
    wrapper.unmount();
  });

  it("deletes an entry after confirmation", async () => {
    vi.mocked(listHostKeys).mockResolvedValue([entry]);
    const wrapper = mount(HostKeyPanel, { global: { stubs } });
    await flushPromises();

    await wrapper.get('[data-test="host-key-delete-10.0.0.1-22"]').trigger("click");
    await flushPromises();

    expect(confirmDialog).toHaveBeenCalled();
    expect(deleteHostKey).toHaveBeenCalledWith("10.0.0.1", 22);
    wrapper.unmount();
  });

  it("does not delete when the user cancels the confirmation", async () => {
    vi.mocked(listHostKeys).mockResolvedValue([entry]);
    confirmDialog.mockResolvedValue(false);
    const wrapper = mount(HostKeyPanel, { global: { stubs } });
    await flushPromises();

    await wrapper.get('[data-test="host-key-delete-10.0.0.1-22"]').trigger("click");
    await flushPromises();

    expect(deleteHostKey).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("surfaces a deletion failure", async () => {
    vi.mocked(listHostKeys).mockResolvedValue([entry]);
    vi.mocked(deleteHostKey).mockRejectedValue(new Error("known_hosts is read-only"));
    const wrapper = mount(HostKeyPanel, { global: { stubs } });
    await flushPromises();

    await wrapper.get('[data-test="host-key-delete-10.0.0.1-22"]').trigger("click");
    await flushPromises();

    expect(wrapper.get('[data-test="host-key-error"]').text()).toContain("known_hosts is read-only");
    wrapper.unmount();
  });
});
