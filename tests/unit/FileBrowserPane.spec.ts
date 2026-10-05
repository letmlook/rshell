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

/** 能双向绑定的 el-input 桩：PathBar 的路径输入走 v-model，存根必须真的回写 */
const editableStubs = {
  ...stubs,
  "el-input": {
    props: ["modelValue"],
    emits: ["update:modelValue"],
    template: '<input :value="modelValue" @input="$emit(\'update:modelValue\', $event.target.value)" />',
  },
};

/** 经 PathBar 手输路径（真实的两段链路：commitEdit → navigate → navigateTo） */
async function typePath(target: string) {
  const wrapper = mount(FileBrowserPane, {
    props: { mode: "local", path: "/home/user", rootPath: "/home/user" },
    global: { stubs: editableStubs },
  });
  await flushPromises();
  await wrapper.get('[data-test="fs-path-edit"]').trigger("click");
  const input = wrapper.get('[data-test="fs-path-input"]');
  await input.setValue(target);
  await input.trigger("keyup.enter");
  await flushPromises();
  return wrapper;
}

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

// R3-10：navigateTo 曾用 `path.startsWith(rootPath + "/")` 判越界，
// `/home/user/../etc` 通过后成为 internalLocalPath —— 而后者就是
// enqueueDownload 的落盘目录，于是下载写到了用户选定的根之外。
// 现在两个入口共用 rootBoundary.isWithinRoot，下表对两个入口的结论必须一致。
describe("FileBrowserPane 已授权根目录边界", () => {
  beforeEach(() => vi.clearAllMocks());

  it.each(["/home/user/../etc", "/home/user/../../etc", "/home/user2", "/home/user2/docs"])(
    "拒绝越界导航 %s：不向上发出 navigate，也不读该目录",
    async (target) => {
      const wrapper = await typePath(target);
      expect(wrapper.emitted("navigate")).toBeUndefined();
      expect(vi.mocked(readDir).mock.calls.map(([dir]) => dir)).not.toContain(target);
    },
  );

  it.each(["/home/user/docs", "/home/user/./docs"])("放行根内导航 %s", async (target) => {
    const wrapper = await typePath(target);
    expect(wrapper.emitted("navigate")?.[0]).toEqual([target]);
  });

  it("放行根目录自身", async () => {
    const wrapper = await typePath("/home/user");
    // 等于当前路径：不重复导航，也不报错
    expect(wrapper.emitted("navigate")).toBeUndefined();
    expect(wrapper.find('[data-test="fs-path-error"]').exists()).toBe(false);
  });

  // 同一个 Windows 根必须经两个入口都能进：旧实现下 navigateTo 只判 `/`，
  // 会把 PathBar 已经放行的 `C:\data\docs` 又挡回去。
  it("Windows 根 C:\\data 的子目录经手输入口被放行", async () => {
    const wrapper = mount(FileBrowserPane, {
      props: { mode: "local", path: "C:\\data", rootPath: "C:\\data" },
      global: { stubs: editableStubs },
    });
    await flushPromises();
    await wrapper.get('[data-test="fs-path-edit"]').trigger("click");
    const input = wrapper.get('[data-test="fs-path-input"]');
    await input.setValue("C:\\data\\docs");
    await input.trigger("keyup.enter");
    await flushPromises();

    expect(wrapper.emitted("navigate")?.[0]).toEqual(["C:\\data\\docs"]);
    expect(vi.mocked(readDir).mock.calls.map(([dir]) => dir)).toContain("C:\\data\\docs");
  });

  it("拒绝穿越出 Windows 根的导航", async () => {
    const wrapper = mount(FileBrowserPane, {
      props: { mode: "local", path: "C:\\data", rootPath: "C:\\data" },
      global: { stubs: editableStubs },
    });
    await flushPromises();
    await wrapper.get('[data-test="fs-path-edit"]').trigger("click");
    const input = wrapper.get('[data-test="fs-path-input"]');
    await input.setValue("C:\\data\\..\\Windows");
    await input.trigger("keyup.enter");
    await flushPromises();

    expect(wrapper.emitted("navigate")).toBeUndefined();
    expect(vi.mocked(readDir).mock.calls.map(([dir]) => dir)).not.toContain("C:\\Windows");
  });
});
