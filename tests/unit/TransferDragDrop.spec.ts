import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { defineComponent } from "vue";
import TransferWorkspace from "../../src/components/transfer/TransferWorkspace.vue";
import FileBrowserPane from "../../src/components/transfer/FileBrowserPane.vue";
import { enqueueUpload, enqueueDownload, browseRemoteDir } from "../../src/ipc/client";
import { clickPending, dragState, hoverPane } from "../../src/utils/transferDrag";

vi.mock("../../src/ipc/client", () => ({
  enqueueUpload: vi.fn().mockResolvedValue(undefined),
  enqueueDownload: vi.fn().mockResolvedValue(undefined),
  createRemoteDirectory: vi.fn().mockResolvedValue(undefined),
  deleteRemoteEntry: vi.fn().mockResolvedValue(undefined),
  getRemoteHomeDir: vi.fn().mockResolvedValue("/"),
  browseRemoteDir: vi.fn().mockResolvedValue({ entries: [] }),
}));
vi.mock("@tauri-apps/plugin-fs", () => ({ readDir: vi.fn().mockResolvedValue([]) }));
vi.mock("../../src/utils/dialog", () => ({
  confirmDialog: vi.fn().mockResolvedValue(false),
  openDialog: vi.fn().mockResolvedValue(null),
  promptDialog: vi.fn().mockResolvedValue(null),
}));
vi.mock("../../src/utils/pathPicker", () => ({ pickLocalPath: vi.fn().mockResolvedValue(null) }));
vi.mock("element-plus", () => ({ ElMessage: { error: vi.fn() } }));

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

/**
 * 应用自己的右键菜单是 DOM 元素：右键命中菜单项时，
 * WebView 原生菜单（Reload / Inspect Element）会叠上来盖住它。
 * 菜单 DOM 必须自己吞掉 contextmenu 的默认行为。
 */
describe("应用右键菜单不得与 WebView 原生菜单叠加", () => {
  beforeEach(() => vi.clearAllMocks());

  it("传输面板的右键菜单吞掉 contextmenu 默认行为", async () => {
    const wrapper = workspace();
    const remote = wrapper.findAllComponents(FileBrowserPane)[1];
    const entry = { name: "a.txt", is_dir: false, size: 1, modified: "" };
    remote.vm.$emit("selection-change", [entry]);
    await flushPromises();
    remote.vm.$emit("row-context-menu", { entries: [entry], x: 10, y: 20 });
    await flushPromises();

    const menu = wrapper.find('[data-test="context-menu"]');
    expect(menu.exists()).toBe(true);
    // 在菜单项上右键：默认行为必须被取消（原生菜单不弹出）
    const event = new Event("contextmenu", { cancelable: true, bubbles: true });
    menu.element.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
  });
});

/**
 * 面板间拖拽：本地 ⇄ 远程 互拖 = 上传 / 下载。
 * 之前拖拽完全没实现（只有计划文档里的待办），传输面板没有任何拖放入口。
 */
