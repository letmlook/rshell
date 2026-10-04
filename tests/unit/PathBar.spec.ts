import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import PathBar from "../../src/components/transfer/PathBar.vue";
import { browseRemoteDir } from "../../src/ipc/client";

vi.mock("../../src/ipc/client", () => ({ browseRemoteDir: vi.fn() }));
// 本地模式的树用 readDir 列子目录；这里替身齐全以防组件走到本地分支
vi.mock("@tauri-apps/plugin-fs", () => ({ readDir: vi.fn(), stat: vi.fn() }));

const stubs = {
  "el-input": {
    props: ["modelValue"],
    emits: ["update:modelValue"],
    template: "<input :value='modelValue' @input=\"$emit('update:modelValue', $event.target.value)\" />",
  },
};

/** 远程目录条目 */
function dirs(...names: string[]) {
  return names.map((name) => ({ name, size: 0, file_type: "Directory", modified: "0", permissions: null, owner: null }));
}

describe("PathBar 自定义路径输入", () => {
  beforeEach(() => vi.clearAllMocks());

  it("回车提交绝对路径并向上发出 navigate", async () => {
    const wrapper = mount(PathBar, {
      props: { mode: "remote", sessionId: "s1", path: "/home/user" },
      global: { stubs },
    });
    await wrapper.get('[data-test="fs-path-edit"]').trigger("click");
    const input = wrapper.get('[data-test="fs-path-input"]');
    await input.setValue("/var/log");
    await input.trigger("keyup.enter");

    expect(wrapper.emitted("navigate")?.[0]).toEqual(["/var/log"]);
    // 提交后退出编辑态
    expect(wrapper.find('[data-test="fs-path-input"]').exists()).toBe(false);
  });

  it("相对路径被拒绝并说明原因，不静默丢弃输入", async () => {
    const wrapper = mount(PathBar, {
      props: { mode: "remote", sessionId: "s1", path: "/home/user" },
      global: { stubs },
    });
    await wrapper.get('[data-test="fs-path-edit"]').trigger("click");
    const input = wrapper.get('[data-test="fs-path-input"]');
    await input.setValue("var/log");
    await input.trigger("keyup.enter");

    expect(wrapper.emitted("navigate")).toBeUndefined();
    expect(wrapper.get('[data-test="fs-path-error"]').text()).toContain("绝对路径");
  });

  it("本地模式拒绝越出已授权根目录的路径", async () => {
    const wrapper = mount(PathBar, {
      props: { mode: "local", path: "D:\\data", rootPath: "D:\\data" },
      global: { stubs },
    });
    await wrapper.get('[data-test="fs-path-edit"]').trigger("click");
    const input = wrapper.get('[data-test="fs-path-input"]');
    await input.setValue("C:\\Windows");
    await input.trigger("keyup.enter");

    expect(wrapper.emitted("navigate")).toBeUndefined();
    expect(wrapper.get('[data-test="fs-path-error"]').text()).toContain("超出已授权的根目录");
  });

  it("Esc 取消编辑且不发出 navigate", async () => {
    const wrapper = mount(PathBar, {
      props: { mode: "remote", sessionId: "s1", path: "/home/user" },
      global: { stubs },
    });
    await wrapper.get('[data-test="fs-path-edit"]').trigger("click");
    const input = wrapper.get('[data-test="fs-path-input"]');
    await input.setValue("/tmp");
    await input.trigger("keyup.esc");

    expect(wrapper.emitted("navigate")).toBeUndefined();
    expect(wrapper.find('[data-test="fs-path-input"]').exists()).toBe(false);
  });
});

describe("PathBar 目录树下拉", () => {
  beforeEach(() => vi.clearAllMocks());

  it("打开时把当前路径所在链展开，点节点即导航", async () => {
    vi.mocked(browseRemoteDir)
      .mockResolvedValueOnce({ entries: dirs("var", "home") } as never)   // 根 /
      .mockResolvedValueOnce({ entries: dirs("log", "www") } as never)   // /var
      .mockResolvedValueOnce({ entries: dirs("nginx") } as never);        // /var/log
    const wrapper = mount(PathBar, {
      props: { mode: "remote", sessionId: "s1", path: "/var/log" },
      global: { stubs },
    });

    await wrapper.get('[data-test="fs-path-tree-toggle"]').trigger("click");
    await flushPromises();

    const tree = wrapper.get('[data-test="fs-path-tree"]');
    expect(tree.text()).toContain("/");
    expect(tree.text()).toContain("var");
    expect(tree.text()).toContain("log");
    // 当前路径所在节点高亮
    expect(wrapper.get('[data-path="/var/log"]').classes()).toContain("is-current");

    await wrapper.findAll('[data-test^="fs-path-tree-label-"]')
      .find((node) => node.text() === "var")!
      .trigger("click");

    expect(wrapper.emitted("navigate")?.[0]).toEqual(["/var"]);
    // 选完自动收起
    expect(wrapper.find('[data-test="fs-path-tree"]').exists()).toBe(false);
  });

  it("未连接时说明原因，不展示成空目录", async () => {
    const wrapper = mount(PathBar, {
      props: { mode: "remote", path: "/" },
      global: { stubs },
    });
    await wrapper.get('[data-test="fs-path-tree-toggle"]').trigger("click");
    await flushPromises();

    expect(wrapper.get('[data-test="fs-path-tree"]').text()).toContain("请先连接 SSH 会话");
    expect(browseRemoteDir).not.toHaveBeenCalled();
  });

  it("目录读取失败时显示真实原因，不伪装成空目录", async () => {
    vi.mocked(browseRemoteDir).mockRejectedValueOnce(new Error("SFTP unavailable"));
    const wrapper = mount(PathBar, {
      props: { mode: "remote", sessionId: "s1", path: "/" },
      global: { stubs },
    });
    await wrapper.get('[data-test="fs-path-tree-toggle"]').trigger("click");
    await flushPromises();

    expect(wrapper.get('[data-test="fs-path-tree"]').text()).toContain("SFTP unavailable");
  });
});
