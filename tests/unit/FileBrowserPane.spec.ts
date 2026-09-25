import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import FileBrowserPane from "../../src/components/transfer/FileBrowserPane.vue";
import { browseRemoteDir } from "../../src/ipc/client";
import { readDir } from "@tauri-apps/plugin-fs";

vi.mock("../../src/ipc/client", () => ({ browseRemoteDir: vi.fn() }));
vi.mock("@tauri-apps/plugin-fs", () => ({ readDir: vi.fn(), stat: vi.fn() }));

const stubs = {
  "el-input": { props: ["size"], template: "<input />" },
  "el-table": { props: ["data"], template: "<div data-testid='rows'>{{ JSON.stringify(data) }}<slot /></div>" },
  "el-table-column": { template: "<div />" },
};

describe("FileBrowserPane", () => {
  beforeEach(() => vi.clearAllMocks());

  it("shows an error and no substitute files when remote browse fails", async () => {
    vi.mocked(browseRemoteDir).mockRejectedValue(new Error("SFTP unavailable"));
    const wrapper = mount(FileBrowserPane, { props: { mode: "remote", path: "/", sessionId: "s" }, global: { stubs } });
    await flushPromises();
    expect(wrapper.text()).toContain("SFTP unavailable");
    expect(wrapper.find("[data-testid='rows']").text()).toContain("[]");
  });

  it("does not read an arbitrary local directory before one is chosen", async () => {
    const wrapper = mount(FileBrowserPane, { props: { mode: "local", path: "" }, global: { stubs } });
    await flushPromises();
    expect(readDir).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("0 项");
  });
});
