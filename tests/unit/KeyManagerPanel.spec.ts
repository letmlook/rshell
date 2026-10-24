import { afterEach, describe, expect, it, vi } from "vitest";
import { DOMWrapper, flushPromises, mount } from "@vue/test-utils";
import ElementPlus from "element-plus";
import KeyManagerPanel from "../../src/components/KeyManagerPanel.vue";
import { deleteSshKey, exportPublicKey, importPrivateKey } from "../../src/ipc/client";
import type { AppEvent } from "../../src/ipc/types";

// 选文件走应用内路径选择器（utils/pathPicker），删除确认走 utils/dialog，
// 都不再经过 tauri-plugin-dialog 的原生窗口。
const { pickLocalPath, confirmDialog } = vi.hoisted(() => ({
  pickLocalPath: vi.fn(),
  confirmDialog: vi.fn(),
}));
vi.mock("../../src/utils/pathPicker", () => ({ pickLocalPath }));
vi.mock("../../src/utils/dialog", () => ({ confirmDialog }));

// 导出公钥的载荷走 PublicKeyExported 事件回来（命令本身返回 None），
// 这里把事件流做成可控的：测试可以主动投递事件。
const { subscribeAppEventsMock, handlers: eventHandlers } = vi.hoisted(() => {
  const handlers: Array<(event: AppEvent) => void> = [];
  return {
    subscribeAppEventsMock: vi.fn((handler: (event: AppEvent) => void) => {
      handlers.push(handler);
      return Promise.resolve(() => {
        const i = handlers.indexOf(handler);
        if (i >= 0) handlers.splice(i, 1);
      });
    }),
    handlers,
  };
});
vi.mock("../../src/ipc/events", () => ({ subscribeAppEvents: subscribeAppEventsMock }));

vi.mock("../../src/ipc/client", () => ({
  listKeys: vi.fn().mockResolvedValue([{ id: "key-1", name: "work", key_type: "ED25519", fingerprint: "SHA256:test", has_passphrase: false }]),
  generateSshKey: vi.fn(), importPrivateKey: vi.fn(), deleteSshKey: vi.fn(),
  exportPublicKey: vi.fn(),
}));

const deliverEvent = (event: AppEvent) => {
  for (const handler of [...eventHandlers]) handler(event);
};

afterEach(() => {
  vi.clearAllMocks();
  eventHandlers.length = 0;
  document.body.innerHTML = "";
});

type Wrapper = ReturnType<typeof mountPanel>;
const mountPanel = () =>
  mount(KeyManagerPanel, { attachTo: document.body, props: { embedded: true }, global: { plugins: [ElementPlus] } });
const importButton = (wrapper: Wrapper) =>
  wrapper.findAll("button").find(button => button.text() === "导入");

/**
 * 对话框开了 `append-to-body`：内容被 teleport 到 <body>，
 * `wrapper.find(".el-dialog …")` 找不到它，必须从 document.body 里取。
 *
 * 为什么必须 teleport：密钥面板挂在侧栏里，而侧栏是 `overflow: hidden`
 * （SidePanel.vue 的 `.side-panel`）——不 teleport 的 el-dialog 会被裁掉，
 * 真机实测点「导出公钥」后对话框完全不可见（DOM 里有、屏幕上没有）。
 */
function dialogInBody(selector: string): DOMWrapper<Element> | undefined {
  const el = document.body.querySelector(selector);
  return el ? new DOMWrapper(el) : undefined;
}

/** 对话框底部按钮（取消 / 确认） */
function dialogFooterButton(label: string): DOMWrapper<Element> | undefined {
  const buttons = [...document.body.querySelectorAll(".el-dialog__footer button")];
  const target = buttons.find((button) => button.textContent?.trim() === label);
  return target ? new DOMWrapper(target) : undefined;
}

async function openImportDialog(wrapper: Wrapper) {
  pickLocalPath.mockResolvedValue("/tmp/id_ed25519");
  await importButton(wrapper)!.trigger("click");
  await flushPromises();
}

describe("embedded key manager", () => {
  it("exposes the supported private key import action", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    const button = importButton(wrapper);
    expect(button, "import must remain reachable inside the side panel").toBeDefined();
    await openImportDialog(wrapper);
    const input = dialogInBody('.el-dialog input[type="password"]');
    expect(input, "passphrase dialog must appear after picking a file").toBeDefined();
    await input!.setValue("test-passphrase");
    await dialogFooterButton("导入")!.trigger("click");
    await flushPromises();
    expect(importPrivateKey).toHaveBeenCalledWith("/tmp/id_ed25519", "test-passphrase");
    wrapper.unmount();
  });

  // 导出公钥此前没有任何界面入口：后端命令与 PublicKeyExported 事件都在，
  // 但密钥面板没有按钮，用户拿不到自己的公钥（贴到服务器的 authorized_keys）。
  it("exports the public key and shows it for copying", async () => {
    const wrapper = mountPanel();
    await flushPromises();

    const exportButton = wrapper
      .findAll("button")
      .find((button) => button.text() === "导出公钥");
    expect(exportButton, "导出公钥入口必须可达").toBeDefined();
    await exportButton!.trigger("click");
    await flushPromises();

    expect(exportPublicKey).toHaveBeenCalledWith("key-1");

    // 公钥通过 PublicKeyExported 事件回来（命令本身返回 None）
    deliverEvent({
      PublicKeyExported: {
        key_id: "key-1",
        public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIexample test",
      },
    });
    await flushPromises();

    expect(dialogInBody(".export-body")!.text()).toContain("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5");
    wrapper.unmount();
  });

  it("keeps an empty passphrase as no-passphrase import and cancel inert", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    await openImportDialog(wrapper);
    await dialogFooterButton("取消")!.trigger("click");
    await flushPromises();
    expect(importPrivateKey).not.toHaveBeenCalled();
    await openImportDialog(wrapper);
    await dialogFooterButton("导入")!.trigger("click");
    await flushPromises();
    expect(importPrivateKey).toHaveBeenCalledWith("/tmp/id_ed25519", null);
    wrapper.unmount();
  });

  it("requires confirmation before deleting a key", async () => {
    const wrapper = mountPanel();
    await flushPromises();
    const button = wrapper.findAll("button").find(button => button.text() === "删除")!;
    confirmDialog.mockResolvedValue(false);
    await button.trigger("click");
    await flushPromises();
    expect(deleteSshKey).not.toHaveBeenCalled();
    confirmDialog.mockResolvedValue(true);
    await button.trigger("click");
    await flushPromises();
    expect(deleteSshKey).toHaveBeenCalledWith("key-1");
    wrapper.unmount();
  });

  // 侧栏只有 200~360px：表格把没写宽度的「名称」「指纹」两列宽度设成 0
  //（Element Plus 在溢出时的行为），真机上名称整列看不见、指纹换行八行，
  // 最右侧的「导出公钥 / 删除」还被裁掉。密钥列表必须用卡片渲染。
  it("密钥列表用卡片渲染，名称与指纹都要看得见", async () => {
    const wrapper = mountPanel();
    await flushPromises();

    expect(wrapper.find(".el-table").exists()).toBe(false);
    const card = wrapper.get('[data-test="key-card-key-1"]');
    expect(card.text()).toContain("work");
    expect(card.text()).toContain("SHA256:test");
    expect(card.text()).toContain("ED25519");
    const actions = card.findAll("button").map((button) => button.text());
    expect(actions).toContain("导出公钥");
    expect(actions).toContain("删除");
    wrapper.unmount();
  });
});
