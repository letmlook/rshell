import { describe, expect, it, vi } from "vitest";
import { installContextMenuGuard } from "../../src/utils/contextMenu";

/** 守卫装在真实 DOM 节点上（应用里装的是 window） */
function host(): HTMLElement {
  const node = document.createElement("div");
  document.body.appendChild(node);
  return node;
}

/**
 * macOS 的 WKWebView 在右键时会弹出原生菜单（Reload / Inspect Element）。
 * 它会盖在应用自己的右键菜单上，且原生菜单的 Reload 会重载整个 WebView、
 * 丢掉前端状态 —— 所以必须全局吞掉 contextmenu 的默认行为。
 */
describe("全局右键菜单守卫", () => {
  it("吞掉普通元素的 contextmenu 默认行为（原生菜单不再弹出）", () => {
    const target = host();
    installContextMenuGuard(target);

    const event = new Event("contextmenu", { cancelable: true, bubbles: true });
    target.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
  });

  // 文本输入区要留原生菜单：主密码框、路径输入框这类地方用户靠右键粘贴
  it("保留文本输入框的原生菜单（右键粘贴可用）", () => {
    const target = host();
    installContextMenuGuard(target);

    const input = document.createElement("input");
    target.appendChild(input);
    const event = new Event("contextmenu", { cancelable: true, bubbles: true });
    input.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(false);
  });

  it("保留 textarea 的原生菜单", () => {
    const target = host();
    installContextMenuGuard(target);

    const area = document.createElement("textarea");
    target.appendChild(area);
    const event = new Event("contextmenu", { cancelable: true, bubbles: true });
    area.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(false);
  });

  // xterm 的 helper textarea 在终端画布里：终端自己管右键菜单（复制/粘贴），
  // 不能因为它是 textarea 就放行原生菜单
  it("终端的 helper textarea 不放行原生菜单", () => {
    const target = host();
    installContextMenuGuard(target);

    const helper = document.createElement("textarea");
    helper.classList.add("xterm-helper-textarea");
    target.appendChild(helper);
    const event = new Event("contextmenu", { cancelable: true, bubbles: true });
    helper.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
  });

  // contenteditable 的**子元素**上右键：目标自身没有 contenteditable 属性，
  // 要向上找，否则用户在可编辑区域里右键粘贴会被吞掉
  it("contenteditable 区域（含其子元素）保留原生菜单", () => {
    const target = host();
    installContextMenuGuard(target);

    const host2 = document.createElement("div");
    host2.setAttribute("contenteditable", "true");
    const child = document.createElement("span");
    host2.appendChild(child);
    target.appendChild(host2);

    const onChild = new Event("contextmenu", { cancelable: true, bubbles: true });
    child.dispatchEvent(onChild);
    expect(onChild.defaultPrevented).toBe(false);

    const onHost = new Event("contextmenu", { cancelable: true, bubbles: true });
    host2.dispatchEvent(onHost);
    expect(onHost.defaultPrevented).toBe(false);
  });

  // 守卫必须能卸载：组件卸载后监听残留会一直吞掉右键
  it("卸载后不再拦截", () => {
    const target = host();
    const remove = installContextMenuGuard(target);
    remove();

    const event = new Event("contextmenu", { cancelable: true, bubbles: true });
    target.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(false);
  });

  // capture 阶段拦截：早于组件自己的处理器，但不影响它们弹出自己的菜单
  it("在 capture 阶段拦截，但事件仍继续传播给组件自己的处理器", () => {
    const target = host();
    installContextMenuGuard(target);
    const componentHandler = vi.fn();
    target.addEventListener("contextmenu", componentHandler);

    target.dispatchEvent(new Event("contextmenu", { cancelable: true, bubbles: true }));

    expect(componentHandler).toHaveBeenCalledOnce();
  });
});
