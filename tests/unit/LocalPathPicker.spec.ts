import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import LocalPathPicker from "../../src/components/dialogs/LocalPathPicker.vue";
import { pickLocalPath, request, resetPathPicker } from "../../src/utils/pathPicker";
import { readDir, stat } from "@tauri-apps/plugin-fs";

vi.mock("@tauri-apps/plugin-fs", () => ({ readDir: vi.fn(), stat: vi.fn(), mkdir: vi.fn() }));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn().mockResolvedValue("C:\\Users\\test") }));
// PathBar 复用组件，桩掉以隔离其目录树逻辑
vi.mock("../../src/components/transfer/PathBar.vue", () => ({
  default: { name: "PathBar", props: ["mode", "path"], template: '<div class="path-bar-stub" />' },
}));

/** 各返回一个元素的数组，便于 `[...dirEntry("a"), ...fileEntry("b")]` 拼列表 */
const dirEntry = (name: string) => [{ name, isDirectory: true }] as never;
const fileEntry = (name: string) => [{ name, isDirectory: false }] as never;

function info(over: Record<string, unknown> = {}) {
  return { size: 10, mtime: new Date(2026, 0, 2), ...over } as never;
}

function mountPicker(props: Record<string, unknown> = {}) {
  return mount(LocalPathPicker, {
    props: { open: true, mode: "directory", title: "选择本地文件夹", initialPath: "C:\\data", ...props },
    global: { stubs: { teleport: true } },
  });
}

/**
 * 这个选择器替掉了 tauri-plugin-dialog 的 open()（操作系统原生窗口）。
 * 关键契约：目录/文件分类正确、目录优先、失败显示真实原因、选完 emit 绝对路径。
 */
describe("LocalPathPicker", () => {
  beforeEach(() => {
    resetPathPicker();
    // resetAllMocks：连同上一次残留的 mockResolvedValueOnce 队列一起清掉，
    // 否则前一个用例没消费完的返回值会漏进下一个用例（表现为"行渲染不出来"）
    vi.resetAllMocks();
    vi.mocked(stat).mockResolvedValue(info());
    vi.mocked(readDir).mockResolvedValue([] as never);
  });

  it("目录模式只列子目录，按钮是「选择此文件夹」并回传当前目录", async () => {
    vi.mocked(readDir).mockResolvedValue([...dirEntry("sub"), ...fileEntry("a.txt")] as never);
    const wrapper = mountPicker();
    await flushPromises();

    expect(wrapper.text()).toContain("sub");
    expect(wrapper.text()).not.toContain("a.txt");
    expect(wrapper.get('[data-test="picker-submit"]').text()).toBe("选择此文件夹");

    await wrapper.get('[data-test="picker-submit"]').trigger("click");
    expect(wrapper.emitted("close")?.[0]).toEqual(["C:\\data"]);
  });

  it("文件模式列出文件，单击选中后才能打开", async () => {
    vi.mocked(readDir).mockResolvedValue([...dirEntry("sub"), ...fileEntry("id_ed25519")] as never);
    const wrapper = mountPicker({ mode: "file", title: "选择私钥文件" });
    await flushPromises();

    const submitDisabled = () =>
      (wrapper.get('[data-test="picker-submit"]').element as HTMLButtonElement).disabled;
    expect(submitDisabled()).toBe(true);

    await wrapper.get('[data-test="picker-row-id_ed25519"]').trigger("click");
    // 交互后重新查询：teleport stub 在重渲染时会替换节点，缓存的 DOM 引用会失效
    // （浏览器里 Teleport 不会这样，这里只是测试桩的行为）
    expect(submitDisabled()).toBe(false);
    await wrapper.get('[data-test="picker-submit"]').trigger("click");
    expect(wrapper.emitted("close")?.[0]).toEqual(["C:\\data\\id_ed25519"]);
  });

  it("accept 过滤掉不像私钥的文件", async () => {
    vi.mocked(readDir).mockResolvedValue([...fileEntry("notes.txt"), ...fileEntry("id_ed25519")] as never);
    const wrapper = mountPicker({ mode: "file", accept: (name: string) => !name.endsWith(".txt") });
    await flushPromises();

    expect(wrapper.text()).toContain("id_ed25519");
    expect(wrapper.text()).not.toContain("notes.txt");
  });

  it("双击目录进入，取消不 emit 结果", async () => {
    vi.mocked(readDir)
      .mockResolvedValueOnce(dirEntry("sub") as never)
      .mockResolvedValueOnce(fileEntry("inner.txt") as never);
    const wrapper = mountPicker();
    await flushPromises();

    await wrapper.get('[data-test="picker-row-sub"]').trigger("dblclick");
    await flushPromises();
    expect(readDir).toHaveBeenLastCalledWith("C:\\data\\sub");

    await wrapper.get('[data-test="picker-cancel"]').trigger("click");
    expect(wrapper.emitted("close")?.[0]).toEqual([null]);
  });

  it("目录读取失败显示真实原因，不伪装成空目录", async () => {
    vi.mocked(readDir).mockRejectedValue(new Error("os error 3: path not found"));
    const wrapper = mountPicker();
    await flushPromises();

    expect(wrapper.get('[data-test="picker-error"]').text()).toContain("目录不存在");
  });

  it("隐藏项默认不显示，勾选后出现", async () => {
    vi.mocked(readDir).mockResolvedValue([...dirEntry(".git"), ...dirEntry("sub")] as never);
    const wrapper = mountPicker();
    await flushPromises();
    expect(wrapper.text()).not.toContain(".git");

    await wrapper.get('input[type="checkbox"]').setValue(true);
    expect(wrapper.text()).toContain(".git");
  });
});

describe("pathPicker 服务", () => {
  beforeEach(() => resetPathPicker());

  it("请求进入槽位后由 resolve 结算", async () => {
    const promise = pickLocalPath({ mode: "file", title: "选私钥" });
    expect(request.value?.title).toBe("选私钥");

    const { resolvePathPicker } = await import("../../src/utils/pathPicker");
    resolvePathPicker("C:\\keys\\id_ed25519");
    await expect(promise).resolves.toBe("C:\\keys\\id_ed25519");
    expect(request.value).toBeNull();
  });

  it("取消返回 null", async () => {
    const promise = pickLocalPath({ mode: "directory", title: "选目录" });
    const { resolvePathPicker } = await import("../../src/utils/pathPicker");
    resolvePathPicker(null);
    await expect(promise).resolves.toBeNull();
  });
});
