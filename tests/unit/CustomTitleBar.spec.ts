/**
 * CustomTitleBar 拖动区契约 —— 锁住 macOS 行为。
 *
 * jsdom 不会模拟 `-webkit-app-region`,但合约要求:
 * - 根 <header> 自身 **不** 带 drag 属性,避免与子节点嵌套冲突。
 * - 中心可拖动区带 `data-tauri-drag-region` 与 `-webkit-app-region: drag`。
 * - 左侧(logo + 菜单)与右侧(窗口按钮)必须带 `-webkit-app-region: no-drag`,
 *   防止点击穿透或菜单被吞。
 */
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import capability from "../../src-tauri/capabilities/default.json";

// The frontend does not depend on @types/node. Load the optional native-test
// boundary through Vitest with only the signatures this harness consumes.
const { existsSync, readdirSync, readFileSync } = await vi.importActual<{
  existsSync(path: string): boolean;
  readdirSync(path: string): string[];
  readFileSync(path: string, encoding: "utf8"): string;
}>("node:fs");
const { homedir } = await vi.importActual<{ homedir(): string }>("node:os");
const { join } = await vi.importActual<{ join(...paths: string[]): string }>("node:path");
const { env } = await vi.importActual<{ env: Record<string, string | undefined> }>("node:process");

const nativeState = vi.hoisted(() => ({ maximized: false }));

// 标题栏在 mount 时会调用 `getCurrentWindow().isMaximized()` 之类的 API,
// 这里在导入前先 stub 掉,避免 jsdom 报错。
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isMaximized: () => Promise.resolve(nativeState.maximized),
    onResized: () => Promise.resolve(() => undefined),
    minimize: () => Promise.resolve(),
    maximize: () => { nativeState.maximized = true; return Promise.resolve(); },
    unmaximize: () => { nativeState.maximized = false; return Promise.resolve(); },
    close: () => Promise.resolve(),
  }),
}));

// el-dropdown 在测试中不需要真实行为,挂一个简单 stub 防止控制台噪音。
vi.mock("element-plus", () => ({
  default: {
    install() {
      /* no-op in tests */
    },
  },
}));
// 显式 stub 模板里出现的 Element Plus 组件,避免"Failed to resolve component" 警告。
const elStub = (name: string) => ({
  name,
  props: ["trigger"],
  emits: ["click"],
  template: "<div class='el-stub' data-name='" + name + "'><slot /></div>",
});
const elDropdownItemStub = {
  name: "el-dropdown-item",
  props: ["disabled"],
  emits: ["click"],
  template: "<button class='el-stub-dropdown-item'><slot /></button>",
};
const elDropdownMenuStub = {
  name: "el-dropdown-menu",
  template: "<div class='el-stub-dropdown-menu'><slot /></div>",
};

import CustomTitleBar from "../../src/components/CustomTitleBar.vue";

beforeEach(() => {
  setActivePinia(createPinia());
  nativeState.maximized = false;
});

const globalStubs = {
  "el-dropdown": elStub("el-dropdown"),
  "el-dropdown-item": elDropdownItemStub,
  "el-dropdown-menu": elDropdownMenuStub,
};

function mountTitleBar() {
  return mount(CustomTitleBar, { global: { stubs: globalStubs } });
}

describe("CustomTitleBar drag contract (macOS)", () => {
  it("grants the native drag command to the main window", () => {
    expect(capability.windows).toContain("main");
    expect(capability.permissions).toContain("core:window:allow-start-dragging");
  });
  it("root header does not declare drag itself", () => {
    const wrapper = mountTitleBar();
    const header = wrapper.element as HTMLElement;
    expect(header.hasAttribute("data-tauri-drag-region")).toBe(false);
    expect(header.classList.contains("is-draggable")).toBe(false);
  });

  it("center region is the only drag-enabled area", () => {
    const wrapper = mountTitleBar();
    const center = wrapper.find(".titlebar-center").element as HTMLElement;
    expect(center.getAttribute("data-tauri-drag-region")).toBe("");
    expect(center.classList.contains("is-draggable")).toBe(true);
  });

  it("left and right groups are excluded from drag", () => {
    const wrapper = mountTitleBar();
    const left = wrapper.find(".titlebar-left").element as HTMLElement;
    const right = wrapper.find(".titlebar-right").element as HTMLElement;
    expect(left.getAttribute("data-tauri-drag-region")).toBe("false");
    expect(left.classList.contains("is-draggable")).toBe(false);
    expect(right.getAttribute("data-tauri-drag-region")).toBe("false");
    expect(right.classList.contains("is-draggable")).toBe(false);
  });

});

