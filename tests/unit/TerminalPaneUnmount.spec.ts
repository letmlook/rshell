// R3-05 / R3-06 回归测试
//
// R3-06：`TerminalPane` 的 `onMounted` 是 async 的，内含 `ensurePty` /
// `attachTerminal` 两次 await。用户在往返窗口内关闭标签时，`onBeforeUnmount`
// 把 `term` 置 null，await 续体恢复后执行 `term.onData(...)` → 对 null 取属性
// 抛 TypeError（unhandled rejection），并且其后的 window / document 监听与
// ResizeObserver 全部不再注册。修复靠 `unmounted` 闩锁。
//
// R3-05：标签在握手中被打开时 `open_terminal` 必然 NotFound，而此前没有任何
// watcher 会在握手完成后补一次「确保 pty + 附加通道」，标签就此永久空白。

import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { useSessionsStore } from "../../src/stores/sessions";
import TerminalPane from "../../src/components/TerminalPane.vue";

const terminalRuntime = vi.hoisted(() => ({ input: (_data: string) => {}, key: (_event: KeyboardEvent): boolean => true, options: { disableStdin: false } }));

const { openTerminalMock, closeTerminalMock, sendInputMock, resizeTerminalMock, invokeMock } =
  vi.hoisted(() => ({
    openTerminalMock: vi.fn<() => Promise<void>>(),
    closeTerminalMock: vi.fn().mockResolvedValue(undefined),
    sendInputMock: vi.fn().mockResolvedValue(undefined),
    resizeTerminalMock: vi.fn().mockResolvedValue(undefined),
    invokeMock: vi.fn().mockResolvedValue(undefined),
  }));

// xterm 替身：只提供 TerminalPane 真实用到的方法，缺一个就会在 mounted 抛错。
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    selection = "";
    options: { theme: unknown; disableStdin: boolean } = { theme: undefined, disableStdin: false };
    constructor() { terminalRuntime.options = this.options; }
    loadAddon() {}
    open() {}
    onData(callback: (data: string) => void) { terminalRuntime.input = callback; terminalRuntime.options = this.options; }
    dispose() {}
    write() {}
    attachCustomKeyEventHandler(callback: (event: KeyboardEvent) => boolean) { terminalRuntime.key = callback; }
    onSelectionChange() {}
    getSelection() {
      return this.selection;
    }
    clear() {
      this.selection = "";
    }
    paste(text: string) { terminalRuntime.input(text); }
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));
vi.mock("@xterm/addon-search", () => ({
  SearchAddon: class {
    clearDecorations() {}
    findNext() {
      return false;
    }
    findPrevious() {
      return false;
    }
  },
}));
vi.mock("@xterm/addon-webgl", () => ({ WebglAddon: class { onContextLoss() {} } }));
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage?: (d: number[]) => void;
  },
  invoke: invokeMock,
}));
vi.mock("../../src/ipc/client", () => ({
  openTerminal: openTerminalMock,
  closeTerminal: closeTerminalMock,
  sendInput: sendInputMock,
  resizeTerminal: resizeTerminalMock,
  ipcErrorMessage: (e: unknown) => String(e),
  listThemes: vi.fn(),
  setAppTheme: vi.fn(),
  setTerminalColorScheme: vi.fn(),
}));
vi.mock("../../src/ipc/events", () => ({ subscribeAppEvents: vi.fn() }));

/** 附加标签（带独立 terminalId）才会走 open_terminal */
const EXTRA_TAB = { sessionId: "session-1", terminalId: "terminal-9" };

/**
 * 取 Node 的 process 以订阅 unhandledRejection。
 *
 * 走 globalThis 取而不是直接写 `process`：本仓库的 tsconfig 没有把
 * `@types/node` 的全局带进测试上下文，直接用 `process` 会报 TS2591。
 */
