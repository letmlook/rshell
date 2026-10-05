import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia } from "pinia";
import ElementPlus from "element-plus";
import SessionList from "../../src/components/SessionList.vue";
import { useSessionsStore } from "../../src/stores/sessions";
import type { SessionConfig } from "../../src/ipc/types";

// R3-14：右键菜单唯一的关闭途径是 openContextMenu 挂在 window 上的
// click/contextmenu 监听，但菜单容器带 @click.stop —— 点菜单项根本不冒泡。
// 五个 handler（连接 / 断开 / 打开SFTP / 打开标签 / 新开标签）触发动作后都没关
// 菜单，动作已执行而浮层继续盖住列表（对「打开SFTP / 打开标签」还盖住刚跳过去
// 的工作区），window 监听也滞留到下一次外部点击。

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const { confirmDialogMock } = vi.hoisted(() => ({ confirmDialogMock: vi.fn() }));
vi.mock("../../src/utils/dialog", () => ({ confirmDialog: confirmDialogMock }));

const session: SessionConfig = {
  id: "saved-session", name: "Production SSH", folder_id: null,
  host: "example.test", port: 22, protocol: "SSH",
  auth_method: { Password: { username: "alice", has_password: true } }, serial_config: null,
};

/** 兜底监听是 setTimeout(0) 挂的：等一次宏任务，flushPromises 只排微任务 */
const nextMacroTask = () => new Promise((resolve) => setTimeout(resolve, 1));

const wrappers: ReturnType<typeof mount>[] = [];
afterEach(() => {
  wrappers.splice(0).forEach((w) => w.unmount());
  document.body.innerHTML = "";
  // 只清调用记录：断开是 fire-and-forget，晚到的 refresh 也必须拿到数组而不是 undefined
  invoke.mockClear();
  confirmDialogMock.mockReset();
});

beforeEach(() => {
  invoke.mockReset();
  invoke.mockImplementation(async (command: string) => {
    if (command === "list_sessions") return [session];
    if (command === "list_session_load_issues") return [];
    return undefined;
  });
  // 删除走应用内确认窗：用户取消，断言只关心「菜单是否关闭」
  confirmDialogMock.mockResolvedValue(false);
});

function button(text: string) {
  return Array.from(document.body.querySelectorAll("button")).find((b) => b.textContent?.includes(text));
}

async function setup() {
  const pinia = createPinia();
  const wrapper = mount(SessionList, { attachTo: document.body, global: { plugins: [pinia, ElementPlus] } });
  wrappers.push(wrapper);
  // 列表数据由 store.refresh 拉取（与 SessionListSecurity.spec.ts 同款）
  await useSessionsStore(pinia).refresh();
  await flushPromises();
  return wrapper;
}

async function openMenu() {
  const wrapper = await setup();
  await wrapper.find(".leaf").trigger("contextmenu", { clientX: 30, clientY: 40 });
  await flushPromises();
  await nextMacroTask();
  expect(document.body.querySelector(".ctx-menu")).not.toBeNull();
  return wrapper;
}

describe("SessionList 右键菜单关闭", () => {
  // 菜单容器 @click.stop 阻断了冒泡，修复前这 8 项没有任何一项能让菜单消失
  const items = [
    "连接", "断开", "更新凭据", "打开 SFTP",
    "打开标签", "新开标签", "复制连接信息", "删除连接",
  ];

  it.each(items)("点击「%s」后菜单从文档中消失", async (item) => {
    await openMenu();
    const target = button(item);
    expect(target).toBeDefined();
    target!.click();
    await flushPromises();
    expect(document.body.querySelector(".ctx-menu")).toBeNull();
  });

  it("菜单项执行后同时摘除兜底的 window 监听", async () => {
    const addSpy = vi.spyOn(window, "addEventListener");
    const removeSpy = vi.spyOn(window, "removeEventListener");
    try {
      const wrapper = await setup();
      await wrapper.find(".leaf").trigger("contextmenu", { clientX: 30, clientY: 40 });
      await flushPromises();
      await nextMacroTask();
      // 记下 openContextMenu 挂上的那个 click 兜底监听
      const clickAdds = addSpy.mock.calls.filter(([type]) => type === "click");
      const menuListener = clickAdds[clickAdds.length - 1]?.[1];
      expect(menuListener).toBeTypeOf("function");

      button("打开 SFTP")!.click();
      await flushPromises();

      expect(removeSpy.mock.calls.some(([type, fn]) => type === "click" && fn === menuListener)).toBe(true);
      expect(removeSpy.mock.calls.some(([type, fn]) => type === "contextmenu" && fn === menuListener)).toBe(true);
    } finally {
      addSpy.mockRestore();
      removeSpy.mockRestore();
    }
  });

  it("重复右键不会叠加兜底监听：一次外部点击即可关闭", async () => {
    const wrapper = await setup();
    const leaf = wrapper.find(".leaf");
    await leaf.trigger("contextmenu", { clientX: 30, clientY: 40 });
    await flushPromises();
    await nextMacroTask();
    await leaf.trigger("contextmenu", { clientX: 50, clientY: 60 });
    await flushPromises();
    await nextMacroTask();

    expect(document.body.querySelectorAll(".ctx-menu")).toHaveLength(1);
    await leaf.trigger("click");
    await flushPromises();
    expect(document.body.querySelector(".ctx-menu")).toBeNull();
  });
});