// Run the exact drag script from the Cargo.lock-pinned Tauri dependency. These
// integration cases require `cargo fetch` / a native build; frontend-only
// checkouts retain the component contract tests above.
const registry = join(env.CARGO_HOME ?? join(homedir(), ".cargo"), "registry", "src");
const tauriVersion = readFileSync("src-tauri/Cargo.lock", "utf8").match(/name = "tauri"\nversion = "([^"]+)"/)?.[1];
const tauriRoot = existsSync(registry) ? readdirSync(registry)
  .map(source => join(registry, source, `tauri-${tauriVersion}`))
  .find(path => existsSync(join(path, "src/window/scripts/drag.js"))) : undefined;
const manifestPath = "src-tauri/gen/schemas/acl-manifests.json";
const nativeHarnessAvailable = !!tauriRoot && existsSync(manifestPath);
const cleanups: Array<() => void> = [];
afterEach(() => { while (cleanups.length) cleanups.pop()!(); });

function installNativeDrag() {
  const source = readFileSync(join(tauriRoot!, "src/window/scripts/drag.js"), "utf8");
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"))["core:window"];
  const permissions = capability.permissions.flatMap(permission => permission === "core:window:default"
    ? manifest.default_permission.permissions as string[]
    : permission.startsWith("core:window:") ? [permission.slice("core:window:".length)] : []);
  const allowed = new Set<string>(permissions.flatMap(permission => manifest.permissions[permission]?.commands.allow ?? []));
  const commands: string[] = [];
  const denied: string[] = [];
  const invoke = async (command: string) => {
    const operation = command.split("|")[1];
    if (!allowed.has(operation)) { denied.push(operation); return; }
    commands.push(operation);
    if (operation === "internal_toggle_maximize") nativeState.maximized = !nativeState.maximized;
  };
  const documentBoundary = { addEventListener(name: string, handler: EventListener) {
    document.addEventListener(name, handler);
    cleanups.push(() => document.removeEventListener(name, handler));
  } };
  new Function("document", "window", "HTMLElement", source.replace("__TEMPLATE_os_name__", '"macos"'))(
    documentBoundary, { __TAURI_INTERNALS__: { invoke } }, HTMLElement,
  );
  return { commands, denied };
}

function mountNativeTitleBar() {
  const wrapper = mount(CustomTitleBar, { attachTo: document.body, global: { stubs: globalStubs } });
  cleanups.push(() => wrapper.unmount());
  return wrapper;
}

describe.skipIf(!nativeHarnessAvailable)("titlebar with bundled Tauri macOS drag handling", () => {
  it("allows native dragging from center while excluding menus and window controls", async () => {
    const { commands, denied } = installNativeDrag();
    const wrapper = mountNativeTitleBar();
    for (const selector of [".titlebar-center", ".titlebar-left", ".titlebar-right"]) {
      wrapper.find(selector).element.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, detail: 1 }));
    }
    expect(denied).toEqual([]);
    expect(commands).toEqual(["start_dragging"]);
  });

  it("maximizes once for a stationary center double click", async () => {
    const { commands } = installNativeDrag();
    const center = mountNativeTitleBar().find(".titlebar-center");
    center.element.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, detail: 2, clientX: 20, clientY: 10 }));
    center.element.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0, detail: 2, clientX: 20, clientY: 10 }));
    center.element.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, button: 0, detail: 2 }));
    await flushPromises();
    expect(commands).toEqual(["internal_toggle_maximize"]);
    expect(nativeState.maximized).toBe(true);
  });

  it("honors macOS movement cancellation instead of toggling on a later dblclick", async () => {
    const { commands } = installNativeDrag();
    const center = mountNativeTitleBar().find(".titlebar-center");
    center.element.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, detail: 2, clientX: 20, clientY: 10 }));
    center.element.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0, detail: 2, clientX: 30, clientY: 10 }));
    center.element.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, button: 0, detail: 2 }));
    await flushPromises();
    expect(commands).toEqual([]);
    expect(nativeState.maximized).toBe(false);
  });
});