type UnhandledEmitter = {
  on(ev: "unhandledRejection", cb: (reason: unknown) => void): void;
  off(ev: "unhandledRejection", cb: (reason: unknown) => void): void;
};
const nodeProcess = (globalThis as unknown as { process: UnhandledEmitter }).process;

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  setActivePinia(createPinia());
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
  openTerminalMock.mockReset();
});

describe("R3-06 卸载与在途 async 竞态", () => {
  it("在 open_terminal 挂起期间卸载，不得抛 TypeError", async () => {
    // 让 open_terminal 一直挂起，模拟 IPC 往返窗口
    let release: () => void = () => {};
    openTerminalMock.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          release = resolve;
        }),
    );

    const unhandled: unknown[] = [];
    const onUnhandled = (reason: unknown) => unhandled.push(reason);
    nodeProcess.on("unhandledRejection", onUnhandled);

    try {
      const wrapper = mount(TerminalPane, {
        props: { ...EXTRA_TAB, connectionState: "connected" },
      });
      // 让 onMounted 跑到 ensurePty 的 await 上并挂起
      await flushPromises();
      expect(openTerminalMock).toHaveBeenCalledTimes(1);

      // 用户在 IPC 往返窗口内关闭标签
      wrapper.unmount();

      // 放行挂起的 open_terminal，让 await 续体恢复
      release();
      await flushPromises();
      await flushPromises();

      const typeErrors = unhandled.filter(
        (r) =>
          r instanceof TypeError &&
          /onData|onSelectionChange|of null/.test(String((r as Error).message)),
      );
      expect(
        typeErrors.map((e) => String((e as Error).message)),
        "卸载后 async onMounted 续体不得再访问已置空的 term（R3-06）",
      ).toEqual([]);
    } finally {
      nodeProcess.off("unhandledRejection", onUnhandled);
    }
  });

  it("卸载后 onMounted 提前返回，不会在已销毁的面板上继续挂 window 监听", async () => {
    // 用一个「永不 resolve」的 open_terminal 制造最宽的竞态窗口
    openTerminalMock.mockImplementation(() => new Promise<void>(() => {}));

    const addSpy = vi.spyOn(window, "addEventListener");
    const wrapper = mount(TerminalPane, {
      props: { ...EXTRA_TAB, connectionState: "connected" },
    });
    await flushPromises();
    wrapper.unmount();

    const afterUnmount = addSpy.mock.calls.filter(
      // `rshell:terminal-action` 注册在 ensurePty/attachTerminal 两次 await
      // **之后**；`rshell:terminal-theme` 在 await 之前就已注册，不在此列。
      ([type]) => type === "rshell:terminal-action",
    );
    expect(
      afterUnmount,
      "卸载之后不得再注册 await 之后的 window 监听（R3-06）",
    ).toHaveLength(0);
    addSpy.mockRestore();
  });
});

describe("R3-05 握手完成后自动补挂", () => {
  it("open_terminal 失败后，连接变为 connected 时自动重试", async () => {
    // 第一次 open 失败 → attachError 置位（模拟「握手中打开标签」）
    openTerminalMock.mockRejectedValueOnce(new Error("not found"));
    const wrapper = mount(TerminalPane, {
      props: { ...EXTRA_TAB, connectionState: "connecting" },
    });
    await flushPromises();
    expect(wrapper.find(".terminal-error-bar").exists()).toBe(true);
    expect(openTerminalMock).toHaveBeenCalledTimes(1);

    // 握手完成：后续 open 成功，watcher 应自动补一次
    openTerminalMock.mockResolvedValue(undefined);
    await wrapper.setProps({ connectionState: "connected" });
    await flushPromises();
    await flushPromises();

    expect(
      openTerminalMock.mock.calls.length,
      "连接变为 connected 后必须自动补一次 ensurePty（R3-05）",
    ).toBeGreaterThanOrEqual(2);
    wrapper.unmount();
  });
});


