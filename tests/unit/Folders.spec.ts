import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import ElementPlus from "element-plus";
import SessionList from "../../src/components/SessionList.vue";
import { useFoldersStore } from "../../src/stores/folders";
import { useSessionsStore } from "../../src/stores/sessions";
import type { Folder, SessionConfig } from "../../src/ipc/types";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const { confirmDialogMock } = vi.hoisted(() => ({ confirmDialogMock: vi.fn() }));
vi.mock("../../src/utils/dialog", () => ({ confirmDialog: confirmDialogMock }));

const folders: Folder[] = [
  { id: "folder-1", name: "生产" },
  { id: "folder-2", name: "测试" },
];

function session(id: string, name: string, folderId: string | null): SessionConfig {
  return {
    id,
    name,
    folder_id: folderId,
    host: `${id}.example.test`,
    port: 22,
    protocol: "SSH",
    auth_method: { Password: { username: "alice", has_password: false } },
    serial_config: null,
  };
}

const sessions = [
  session("s-prod", "prod-host", "folder-1"),
  session("s-test", "test-host", "folder-2"),
  session("s-plain", "plain-host", null),
];

const stubs = {
  "el-input": {
    props: ["modelValue"],
    emits: ["update:modelValue"],
    template:
      '<input :value="modelValue" @input="$emit(\'update:modelValue\', $event.target.value)" />',
  },
  "el-button": {
    props: ["disabled"],
    template: '<button :disabled="disabled"><slot /></button>',
  },
  "el-tooltip": { template: "<span><slot /></span>" },
  "el-empty": { props: ["description"], template: '<div>{{ description }}</div>' },
};

async function mountList() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const wrapper = mount(SessionList, {
    attachTo: document.body,
    global: { plugins: [pinia, ElementPlus], stubs },
  });
  // 列表数据由 store.refresh 拉取（与 SessionListContextMenu.spec.ts 同款）
  await useSessionsStore(pinia).refresh();
  await flushPromises();
  return wrapper;
}

describe("连接分组", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    confirmDialogMock.mockResolvedValue(true);
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") return sessions;
      if (command === "list_session_load_issues") return [];
      if (command === "list_folders") return folders;
      if (command === "create_folder") return "folder-3";
      if (command === "delete_folder") return null;
      return undefined;
    });
  });

  it("按真实分组把连接分到各组，而不是全部塞进「默认」", async () => {
    const wrapper = await mountList();
    const text = wrapper.text();
    expect(text).toContain("生产");
    expect(text).toContain("测试");
    expect(text).toContain("默认");
    // 每个连接落在自己的组里
    expect(text).toContain("prod-host");
    expect(text).toContain("test-host");
    expect(text).toContain("plain-host");
    wrapper.unmount();
  });

  it("分组被删除后，该组的连接回到「默认」而不是消失", async () => {
    // 后端删除分组时会把组内连接的 folder_id 置空
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") {
        return [session("s-plain", "plain-host", null)];
      }
      if (command === "list_session_load_issues") return [];
      if (command === "list_folders") return [];
      return undefined;
    });
    const wrapper = await mountList();
    expect(wrapper.text()).toContain("plain-host");
    expect(wrapper.find('[data-test="group-delete-folder-1"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("「默认」分组没有删除按钮（它是隐式分组）", async () => {
    const wrapper = await mountList();
    expect(wrapper.find('[data-test="group-delete-folder-1"]').exists()).toBe(true);
    // 默认组的 id 是 null → 不渲染删除按钮
    const deleteButtons = wrapper.findAll(".group-delete");
    expect(deleteButtons.length).toBe(2);
    wrapper.unmount();
  });

  it("可以新建分组", async () => {
    const wrapper = await mountList();
    await wrapper.get('[data-test="folder-name"]').setValue("新分组");
    await wrapper.get('[data-test="folder-create"]').trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith(
      "create_folder",
      expect.objectContaining({ name: "新分组" }),
    );
    wrapper.unmount();
  });

  it("删除分组前要确认，并说明组内连接不会被删除", async () => {
    const wrapper = await mountList();
    await wrapper.get('[data-test="group-delete-folder-1"]').trigger("click");
    await flushPromises();

    expect(confirmDialogMock).toHaveBeenCalledTimes(1);
    const spec = confirmDialogMock.mock.calls[0][0];
    expect(String(spec.message)).toContain("生产");
    expect(String(spec.detail)).toContain("不会被删除");
    wrapper.unmount();
  });

  it("用户取消时���删除分组", async () => {
    confirmDialogMock.mockResolvedValue(false);
    const wrapper = await mountList();
    await wrapper.get('[data-test="group-delete-folder-1"]').trigger("click");
    await flushPromises();

    expect(invoke).not.toHaveBeenCalledWith("delete_folder", expect.anything());
    wrapper.unmount();
  });
});

describe("folders store", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_folders") return folders;
      if (command === "create_folder") return "folder-3";
      return undefined;
    });
  });

  it("列出分组并按名称解析分组名", async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useFoldersStore();
    await store.refresh();

    expect(store.items).toHaveLength(2);
    expect(store.nameOf("folder-1")).toBe("生产");
    // 没有分组 / 指向已删除的分组 → 「默认」
    expect(store.nameOf(null)).toBe("默认");
    expect(store.nameOf("missing")).toBe("默认");
  });

  it("空名字不发给后端", async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useFoldersStore();
    const id = await store.create("   ");

    expect(id).toBeNull();
    expect(store.error).toContain("分组名不能为空");
    expect(invoke).not.toHaveBeenCalledWith("create_folder", expect.anything());
  });
});

// 空分组必须仍然显示：分组头挂着「删除」按钮，不显示就等于用户永远删不掉
// 自己刚建的空分组。
describe("SessionList 空分组可见性", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    confirmDialogMock.mockResolvedValue(true);
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") return [];
      if (command === "list_session_load_issues") return [];
      if (command === "list_folders") return [{ id: "folder-empty", name: "空的分组" }];
      return undefined;
    });
  });

  it("即使分组里没有连接也列出该分组（否则永远删不掉）", async () => {
    const wrapper = await mountList();
    expect(wrapper.text()).toContain("空的分组");
    expect(wrapper.find('[data-test="group-delete-folder-empty"]').exists()).toBe(true);
    wrapper.unmount();
  });
});