describe("传输面板拖拽上传下载", () => {
  beforeEach(() => vi.clearAllMocks());

  it("把远端文件拖到本地面板 = 入队下载", async () => {
    const wrapper = workspace();
    const local = wrapper.findAllComponents(FileBrowserPane)[0];
    local.vm.$emit("drop-entries", {
      from: "remote",
      to: "local",
      entries: [{ name: "a.txt", is_dir: false, size: 1, modified: "" }],
    });
    await flushPromises();

    expect(enqueueDownload).toHaveBeenCalledOnce();
    expect(enqueueDownload).toHaveBeenCalledWith("/a.txt", "/Users/test/files/a.txt", "session-1", "Fail");
  });

  it("把本地文件拖到远端面板 = 入队上传", async () => {
    const wrapper = workspace();
    const remote = wrapper.findAllComponents(FileBrowserPane)[1];
    remote.vm.$emit("drop-entries", {
      from: "local",
      to: "remote",
      entries: [{ name: "b.txt", is_dir: false, size: 1, modified: "" }],
    });
    await flushPromises();

    expect(enqueueUpload).toHaveBeenCalledOnce();
    expect(enqueueUpload).toHaveBeenCalledWith("/Users/test/files/b.txt", "/b.txt", "session-1", "Fail");
  });

  // 拖回本面板不做任何事：不能把远端文件拖成"下载到远端自己"
  it("拖回来源面板不做任何传输", async () => {
    const wrapper = workspace();
    const remote = wrapper.findAllComponents(FileBrowserPane)[1];
    remote.vm.$emit("drop-entries", {
      from: "remote",
      to: "remote",
      entries: [{ name: "a.txt", is_dir: false, size: 1, modified: "" }],
    });
    await flushPromises();

    expect(enqueueUpload).not.toHaveBeenCalled();
    expect(enqueueDownload).not.toHaveBeenCalled();
  });

  // 目录不能拖：后端一次只传一个普通文件，目录递归不在契约内
  it("拖动目录不入队（后端只支持单个普通文件）", async () => {
    const wrapper = workspace();
    const remote = wrapper.findAllComponents(FileBrowserPane)[1];
    remote.vm.$emit("drop-entries", {
      from: "local",
      to: "remote",
      entries: [{ name: "folder", is_dir: true, size: 0, modified: "" }],
    });
    await flushPromises();

    expect(enqueueUpload).not.toHaveBeenCalled();
  });
});

/**
 * FileBrowserPane 的拖放事件链：dragstart → dragover → drop。
 *
 * 关键点：dragover 阶段**不能**依赖 dataTransfer.types 判断「是不是本应用的
 * 拖拽」—— WebKit 在 dragover 阶段未必暴露自定义 MIME 类型，判不到就不
 * preventDefault，drop 事件根本不会被允许触发（真机上拖拽完全没反应）。
 * 改用进程内标志位（utils/transferDrag）传递拖拽来源。
 */
/**
 * 未选本地目录时，拖拽不能把文件写到文件系统根目录。
 *
 * localRoot 是**唯一**的本地落点来源：没有它 internalLocalPath 是空串，
 * joinPath("", name) 会拼成 "/name" —— 直接写根目录。工具栏与右键菜单都被
 * capabilities 的 localRoot 闸门挡住，拖拽这条路必须守同一道闸门。
 */
describe("TransferWorkspace 拖拽的本地落点边界", () => {
  beforeEach(() => vi.clearAllMocks());

  it("未选本地目录时拖到本地面板不入队（不会写到 /）", async () => {
    const wrapper = mount(TransferWorkspace, {
      props: { sessionId: "session-1", connected: true },
      global: { stubs: { FileBrowserPane: paneStub } },
    });
    await flushPromises();

    wrapper.findAllComponents(FileBrowserPane)[0].vm.$emit("drop-entries", {
      from: "remote",
      to: "local",
      entries: [{ name: "secret.txt", is_dir: false, size: 1, modified: "" }],
    });
    await flushPromises();

    expect(enqueueDownload).not.toHaveBeenCalled();
  });

  it("未选本地目录时拖到远端面板不入队（本地源路径不成立）", async () => {
    const wrapper = mount(TransferWorkspace, {
      props: { sessionId: "session-1", connected: true },
      global: { stubs: { FileBrowserPane: paneStub } },
    });
    await flushPromises();

    wrapper.findAllComponents(FileBrowserPane)[1].vm.$emit("drop-entries", {
      from: "local",
      to: "remote",
      entries: [{ name: "a.txt", is_dir: false, size: 1, modified: "" }],
    });
    await flushPromises();

    expect(enqueueUpload).not.toHaveBeenCalled();
  });
});