describe("SSH uncertain input recovery", () => {
  it("one uncertain input stops keyboard, backspace and paste until real reconnect finishes", async () => {
    const wrapper = mount(TerminalPane, { props: { sessionId: "session-1", connectionState: "connected" } });
    await flushPromises();
    const store = useSessionsStore();
    const disconnect = vi.spyOn(store, "disconnect").mockResolvedValue(undefined);
    let finish: () => void = () => {};
    const connect = vi.spyOn(store, "connect").mockImplementation(() => new Promise<void>(resolve => { finish = resolve; }));
    sendInputMock.mockRejectedValueOnce({ kind: "terminal_recovery_required", message: "input outcome uncertain" });
    terminalRuntime.input("first");
    await flushPromises();
    expect(wrapper.find('[data-test="term-recovery"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("不确定");
    expect(terminalRuntime.options.disableStdin).toBe(true);
    terminalRuntime.input("later");
    terminalRuntime.key(new KeyboardEvent("keydown", { key: "Backspace" }));
    vi.stubGlobal("navigator", { clipboard: { readText: vi.fn().mockResolvedValue("clipboard") } });
    window.dispatchEvent(new CustomEvent("rshell:terminal-action", { detail: { sessionId: "session-1", action: "paste" } }));
    await flushPromises();
    expect(sendInputMock).toHaveBeenCalledTimes(1);
    await wrapper.find('[data-test="term-reconnect"]').trigger("click");
    await flushPromises();
    expect(disconnect).toHaveBeenCalledWith("session-1");
    expect(connect).toHaveBeenCalledWith("session-1");
    expect(terminalRuntime.options.disableStdin).toBe(true);
    finish();
    await flushPromises();
    expect(wrapper.find('[data-test="term-recovery"]').exists()).toBe(false);
    expect(terminalRuntime.options.disableStdin).toBe(false);
    terminalRuntime.input("new input");
    await flushPromises();
    expect(sendInputMock).toHaveBeenCalledTimes(2);
    expect(new TextDecoder().decode(sendInputMock.mock.calls[1][1])).toBe("new input");
    wrapper.unmount();
  });

  it("failed reconnect and late successful IO cannot clear uncertain recovery", async () => {
    const wrapper = mount(TerminalPane, { props: { sessionId: "session-1", connectionState: "connected" } });
    await flushPromises();
    const store = useSessionsStore();
    vi.spyOn(store, "disconnect").mockResolvedValue(undefined);
    vi.spyOn(store, "connect").mockRejectedValue(new Error("network unavailable"));
    let lateSuccess: () => void = () => {};
    sendInputMock.mockImplementationOnce(() => new Promise<void>(resolve => { lateSuccess = resolve; }));
    sendInputMock.mockRejectedValueOnce({ kind: "terminal_recovery_required", message: "input outcome uncertain" });
    terminalRuntime.input("earlier pending");
    terminalRuntime.input("first");
    await flushPromises();
    lateSuccess();
    await flushPromises();
    expect(wrapper.find('[data-test="term-recovery"]').exists()).toBe(true);
    await wrapper.find('[data-test="term-reconnect"]').trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("network unavailable");
    expect(wrapper.find('[data-test="term-recovery"]').exists()).toBe(true);
    expect(terminalRuntime.options.disableStdin).toBe(true);
    wrapper.unmount();
  });
  it("a sibling label keeps input disabled until its replacement PTY and attach succeed", async () => {
    openTerminalMock.mockRejectedValueOnce(new Error("not connected"));
    const wrapper = mount(TerminalPane, { props: { ...EXTRA_TAB, connectionState: "connecting" } });
    await flushPromises();
    let finish: () => void = () => {};
    openTerminalMock.mockImplementation(() => new Promise<void>(resolve => { finish = resolve; }));
    await wrapper.setProps({ connectionState: "connected" });
    await flushPromises();
    expect(terminalRuntime.options.disableStdin).toBe(true);
    finish();
    await flushPromises();
    expect(terminalRuntime.options.disableStdin).toBe(false);
    wrapper.unmount();
  });

});
