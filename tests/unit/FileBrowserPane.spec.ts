import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import FileBrowserPane from "../../src/components/transfer/FileBrowserPane.vue";
import { browseRemoteDir } from "../../src/ipc/client";
import { readDir, stat } from "@tauri-apps/plugin-fs";

vi.mock("../../src/ipc/client", () => ({ browseRemoteDir: vi.fn() }));
vi.mock("@tauri-apps/plugin-fs", () => ({ readDir: vi.fn(), stat: vi.fn() }));

const stubs = {
  "el-input": { props: ["size"], template: "<input />" },
  "el-table": {
    name: "el-table",
    props: ["data"],
    template: "<div data-testid='rows'>{{ JSON.stringify(data) }}<slot /></div>",
  },
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

  // 目录读不到时，界面不得只甩一句英文原始错误：用户要知道是「目录没了」
  // 还是「没权限」，以及下一步做什么。
  it("本地目录读不到时给出中文原因与下一步，而不是只显示原始英文错误", async () => {
    vi.mocked(readDir).mockRejectedValue(
      new Error("failed to read directory at path: /tmp/gone with error: No such file or directory (os error 2)"),
    );
    const wrapper = mount(FileBrowserPane, {
      props: { mode: "local", path: "/tmp/gone", rootPath: "/tmp" },
      global: { stubs },
    });
    await flushPromises();
    const text = wrapper.text();
    expect(text).toContain("不存在");
    expect(text).toContain("/tmp/gone");
    expect(text).toContain("更换本地目录");
    expect(text, "不得只剩英文原始错误").not.toContain("failed to read directory at path");
  });

  it("本地目录没权限时给出中文原因", async () => {
    vi.mocked(readDir).mockRejectedValue(
      new Error("failed to read directory at path: /root with error: Permission denied (os error 13)"),
    );
    const wrapper = mount(FileBrowserPane, {
      props: { mode: "local", path: "/root", rootPath: "/" },
      global: { stubs },
    });
    await flushPromises();
    const text = wrapper.text();
    expect(text).toContain("无权读取");
    expect(text).toContain("/root");
  });

  it("does not read an arbitrary local directory before one is chosen", async () => {
    const wrapper = mount(FileBrowserPane, { props: { mode: "local", path: "" }, global: { stubs } });
    await flushPromises();
    expect(readDir).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("0 项");
  });

  // readDir 不跟随符号链接：macOS 的 /tmp、/var、/etc 会被报成「文件」，
  // 图标与双击进入都会失准。面板必须以 stat（跟随符号链接）的类型为准。
  it("本地列表按 stat 的类型把符号链接目录当作文件夹", async () => {
    vi.mocked(readDir).mockResolvedValue([
      { name: "tmp", isDirectory: false, isFile: false, isSymlink: true },
    ] as never);
    vi.mocked(stat).mockResolvedValue({
      isDirectory: true,
      isFile: false,
      isSymlink: true,
      size: 0,
      mtime: null,
    } as never);
    const wrapper = mount(FileBrowserPane, {
      props: { mode: "local", path: "/", rootPath: "/" },
      global: { stubs },
    });
    await flushPromises();

    expect(wrapper.find("[data-testid='rows']").text()).toContain('"is_dir":true');
  });
});

/**
 * 多选。
 *
 * toggleSelect 的 ctrlKey/shiftKey 两个形参之前恒为 false —— 调用点永远传
 * (false, false)，于是多选在界面上根本做不到，「批量上传/批量下载」这条
 * 后端支持的路径不可达，状态行的「已选 N 项」也永远只有 1。
 */
describe("FileBrowserPane 多选", () => {
  // 远端列表走 SFTP 协议字段（file_type），不是本地的 is_dir
  const remoteFiles = [
    { name: "a.txt", size: 1, file_type: "File", modified: "0", owner: "u", permissions: "rw-r--r--" },
    { name: "b.txt", size: 1, file_type: "File", modified: "0", owner: "u", permissions: "rw-r--r--" },
    { name: "c.txt", size: 1, file_type: "File", modified: "0", owner: "u", permissions: "rw-r--r--" },
  ];

  async function mountRemote() {
    vi.mocked(browseRemoteDir).mockResolvedValue({ entries: remoteFiles } as never);
    const wrapper = mount(FileBrowserPane, {
      props: { mode: "remote", path: "/", sessionId: "s" },
      global: { stubs },
    });
    await flushPromises();
    return wrapper;
  }

  /** 触发 el-table 的 row-click（签名 row, column, event） */
  async function clickRow(wrapper: VueWrapper, name: string, modifiers: { ctrlKey?: boolean; shiftKey?: boolean } = {}) {
    const event = new MouseEvent("click", { bubbles: true, cancelable: true, ...modifiers });
    const table = wrapper.findComponent({ name: "el-table" });
    const row = remoteFiles.find((f) => f.name === name)!;
    table.vm.$emit("row-click", { name: row.name, size: row.size, is_dir: false, modified: "" }, null, event);
    await flushPromises();
  }

  beforeEach(() => vi.clearAllMocks());

  it("Ctrl+点击把行加入选中集（多选可达）", async () => {
    const wrapper = await mountRemote();
    await clickRow(wrapper, "a.txt");
    await clickRow(wrapper, "b.txt", { ctrlKey: true });

    const selected = wrapper.emitted("selection-change")?.at(-1)?.[0] as { name: string }[];
    expect(selected?.map((x) => x.name)).toEqual(["a.txt", "b.txt"]);
  });

  it("Ctrl+点击已选中的行会把它移出选中集", async () => {
    const wrapper = await mountRemote();
    await clickRow(wrapper, "a.txt");
    await clickRow(wrapper, "b.txt", { ctrlKey: true });
    await clickRow(wrapper, "a.txt", { ctrlKey: true });

    const selected = wrapper.emitted("selection-change")?.at(-1)?.[0] as { name: string }[];
    expect(selected?.map((x) => x.name)).toEqual(["b.txt"]);
  });

  it("Shift+点击从锚点行选到该行（区间选择）", async () => {
    const wrapper = await mountRemote();
    await clickRow(wrapper, "a.txt");
    await clickRow(wrapper, "c.txt", { shiftKey: true });

    const selected = wrapper.emitted("selection-change")?.at(-1)?.[0] as { name: string }[];
    expect(selected?.map((x) => x.name)).toEqual(["a.txt", "b.txt", "c.txt"]);
  });

  it("无修饰键点击只选中该行（多选被替换）", async () => {
    const wrapper = await mountRemote();
    await clickRow(wrapper, "a.txt");
    await clickRow(wrapper, "b.txt", { ctrlKey: true });
    await clickRow(wrapper, "c.txt");

    const selected = wrapper.emitted("selection-change")?.at(-1)?.[0] as { name: string }[];
    expect(selected?.map((x) => x.name)).toEqual(["c.txt"]);
  });

  it("状态行的「已选 N 项」反映真实选中数", async () => {
    const wrapper = await mountRemote();
    await clickRow(wrapper, "a.txt");
    await clickRow(wrapper, "b.txt", { ctrlKey: true });

    expect(wrapper.text()).toContain("已选 2 项");
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