describe("FileBrowserPane 拖放事件链", () => {
  const remoteFiles = [
    { name: "a.txt", size: 1, file_type: "File", modified: "0", owner: "u", permissions: "rw-r--r--" },
  ];

  const stubs = {
    "el-input": { props: ["size"], template: "<input />" },
    "el-table": {
      name: "el-table",
      props: ["data"],
      // 渲染列的默认 slot，行内容（文件名单元格）才会出现在 DOM 里
      template: "<div data-testid='rows'>{{ JSON.stringify(data) }}<slot /></div>",
    },
    "el-table-column": {
      props: ["prop", "label"],
      template: "<div><slot :row=\"{ name: 'a.txt', size: 1, is_dir: false, modified: '' }\" /></div>",
    },
  };

  beforeEach(() => {
    vi.clearAllMocks();
    dragState.value = null;
    hoverPane.value = null;
  });

  async function mountRemote() {
    vi.mocked(browseRemoteDir).mockResolvedValue({ entries: remoteFiles } as never);
    const wrapper = mount(FileBrowserPane, {
      props: { mode: "remote", path: "/", sessionId: "s" },
      global: { stubs },
    });
    await flushPromises();
    return wrapper;
  }

  /** 让 elementFromPoint 返回指定面板（jsdom 不做布局，指针命中要手动指定） */
  function stubPointerTarget(mode: "local" | "remote" | null) {
    const original = (document as Document & { elementFromPoint?: unknown }).elementFromPoint;
    (document as Document & { elementFromPoint: unknown }).elementFromPoint = () => {
      if (!mode) return null;
      const host = document.createElement("div");
      host.setAttribute("data-pane", mode);
      return host;
    };
    return () => {
      (document as Document & { elementFromPoint: unknown }).elementFromPoint = original;
    };
  }

  /** 在 window 上派发指针事件（拖拽监听挂在 window 上） */
  function firePointer(type: string, init: { clientX: number; clientY: number; button?: number }) {
    const event = new Event(type, { bubbles: true, cancelable: true });
    Object.assign(event, init);
    window.dispatchEvent(event);
  }

  /** 按下行 → 移动 → 抬起（指针事件序列） */
  async function dragRow(wrapper: VueWrapper, to: { x: number; y: number }) {
    const nameCell = wrapper.find('[data-test="fs-row-name"]');
    await nameCell.trigger("pointerdown", { button: 0, clientX: 10, clientY: 10 });
    firePointer("pointermove", { clientX: to.x, clientY: to.y });
    firePointer("pointerup", { clientX: to.x, clientY: to.y });
    await flushPromises();
  }

  it("指针越过阈值后置位拖拽状态", async () => {
    const wrapper = await mountRemote();
    stubPointerTarget("local");
    const nameCell = wrapper.find('[data-test="fs-row-name"]');
    await nameCell.trigger("pointerdown", { button: 0, clientX: 10, clientY: 10 });
    expect(dragState.value).toBeNull();

    firePointer("pointermove", { clientX: 60, clientY: 40 });
    expect(dragState.value).toEqual({ from: "remote", names: ["a.txt"] });
  });

  // 位移不够就是普通点击，不算拖拽 —— 否则每次点行都会被当成拖拽
  it("位移未越过阈值不算拖拽", async () => {
    const wrapper = await mountRemote();
    const nameCell = wrapper.find('[data-test="fs-row-name"]');
    await nameCell.trigger("pointerdown", { button: 0, clientX: 10, clientY: 10 });
    firePointer("pointermove", { clientX: 12, clientY: 11 });

    expect(dragState.value).toBeNull();
    expect(dragState.value).toBeNull();
  });

  it("拖到另一个面板上抬起 = 投放（发出 drop-entries）", async () => {
    const wrapper = await mountRemote();
    stubPointerTarget("local");
    await dragRow(wrapper, { x: 300, y: 200 });

    const dropped = wrapper.emitted("drop-entries")?.at(-1)?.[0] as
      { from: string; to: string; entries: { name: string }[] };
    expect(dropped?.from).toBe("remote");
    // 抬起发生在来源面板，所以目标面板必须由来源面板判定并带上来
    expect(dropped?.to).toBe("local");
    expect(dropped?.entries.map((e) => e.name)).toEqual(["a.txt"]);
    // 拖完要清干净共享状态，否则下一个拖拽会带上一次的信息
    expect(dragState.value).toBeNull();
    expect(hoverPane.value).toBeNull();
  });

  it("拖回本面板不投放", async () => {
    const wrapper = await mountRemote();
    stubPointerTarget("remote");
    await dragRow(wrapper, { x: 300, y: 200 });

    expect(wrapper.emitted("drop-entries")).toBeUndefined();
  });

  it("拖到面板外不投放", async () => {
    const wrapper = await mountRemote();
    stubPointerTarget(null);
    await dragRow(wrapper, { x: 300, y: 200 });

    expect(wrapper.emitted("drop-entries")).toBeUndefined();
  });

  // 拖过之后浏览器仍会派发 click（指针起止在同一行时），不吞掉的话
  // 「拖一下」会顺手改掉选中集
  it("拖拽后紧随的那次 click 被吞掉（不改选中集）", async () => {
    const wrapper = await mountRemote();
    stubPointerTarget("local");
    const row = wrapper.find('[data-test="fs-row-name"]');
    const before = wrapper.emitted("selection-change")?.length ?? 0;

    await row.trigger("pointerdown", { button: 0, clientX: 10, clientY: 10 });
    firePointer("pointermove", { clientX: 60, clientY: 40 });
    firePointer("pointerup", { clientX: 60, clientY: 40 });
    await flushPromises();

    // 拖拽本身不改选中集；随后的 click 必须被吞掉（默认行为被取消），
    // 否则「拖一下」会顺手改掉选中集。
    // click 与 pointerup 在同一 tick 内派发（标志位还没被定时器复位）。
    const clickEvent = new Event("click", { bubbles: true, cancelable: true });
    row.element.dispatchEvent(clickEvent);

    expect(clickEvent.defaultPrevented).toBe(true);
    const after = wrapper.emitted("selection-change")?.length ?? 0;
    expect(after).toBe(before);
  });

  // 跨面板拖拽时，click 按 DOM 规范派发到两个面板的公共祖先（不在面板内），
  // 标志位若留着就会把面板内**下一个**点击也吞掉（用户看到「点了没反应」）
  it("跨面板拖拽后，面板内下一个点击不被吞掉", async () => {
    vi.useFakeTimers();
    try {
      const wrapper = await mountRemote();
      stubPointerTarget("local");
      const row = wrapper.find('[data-test="fs-row-name"]');
      await row.trigger("pointerdown", { button: 0, clientX: 10, clientY: 10 });
      firePointer("pointermove", { clientX: 60, clientY: 40 });
      firePointer("pointerup", { clientX: 60, clientY: 40 });

      // 跨面板拖拽时 click 按 DOM 规范派发到两个面板的公共祖先，
      // 面板内的拦截器**看不到**它 —— 标志位必须靠「下一 tick 复位」回收，
      // 否则它会一直留着，把面板内的下一个点击也吞掉。
      vi.advanceTimersByTime(1);
      expect(clickPending.value).toBe(false);

      // 面板内的下一个点击必须正常放行
      const innerClick = new Event("click", { bubbles: true, cancelable: true });
      wrapper.find('[data-test="fs-row-name"]').element.dispatchEvent(innerClick);
      expect(innerClick.defaultPrevented).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });


  /**
   * 拖拽不能拉出文本选区。
   *
   * WebKit 在 mousedown 的默认动作里就把选区锚点定下来了 —— 等 pointermove
   * 发现是拖拽再改 user-select 已经太晚，拖过去会拖出一条跨面板的文本高亮。
   * 所以必须在 **capture 阶段**拦 mousedown，取消默认的文本选择。
   */
  it("从文件名单元格起手时取消浏览器的文本选择（capture 阶段）", async () => {
    const wrapper = await mountRemote();
    const row = wrapper.find('[data-test="fs-row-name"]');

    const down = new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 });
    row.element.dispatchEvent(down);
    expect(down.defaultPrevented).toBe(true);
  });

  // 面板其它区域（表头、搜索框）的文本仍要能正常选中
  it("面板其它区域的鼠标按下不被拦（文本仍可选）", async () => {
    const wrapper = await mountRemote();
    const list = wrapper.find('[data-test="fs-list"]');

    const down = new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 });
    list.element.dispatchEvent(down);
    expect(down.defaultPrevented).toBe(false);
  });

  // pointercancel 是手势被系统取消，没有可靠落点，不能当 pointerup 投放
  it("pointercancel 只清理不投放", async () => {
    const wrapper = await mountRemote();
    stubPointerTarget("local");
    const row = wrapper.find('[data-test="fs-row-name"]');
    await row.trigger("pointerdown", { button: 0, clientX: 10, clientY: 10 });
    firePointer("pointermove", { clientX: 60, clientY: 40 });
    firePointer("pointercancel", { clientX: 60, clientY: 40 });
    await flushPromises();

    expect(wrapper.emitted("drop-entries")).toBeUndefined();
  });

  // 组件卸载时 window 上的指针监听必须摘干净
  it("卸载时摘掉 window 上的拖拽监听", async () => {
    const wrapper = await mountRemote();
    stubPointerTarget("local");
    const row = wrapper.find('[data-test="fs-row-name"]');
    await row.trigger("pointerdown", { button: 0, clientX: 10, clientY: 10 });
    firePointer("pointermove", { clientX: 60, clientY: 40 });

    wrapper.unmount();
    // 卸载后再移动指针不应再有任何反应（监听已摘）
    expect(() => firePointer("pointermove", { clientX: 300, clientY: 300 })).not.toThrow();
    expect(dragState.value).toBeNull();
  });

  // 目标面板要能高亮提示「能放这儿」
  it("指针停在另一个面板上时，本面板不高亮；对方面板才高亮", async () => {
    const wrapper = await mountRemote();
    stubPointerTarget("local");
    const nameCell = wrapper.find('[data-test="fs-row-name"]');
    await nameCell.trigger("pointerdown", { button: 0, clientX: 10, clientY: 10 });
    firePointer("pointermove", { clientX: 60, clientY: 40 });

    // 悬停在 local 上，而本面板是 remote → 本面板不是目标
    expect(wrapper.find('[data-test="fs-list"]').classes()).not.toContain("is-drop-target");
  });

  /**
   * 投放高亮必须真的出现。
   *
   * 判定只看**共享状态**（正在拖拽 / 指针停在哪个面板 / 来源是哪个面板）：
   * 「是否正在拖拽」是来源面板的实例级状态，混进判定会让目标面板永远不高亮
   * —— 而目标面板恰恰是最需要提示的那个。
   */
  it("指针停在目标面板上时，目标面板高亮", async () => {
    // 源面板 = local，目标面板 = remote：源面板先按下并移动
    const source = mount(FileBrowserPane, {
      props: { mode: "local", path: "/", rootPath: "/" },
      global: { stubs },
    });
    await flushPromises();
    const target = mount(FileBrowserPane, {
      props: { mode: "remote", path: "/", sessionId: "s" },
      global: { stubs },
    });
    await flushPromises();

    // elementFromPoint 命中目标面板的行
    stubPointerTarget("remote");
    await source.find('[data-test="fs-row-name"]').trigger("pointerdown", { button: 0, clientX: 10, clientY: 10 });
    firePointer("pointermove", { clientX: 60, clientY: 40 });

    // computed 重算 → DOM 更新要等一次渲染 tick
    await flushPromises();
    expect(target.find('[data-test="fs-list"]').classes()).toContain("is-drop-target");
    expect(source.find('[data-test="fs-list"]').classes()).not.toContain("is-drop-target");
  });
});
