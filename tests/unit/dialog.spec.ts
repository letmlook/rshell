import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import {
  DIALOG_CANCEL,
  active,
  closeDialog,
  confirmDialog,
  openDialog,
  promptDialog,
  resetDialogs,
  setActiveDialogInput,
  submitActiveDialog,
} from "../../src/utils/dialog";
import DialogHost from "../../src/components/dialogs/DialogHost.vue";

/**
 * 这些窗体替代的是 ElMessageBox 与 tauri-plugin-dialog 的原生窗口：
 * 「确认必须先出现、取消不执行、输入非法不关窗」是删除类操作的安全底线。
 */
describe("dialog 服务", () => {
  beforeEach(() => resetDialogs());

  it("确认窗返回 true，且请求里带上危险标记与原文", async () => {
    const promise = confirmDialog({
      title: "删除会话",
      message: "删除 dev-ubuntu？",
      confirmText: "删除",
      danger: true,
    });
    expect(active.value?.title).toBe("删除会话");
    const confirmButton = active.value?.buttons.find((b) => b.variant === "danger");
    expect(confirmButton?.label).toBe("删除");
    closeDialog(confirmButton!.value);

    await expect(promise).resolves.toBe(true);
  });

  it("取消/关闭都返回 false，调用点无需 try/catch", async () => {
    const cancelled = confirmDialog({ title: "t", message: "m" });
    closeDialog(DIALOG_CANCEL);
    await expect(cancelled).resolves.toBe(false);

    const dismissed = confirmDialog({ title: "t", message: "m" });
    closeDialog(null);
    await expect(dismissed).resolves.toBe(false);
  });

  it("输入窗确认时回填输入内容", async () => {
    const promise = promptDialog({ title: "新建文件夹", defaultValue: "logs" });
    setActiveDialogInput("logs-2026");
    submitActiveDialog();

    await expect(promise).resolves.toBe("logs-2026");
  });

  it("输入非法时不关窗，就地显示原因", async () => {
    const promise = promptDialog({
      title: "重命名",
      validate: (value) => (value.includes("/") ? "文件名不能包含 \\ /" : null),
    });
    setActiveDialogInput("a/b");
    expect(submitActiveDialog()).toBe(false);
    await flushPromises();
    expect(active.value).not.toBeNull();
    expect(active.value?.inputError).toContain("\\ /");

    // 修正后同一次调用链即可提交，前一个 Promise 不会被提前结算
    setActiveDialogInput("ab");
    expect(submitActiveDialog()).toBe(true);
    await expect(promise).resolves.toBe("ab");
  });

  it("取消输入窗返回 null", async () => {
    const promise = promptDialog({ title: "t" });
    closeDialog(DIALOG_CANCEL);
    await expect(promise).resolves.toBeNull();
  });

  it("并发请求排队：解决第一个后第二个才成为当前窗", async () => {
    const first = openDialog({ title: "1", buttons: [{ label: "ok", value: "ok" }] });
    const second = openDialog({ title: "2", buttons: [{ label: "ok", value: "ok" }] });
    expect(active.value?.title).toBe("1");

    closeDialog("ok");
    await expect(first).resolves.toBe("ok");
    expect(active.value?.title).toBe("2");

    closeDialog("ok");
    await expect(second).resolves.toBe("ok");
    expect(active.value).toBeNull();
  });
});

describe("DialogHost", () => {
  beforeEach(() => resetDialogs());

  function mountHost() {
    // Teleport 到 body 后内容不在 wrapper 树内，测试里 stub 掉 teleport
    // 让标记留在组件树中，才能直接用 wrapper.get 触发交互
    return mount(DialogHost, { attachTo: document.body, global: { stubs: { teleport: true } } });
  }

  it("渲染标题、消息与按钮，取消按钮不结算为确认", async () => {
    const promise = confirmDialog({ title: "删除远程文件", message: "确定删除？", confirmText: "删除" });
    const wrapper = mountHost();
    await flushPromises();

    expect(wrapper.get('[data-test="dlg-panel"]').text()).toContain("删除远程文件");
    expect(wrapper.get('[data-test="dlg-panel"]').text()).toContain("确定删除？");

    await wrapper.get('[data-test="dlg-btn-cancel"]').trigger("click");
    await expect(promise).resolves.toBe(false);
    wrapper.unmount();
  });

  it("点遮罩关闭等价于取消", async () => {
    const promise = confirmDialog({ title: "t", message: "m" });
    const wrapper = mountHost();
    await flushPromises();

    await wrapper.get('[data-test="dlg-backdrop"]').trigger("mousedown");
    await expect(promise).resolves.toBe(false);
    expect(wrapper.find('[data-test="dlg-panel"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("dismissible=false 的窗体点遮罩不关闭（强确认场景）", async () => {
    const promise = openDialog({
      title: "将暴露给局域网",
      message: "确认继续？",
      dismissible: false,
      buttons: [{ label: "确认", value: "ok" }],
    });
    const wrapper = mountHost();
    await flushPromises();

    await wrapper.get('[data-test="dlg-backdrop"]').trigger("mousedown");
    await flushPromises();
    expect(wrapper.find('[data-test="dlg-panel"]').exists()).toBe(true);

    await wrapper.get('[data-test="dlg-btn-ok"]').trigger("click");
    await expect(promise).resolves.toBe("ok");
    wrapper.unmount();
  });

  it("输入框回车提交并显示校验错误", async () => {
    const promise = promptDialog({ title: "重命名", validate: (v) => (v ? null : "不能为空") });
    const wrapper = mountHost();
    await flushPromises();

    const input = wrapper.get('[data-test="dlg-input"]');
    await input.trigger("keydown", { key: "Enter" });
    await flushPromises();
    expect(wrapper.get('[data-test="dlg-input-error"]').text()).toContain("不能为空");

    await input.setValue("a.log");
    await wrapper.get('[data-test="dlg-input"]').trigger("keydown", { key: "Enter" });
    await expect(promise).resolves.toBe("a.log");
    wrapper.unmount();
  });

  it("Esc 关闭窗体", async () => {
    const promise = confirmDialog({ title: "t", message: "m" });
    const wrapper = mountHost();
    await flushPromises();

    await wrapper.get('[data-test="dlg-backdrop"]').trigger("keydown", { key: "Escape" });
    await expect(promise).resolves.toBe(false);
    wrapper.unmount();
  });
});

describe("dialog 服务无残留", () => {
  it("resetDialogs 清空队列，避免用例互相污染", async () => {
    const pending = confirmDialog({ title: "t", message: "m" });
    resetDialogs();
    expect(active.value).toBeNull();
    // 被 reset 丢弃的 Promise 永不结算：用哨兵避免未处理拒绝静默通过
    const settled = vi.fn();
    void pending.then(settled, settled);
    await flushPromises();
    expect(settled).not.toHaveBeenCalled();
  });
});
