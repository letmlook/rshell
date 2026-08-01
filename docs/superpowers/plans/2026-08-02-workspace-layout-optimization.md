# Workspace Layout Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the duplicated left navigation with a top horizontal context toolbar and one resizable contextual sidebar while preserving terminal, SFTP, session, and transfer state.

**Architecture:** Keep `App.vue` as the owner of workspace and layout state. Add a small pure layout utility for width clamping and keyboard adjustments, make `WorkspaceToolbar` emit navigation intent, and make `SidePanel` render exactly one contextual child plus its own nested tabs. The main workspace remains independent of sidebar visibility and width; no Rust or IPC contract changes are needed.

**Tech Stack:** Vue 3 `<script setup>`, TypeScript strict mode, Pinia, Element Plus, dockview-vue, Vitest + jsdom + `@vue/test-utils`, CSS custom properties in `tokens.css`.

---

## File map and boundaries

The implementation changes are intentionally limited to the frontend layout layer:

- Create `src/utils/workspaceLayout.ts`: pure constants and functions for sidebar width, responsive maximum width, keyboard steps, and clamping. It must not import Vue, Pinia, Tauri, or browser globals.
- Create `tests/unit/workspaceLayout.spec.ts`: unit tests for every exported layout helper, including narrow viewport behavior and invalid numeric input.
- Modify `src/components/WorkspaceToolbar.vue`: add top-level context buttons (`会话`, `文件`, `密钥`, `工具`, `设置`) and a sidebar toggle; emit intent events rather than touching stores.
- Modify `src/components/SidePanel.vue`: become the only sidebar container, add nested tabs for tools/settings, add the keyboard-accessible resize separator, and render one contextual child at a time.
- Modify `src/components/SessionList.vue`, `src/components/KeyManagerPanel.vue`, `src/components/QuickCommandPanel.vue`, `src/components/TriggerEditor.vue`, `src/components/TunnelPanel.vue`, `src/components/ThemePanel.vue`, and `src/components/PluginPanel.vue`: accept an `embedded` prop and suppress their duplicate outer headers when embedded.
- Modify `src/App.vue`: remove `ActivityBar`, own sidebar width and nested-panel state, wire toolbar events, and preserve both workspace subtrees while switching.
- Modify `src/styles/tokens.css`: set sidebar defaults to 280/200/360px and add separator geometry tokens.
- Modify `src/styles/global.css` only if shared focus or responsive rules cannot remain scoped to the changed components.
- Create `tests/unit/WorkspaceToolbar.spec.ts`: mount the toolbar with stubbed SVG/Element Plus-independent props and verify emitted navigation intent plus ARIA state.
- Create `tests/unit/SidePanel.spec.ts`: mount the panel with child stubs and verify mutually exclusive rendering, width separator semantics, and keyboard events.
- Create `tests/unit/AppLayout.spec.ts`: mount `App.vue` with Tauri/Pinia/child-component stubs and verify top-level composition and layout state transitions without opening a real Tauri window.

Do not modify `src-tauri/`, `src/ipc/types.ts`, `src/ipc/client.ts`, or backend crates.

---

### Task 1: Add tested sidebar layout primitives

**Files:**
- Create: `src/utils/workspaceLayout.ts`
- Test: `tests/unit/workspaceLayout.spec.ts`

- [ ] **Step 1: Write the failing unit tests**

Create tests that define the public API before implementation:

```ts
import { describe, expect, it } from "vitest";
import {
  DEFAULT_SIDEBAR_WIDTH,
  MIN_SIDEBAR_WIDTH,
  MAX_SIDEBAR_WIDTH,
  SIDEBAR_RESIZE_STEP,
  clampSidebarWidth,
  maxSidebarWidthForViewport,
  resizeSidebarWithKey,
} from "../../src/utils/workspaceLayout";

describe("workspace layout constants", () => {
  it("uses the confirmed 280/200/360 geometry", () => {
    expect(DEFAULT_SIDEBAR_WIDTH).toBe(280);
    expect(MIN_SIDEBAR_WIDTH).toBe(200);
    expect(MAX_SIDEBAR_WIDTH).toBe(360);
    expect(SIDEBAR_RESIZE_STEP).toBe(8);
  });
});

describe("clampSidebarWidth", () => {
  it("clamps values to the normal range", () => {
    expect(clampSidebarWidth(100)).toBe(200);
    expect(clampSidebarWidth(280)).toBe(280);
    expect(clampSidebarWidth(500)).toBe(360);
  });

  it("falls back to the default for non-finite input", () => {
    expect(clampSidebarWidth(Number.NaN)).toBe(280);
    expect(clampSidebarWidth(Number.POSITIVE_INFINITY)).toBe(280);
    expect(clampSidebarWidth(Number.NEGATIVE_INFINITY)).toBe(280);
  });

  it("honors a valid dynamic upper bound without violating the minimum", () => {
    expect(clampSidebarWidth(340, 300)).toBe(300);
    expect(clampSidebarWidth(340, 120)).toBe(200);
  });
});

describe("maxSidebarWidthForViewport", () => {
  it("preserves the 360px main-area target when possible", () => {
    expect(maxSidebarWidthForViewport(1280)).toBe(360);
    expect(maxSidebarWidthForViewport(700)).toBe(340);
  });

  it("returns the minimum when the viewport cannot fit both preferred regions", () => {
    expect(maxSidebarWidthForViewport(500)).toBe(200);
    expect(maxSidebarWidthForViewport(Number.NaN)).toBe(360);
  });
});

describe("resizeSidebarWithKey", () => {
  it("moves by 8px and clamps", () => {
    expect(resizeSidebarWithKey(280, "ArrowRight")).toBe(288);
    expect(resizeSidebarWithKey(280, "ArrowLeft")).toBe(272);
    expect(resizeSidebarWithKey(200, "ArrowLeft")).toBe(200);
    expect(resizeSidebarWithKey(360, "ArrowRight")).toBe(360);
  });

  it("supports Home and End and ignores unrelated keys", () => {
    expect(resizeSidebarWithKey(280, "Home")).toBe(200);
    expect(resizeSidebarWithKey(280, "End")).toBe(360);
    expect(resizeSidebarWithKey(280, "Enter")).toBeNull();
  });
});
```

- [ ] **Step 2: Run the focused test and verify it fails**

Run:

```bash
npm test -- tests/unit/workspaceLayout.spec.ts
```

Expected: Vitest fails because `src/utils/workspaceLayout.ts` does not exist or does not export the named functions.

- [ ] **Step 3: Implement the pure helpers**

Create `src/utils/workspaceLayout.ts` with no browser dependencies:

```ts
export const DEFAULT_SIDEBAR_WIDTH = 280;
export const MIN_SIDEBAR_WIDTH = 200;
export const MAX_SIDEBAR_WIDTH = 360;
export const SIDEBAR_RESIZE_STEP = 8;
export const MAIN_AREA_TARGET_WIDTH = 360;

function finiteOr(value: number, fallback: number): number {
  return Number.isFinite(value) ? value : fallback;
}

export function clampSidebarWidth(
  value: number,
  upperBound = MAX_SIDEBAR_WIDTH,
): number {
  const safeUpper = Math.max(MIN_SIDEBAR_WIDTH, finiteOr(upperBound, MAX_SIDEBAR_WIDTH));
  const safeValue = finiteOr(value, DEFAULT_SIDEBAR_WIDTH);
  return Math.min(Math.max(safeValue, MIN_SIDEBAR_WIDTH), safeUpper);
}

export function maxSidebarWidthForViewport(viewportWidth: number): number {
  if (!Number.isFinite(viewportWidth)) return MAX_SIDEBAR_WIDTH;
  return Math.max(
    MIN_SIDEBAR_WIDTH,
    Math.min(MAX_SIDEBAR_WIDTH, viewportWidth - MAIN_AREA_TARGET_WIDTH),
  );
}

export function resizeSidebarWithKey(
  current: number,
  key: string,
  upperBound = MAX_SIDEBAR_WIDTH,
): number | null {
  if (key === "Home") return MIN_SIDEBAR_WIDTH;
  if (key === "End") return clampSidebarWidth(MAX_SIDEBAR_WIDTH, upperBound);
  if (key === "ArrowLeft") return clampSidebarWidth(current - SIDEBAR_RESIZE_STEP, upperBound);
  if (key === "ArrowRight") return clampSidebarWidth(current + SIDEBAR_RESIZE_STEP, upperBound);
  return null;
}
```

- [ ] **Step 4: Run the focused test and verify it passes**

Run:

```bash
npm test -- tests/unit/workspaceLayout.spec.ts
```

Expected: all layout helper tests pass.

- [ ] **Step 5: Commit the isolated utility**

```bash
git add src/utils/workspaceLayout.ts tests/unit/workspaceLayout.spec.ts
git commit -m "test(ui): define resizable sidebar layout rules"
```

---

### Task 2: Move context navigation into the top toolbar

**Files:**
- Modify: `src/components/WorkspaceToolbar.vue`
- Test: `tests/unit/WorkspaceToolbar.spec.ts`

- [ ] **Step 1: Write failing toolbar behavior tests**

Mount the component with `global.stubs = { svg: true }` and use the existing required props. Test the public events and accessible state:

```ts
import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import WorkspaceToolbar from "../../src/components/WorkspaceToolbar.vue";

const props = {
  workspace: "terminal" as const,
  connectionState: "connected",
  sidebarExpanded: true,
  onNewSession: () => undefined,
};

describe("WorkspaceToolbar context navigation", () => {
  it("renders all five context buttons in the top toolbar", () => {
    const wrapper = mount(WorkspaceToolbar, { props });
    expect(wrapper.get('[data-testid="context-sessions"]').text()).toContain("会话");
    expect(wrapper.get('[data-testid="context-files"]').text()).toContain("文件");
    expect(wrapper.get('[data-testid="context-keys"]').text()).toContain("密钥");
    expect(wrapper.get('[data-testid="context-tools"]').text()).toContain("工具");
    expect(wrapper.get('[data-testid="context-settings"]').text()).toContain("设置");
  });

  it("emits select-panel and expands the sidebar from a context button", async () => {
    const wrapper = mount(WorkspaceToolbar, { props });
    await wrapper.get('[data-testid="context-keys"]').trigger("click");
    expect(wrapper.emitted("select-panel")).toEqual([["keys"]]);
    expect(wrapper.emitted("toggle-sidebar")).toEqual([[true]]);
  });

  it("emits a toggle request and exposes aria-pressed", async () => {
    const wrapper = mount(WorkspaceToolbar, { props: { ...props, sidebarExpanded: false } });
    const button = wrapper.get('[data-testid="toggle-sidebar"]');
    expect(button.attributes("aria-pressed")).toBe("false");
    await button.trigger("click");
    expect(wrapper.emitted("toggle-sidebar")).toEqual([[true]]);
  });
});
```

- [ ] **Step 2: Run the focused test and verify it fails**

Run:

```bash
npm test -- tests/unit/WorkspaceToolbar.spec.ts
```

Expected: failure because `sidebarExpanded`, the context buttons, and the new events do not exist yet.

- [ ] **Step 3: Extend the toolbar contract and template**

Add these types and props/events near the existing `WorkspaceKind` declaration:

```ts
export type PanelKind = "sessions" | "files" | "keys" | "tools" | "settings";

const props = defineProps<{
  workspace: WorkspaceKind;
  connectionState: string;
  activePanel?: PanelKind;
  sidebarExpanded: boolean;
  onNewSession?: () => void;
  onConnect?: () => void;
  onDisconnect?: () => void;
  onFind?: () => void;
  onClearScreen?: () => void;
  onScreenshot?: () => void;
  onRecord?: () => void;
  onSyncToggle?: () => void;
  onUpload?: () => void;
  onDownload?: () => void;
  onNewFolder?: () => void;
  onDelete?: () => void;
  onRefresh?: () => void;
  onToggleTransferPanel?: () => void;
  syncEnabled?: boolean;
  transferPanelExpanded?: boolean;
}>();

const emit = defineEmits<{
  (e: "change-workspace", workspace: WorkspaceKind): void;
  (e: "select-panel", panel: PanelKind): void;
  (e: "toggle-sidebar", expanded: boolean): void;
}>();

const contextItems: Array<{ id: PanelKind; label: string; icon: string }> = [
  { id: "sessions", label: "会话", icon: "▤" },
  { id: "files", label: "文件", icon: "▥" },
  { id: "keys", label: "密钥", icon: "⚷" },
  { id: "tools", label: "工具", icon: "⚒" },
  { id: "settings", label: "设置", icon: "⚙" },
];

function selectPanel(panel: PanelKind) {
  emit("select-panel", panel);
  emit("toggle-sidebar", true);
}

function toggleSidebar() {
  emit("toggle-sidebar", !props.sidebarExpanded);
}
```

Add a context cluster after the workspace switcher separator. Each button must include `data-testid`, `aria-label`, a title, and active state supplied by a new `activePanel?: PanelKind` prop. Add the sidebar button at the end of the toolbar:

```vue
<div class="context-cluster" role="group" aria-label="上下文面板">
  <button
    v-for="item in contextItems"
    :key="item.id"
    :data-testid="`context-${item.id}`"
    class="tb-btn context-btn"
    :class="{ 'is-on': activePanel === item.id }"
    :title="item.label"
    :aria-label="item.label"
    @click="selectPanel(item.id)"
  >
    <span aria-hidden="true">{{ item.icon }}</span>
    <span class="context-label">{{ item.label }}</span>
  </button>
</div>

<button
  data-testid="toggle-sidebar"
  class="tb-btn"
  title="显示/隐藏侧栏"
  aria-label="显示/隐藏侧栏"
  :aria-pressed="sidebarExpanded"
  @click="toggleSidebar"
>
  <span aria-hidden="true">{{ sidebarExpanded ? "«" : "»" }}</span>
</button>
```

Keep the existing workspace-specific action callbacks and fix the existing transfer sync button to bind `:aria-pressed="syncEnabled"` instead of the current literal `false`.

- [ ] **Step 4: Add compact-toolbar styles and run tests**

Add styles that keep the toolbar single-line, hide `.context-label` below 900px, preserve visible focus, and retain the existing token-based colors:

```css
.context-cluster { display: flex; align-items: center; gap: 2px; min-width: 0; }
.context-btn { gap: 4px; }
.context-btn.is-on { color: var(--rs-accent); background: var(--rs-row-selected); }
.tb-btn:focus-visible,
.ws-btn:focus-visible { outline: 2px solid var(--rs-accent); outline-offset: -2px; }
@media (max-width: 900px) {
  .context-label { display: none; }
  .status-text { max-width: 90px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
}
```

Run:

```bash
npm test -- tests/unit/WorkspaceToolbar.spec.ts
npm run typecheck
```

Expected: toolbar tests pass and `vue-tsc` reports no errors.

- [ ] **Step 5: Commit the toolbar change**

```bash
git add src/components/WorkspaceToolbar.vue tests/unit/WorkspaceToolbar.spec.ts
git commit -m "feat(ui): move context navigation into top toolbar"
```

---

### Task 3: Refactor `SidePanel` into one contextual panel with nested tabs

**Files:**
- Modify: `src/components/SidePanel.vue`
- Modify: `src/components/SessionList.vue`
- Modify: `src/components/KeyManagerPanel.vue`
- Modify: `src/components/QuickCommandPanel.vue`
- Modify: `src/components/TriggerEditor.vue`
- Modify: `src/components/TunnelPanel.vue`
- Modify: `src/components/ThemePanel.vue`
- Modify: `src/components/PluginPanel.vue`
- Test: `tests/unit/SidePanel.spec.ts`

- [ ] **Step 1: Write failing SidePanel tests**

Use child stubs so the test focuses on layout behavior:

```ts
import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import SidePanel from "../../src/components/SidePanel.vue";

const stubs = {
  SessionList: { template: "<div data-testid='session-child' />" },
  KeyManagerPanel: { template: "<div data-testid='key-child' />" },
  QuickCommandPanel: { template: "<div data-testid='quick-child' />" },
  TriggerEditor: { template: "<div data-testid='trigger-child' />" },
  TunnelPanel: { template: "<div data-testid='tunnel-child' />" },
  ThemePanel: { template: "<div data-testid='theme-child' />" },
  PluginPanel: { template: "<div data-testid='plugin-child' />" },
};

describe("SidePanel", () => {
  it("renders only the selected top-level panel", () => {
    const wrapper = mount(SidePanel, { props: { active: "keys" }, global: { stubs } });
    expect(wrapper.find('[data-testid="key-child"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-child"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="quick-child"]').exists()).toBe(false);
  });

  it("renders only one tools subview", async () => {
    const wrapper = mount(SidePanel, { props: { active: "tools" }, global: { stubs } });
    expect(wrapper.find('[data-testid="quick-child"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="trigger-child"]').exists()).toBe(false);
    await wrapper.get('[data-testid="tools-triggers"]').trigger("click");
    expect(wrapper.find('[data-testid="quick-child"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="trigger-child"]').exists()).toBe(true);
  });

  it("exposes a keyboard separator with confirmed geometry", async () => {
    const wrapper = mount(SidePanel, { props: { active: "sessions", width: 280 }, global: { stubs } });
    const separator = wrapper.get('[data-testid="sidebar-separator"]');
    expect(separator.attributes("role")).toBe("separator");
    expect(separator.attributes("aria-valuemin")).toBe("200");
    expect(separator.attributes("aria-valuemax")).toBe("360");
    expect(separator.attributes("aria-valuenow")).toBe("280");
    await separator.trigger("keydown", { key: "ArrowRight" });
    expect(wrapper.emitted("update:width")).toEqual([[288]]);
  });
});
```

- [ ] **Step 2: Run the focused test and verify it fails**

Run:

```bash
npm test -- tests/unit/SidePanel.spec.ts
```

Expected: failure because nested tabs, `width`, the separator, and the single-child rendering do not exist.

- [ ] **Step 3: Add panel types, props, and nested state**

Replace the current `SidePanel` setup contract with:

```ts
import { onBeforeUnmount, ref } from "vue";
import {
  DEFAULT_SIDEBAR_WIDTH,
  MAX_SIDEBAR_WIDTH,
  MIN_SIDEBAR_WIDTH,
  clampSidebarWidth,
  resizeSidebarWithKey,
} from "../utils/workspaceLayout";

export type ToolSubview = "quick-commands" | "triggers" | "tunnels";
export type SettingsSubview = "theme" | "plugins";

const props = withDefaults(defineProps<{
  active: string;
  width?: number;
  maxWidth?: number;
  expanded?: boolean;
}>(), {
  width: DEFAULT_SIDEBAR_WIDTH,
  maxWidth: MAX_SIDEBAR_WIDTH,
  expanded: true,
});

const emit = defineEmits<{
  (e: "update:width", width: number): void;
  (e: "select-session", id: string): void;
  (e: "open-sftp", id: string): void;
  (e: "open-terminal", id: string, path: string): void;
}>();

const activeToolSubview = ref<ToolSubview>("quick-commands");
const activeSettingsSubview = ref<SettingsSubview>("theme");
const dragging = ref(false);
let stopResize: (() => void) | null = null;

function setWidth(value: number) {
  emit("update:width", clampSidebarWidth(value, props.maxWidth));
}

function onSeparatorKeydown(event: KeyboardEvent) {
  const next = resizeSidebarWithKey(props.width, event.key, props.maxWidth);
  if (next === null) return;
  event.preventDefault();
  setWidth(next);
}

function startResize(event: PointerEvent) {
  stopResize?.();
  dragging.value = true;
  event.preventDefault();
  const move = (moveEvent: PointerEvent) => setWidth(moveEvent.clientX);
  const stop = () => {
    dragging.value = false;
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", stop);
    window.removeEventListener("pointercancel", stop);
    stopResize = null;
  };
  stopResize = stop;
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", stop);
  window.addEventListener("pointercancel", stop);
}

onBeforeUnmount(() => stopResize?.());

function resetWidth() {
  setWidth(DEFAULT_SIDEBAR_WIDTH);
}
```

Use `pointermove` rather than mouse-only listeners so the same separator works with a mouse, pen, or touch-capable desktop. Since the sidebar is rendered at the left edge, `clientX` is the desired width; `App.vue` will apply the viewport-aware upper bound when it receives the value.

- [ ] **Step 4: Replace the template with one child and nested tabs**

The panel template must have one header, an optional nested tab row, one body child, and a separator:

```vue
<aside
  class="side-panel"
  :class="{ 'is-dragging': dragging, 'is-collapsed': !props.expanded }"
  :style="{ width: props.expanded ? `${clampSidebarWidth(props.width, props.maxWidth)}px` : '0px' }"
>
  <header class="side-panel-header">
    <h3>{{ titles[active] || active }}</h3>
    <button class="panel-reset" title="恢复侧栏宽度" aria-label="恢复侧栏宽度" @dblclick="resetWidth">↺</button>
  </header>

  <nav v-if="active === 'tools'" class="subview-tabs" aria-label="工具面板">
    <button data-testid="tools-quick-commands" :class="{ active: activeToolSubview === 'quick-commands' }" @click="activeToolSubview = 'quick-commands'">快速命令</button>
    <button data-testid="tools-triggers" :class="{ active: activeToolSubview === 'triggers' }" @click="activeToolSubview = 'triggers'">触发器</button>
    <button data-testid="tools-tunnels" :class="{ active: activeToolSubview === 'tunnels' }" @click="activeToolSubview = 'tunnels'">隧道</button>
  </nav>
  <nav v-else-if="active === 'settings'" class="subview-tabs" aria-label="设置面板">
    <button data-testid="settings-theme" :class="{ active: activeSettingsSubview === 'theme' }" @click="activeSettingsSubview = 'theme'">主题</button>
    <button data-testid="settings-plugins" :class="{ active: activeSettingsSubview === 'plugins' }" @click="activeSettingsSubview = 'plugins'">插件</button>
  </nav>

  <div class="side-panel-body">
    <SessionList
      v-if="active === 'sessions'"
      embedded
      @select="(id) => emit('select-session', id)"
      @open-sftp="(id) => emit('open-sftp', id)"
      @open-terminal="(id, path) => emit('open-terminal', id, path)"
    />
    <div v-else-if="active === 'files'" class="placeholder">
      <p>文件浏览（切片 5.2）</p>
      <p class="hint">本地 / 远端双面板 · 待 Tauri-plugin-fs + SFTP 接入</p>
    </div>
    <KeyManagerPanel v-else-if="active === 'keys'" embedded />
    <QuickCommandPanel
      v-else-if="active === 'tools' && activeToolSubview === 'quick-commands'"
      embedded
    />
    <TriggerEditor
      v-else-if="active === 'tools' && activeToolSubview === 'triggers'"
      embedded
    />
    <TunnelPanel
      v-else-if="active === 'tools' && activeToolSubview === 'tunnels'"
      embedded
    />
    <ThemePanel
      v-else-if="active === 'settings' && activeSettingsSubview === 'theme'"
      embedded
    />
    <PluginPanel
      v-else-if="active === 'settings' && activeSettingsSubview === 'plugins'"
      embedded
    />
  </div>

  <div
    data-testid="sidebar-separator"
    class="sidebar-separator"
    role="separator"
    aria-orientation="vertical"
    :aria-valuemin="MIN_SIDEBAR_WIDTH"
    :aria-valuemax="props.maxWidth"
    :aria-valuenow="props.width"
    tabindex="0"
    @pointerdown="startResize"
    @dblclick="resetWidth"
    @keydown="onSeparatorKeydown"
  />
</aside>
```

The event forwarding expressions are fully specified above; `SessionList` remains the only child emitting session events. Do not render the three tool children or two settings children simultaneously.

- [ ] **Step 5: Add embedded props to existing child panels**

For each listed child component, add:

```ts
const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });
```

Wrap only the component’s outer `header` in `v-if="!props.embedded"`; preserve its data table/form/error content. For `SessionList`, wrap `.tree-header`; for the six management panels, wrap their `header` element. Do not remove the component’s IPC calls or change its error handling. In `SidePanel`, pass `embedded` so the outer `SidePanel` title is the only visible title.

- [ ] **Step 6: Add scoped sidebar styles and run tests**

Use the existing `--rs-*` tokens and keep the body scrollable:

```css
.side-panel {
  position: relative;
  flex: 0 0 auto;
  min-width: 0;
  max-width: var(--rs-sidebar-w-max);
  background: var(--rs-bg-panel);
  border-right: 1px solid var(--rs-border);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  transition: width 250ms var(--rs-easing);
}
.side-panel.is-collapsed { border-right: 0; pointer-events: none; }
.side-panel.is-dragging { transition: none; user-select: none; }
.side-panel-header { height: var(--rs-panel-header-h); display:flex; align-items:center; padding:0 var(--rs-s-3); border-bottom:1px solid var(--rs-border); }
.side-panel-body { flex:1; min-height:0; overflow:auto; }
.subview-tabs { display:flex; min-height:32px; border-bottom:1px solid var(--rs-border); background:var(--rs-bg-surface); }
.subview-tabs button { flex:1; border:0; background:transparent; color:var(--rs-fg-muted); font-size:var(--rs-fs-xs); cursor:pointer; }
.subview-tabs button.active { color:var(--rs-fg); border-bottom:2px solid var(--rs-accent); }
.sidebar-separator { position:absolute; inset-block:0; inset-inline-end:-3px; width:6px; cursor:col-resize; z-index:2; }
.sidebar-separator:hover, .sidebar-separator:focus-visible { background:color-mix(in srgb, var(--rs-accent) 35%, transparent); outline:none; }
```

Run:

```bash
npm test -- tests/unit/SidePanel.spec.ts
npm run typecheck
```

Expected: SidePanel tests pass and no child prop/type errors remain.

- [ ] **Step 7: Commit the contextual sidebar**

```bash
git add src/components/SidePanel.vue src/components/SessionList.vue src/components/KeyManagerPanel.vue src/components/QuickCommandPanel.vue src/components/TriggerEditor.vue src/components/TunnelPanel.vue src/components/ThemePanel.vue src/components/PluginPanel.vue tests/unit/SidePanel.spec.ts
git commit -m "feat(ui): make sidebar contextual and mutually exclusive"
```

---

### Task 4: Integrate the new layout in `App.vue` and preserve workspaces

**Files:**
- Modify: `src/App.vue`
- Modify: `src/styles/tokens.css`
- Test: `tests/unit/AppLayout.spec.ts`

- [ ] **Step 1: Write failing App layout tests**

Mount `App.vue` with child components stubbed and mock the Tauri-dependent stores before import. The tests should assert composition and event behavior rather than xterm or Dockview internals:

```ts
import { describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import App from "../../src/App.vue";

vi.mock("../../src/stores/sessions", () => ({
  useSessionsStore: () => ({
    currentId: null,
    current: null,
    items: [],
    connectionState: new Map(),
    refresh: vi.fn().mockResolvedValue(undefined),
    subscribeEvents: vi.fn().mockResolvedValue(undefined),
    connect: vi.fn().mockResolvedValue(undefined),
  }),
}));
vi.mock("../../src/stores/hostKey", () => ({
  useHostKeyStore: () => ({ subscribeEvents: vi.fn().mockResolvedValue(undefined) }),
}));

describe("App layout", () => {
  const childStubs = {
    CustomTitleBar: { template: "<header data-testid='titlebar' />" },
    WorkspaceToolbar: {
      props: ["workspace", "connectionState", "activePanel", "sidebarExpanded"],
      emits: ["select-panel", "toggle-sidebar", "change-workspace"],
      template: "<div data-testid='toolbar' />",
    },
    SidePanel: {
      props: ["active", "width", "expanded"],
      emits: ["update:width", "select-session", "open-sftp", "open-terminal"],
      template: "<aside data-testid='side-panel' />",
    },
    StatusBar: { template: "<footer data-testid='statusbar' />" },
    DockviewVue: { template: "<div data-testid='dockview'><slot name='terminal' /></div>" },
    TerminalPane: { template: "<div data-testid='terminal-pane' />" },
    TransferWorkspace: { template: "<div data-testid='transfer-workspace' />" },
    TransferPanel: { template: "<div data-testid='transfer-panel' />" },
    SessionCreateDialog: { template: "<div />" },
    HostKeyMismatchDialog: { template: "<div />" },
    TransferQueue: { template: "<div />" },
    MasterPasswordDialog: { template: "<div />" },
  };

  it("opens and hides the mounted sidebar without destroying its subtree", async () => {
    const wrapper = mount(App, { global: { stubs: childStubs } });
    const sidebar = wrapper.findComponent({ name: "SidePanel" });
    expect(sidebar.props("expanded")).toBe(true);
    const toolbarVm = wrapper.findComponent({ name: "WorkspaceToolbar" }).vm;
    await toolbarVm.$emit("toggle-sidebar", false);
    expect(wrapper.findComponent({ name: "SidePanel" }).exists()).toBe(true);
    expect(wrapper.findComponent({ name: "SidePanel" }).props("expanded")).toBe(false);
  });

  it("renders one sidebar and no ActivityBar", () => {
    const wrapper = mount(App, { global: { stubs: childStubs } });
    expect(wrapper.find('[data-testid="side-panel"]').exists()).toBe(true);
    expect(wrapper.findComponent({ name: "ActivityBar" }).exists()).toBe(false);
  });

  it("opens the sidebar and changes the selected panel from toolbar intent", async () => {
    const wrapper = mount(App, { global: { stubs: childStubs } });
    const toolbar = wrapper.find('[data-testid="toolbar"]');
    await toolbar.trigger("click");
    const toolbarVm = wrapper.findComponent({ name: "WorkspaceToolbar" }).vm;
    await toolbarVm.$emit("toggle-sidebar", false);
    await toolbarVm.$emit("select-panel", "keys");
    expect(wrapper.findComponent({ name: "SidePanel" }).props("active")).toBe("keys");
  });
});
```

When converting this outline into code, provide concrete stubs for `CustomTitleBar`, `WorkspaceToolbar`, `SidePanel`, `StatusBar`, `DockviewVue`, dialogs, `TransferWorkspace`, `TransferPanel`, and `TransferQueue`; do not leave a comment placeholder in the committed test.

- [ ] **Step 2: Run the focused test and verify it fails**

Run:

```bash
npm test -- tests/unit/AppLayout.spec.ts
```

Expected: failure because `App.vue` still renders `ActivityBar` and does not expose the new toolbar/sidebar props or width state.

- [ ] **Step 3: Replace App layout state and imports**

In `src/App.vue`:

- Remove the `ActivityBar` import and template node.
- Add `onBeforeUnmount` to the existing Vue lifecycle import and import the pure layout helpers; do not add `watchEffect`.
- Change `activePanel` to the `PanelKind` type and add:

```ts
const sidebarWidth = ref(DEFAULT_SIDEBAR_WIDTH);
const viewportWidth = ref(typeof window === "undefined" ? 1280 : window.innerWidth);

const sidebarMaxWidth = computed(() => maxSidebarWidthForViewport(viewportWidth.value));

function setSidebarWidth(width: number) {
  sidebarWidth.value = clampSidebarWidth(width, sidebarMaxWidth.value);
}

function onViewportResize() {
  viewportWidth.value = window.innerWidth;
  sidebarWidth.value = clampSidebarWidth(sidebarWidth.value, sidebarMaxWidth.value);
}

function selectPanel(panel: PanelKind) {
  activePanel.value = panel;
  panelExpanded.value = true;
}

function toggleSidebar(expanded?: boolean) {
  panelExpanded.value = expanded ?? !panelExpanded.value;
}
```

Register `window.addEventListener("resize", onViewportResize)` in `onMounted` and remove it in `onBeforeUnmount`. Keep `selectSession`, `onOpenSftp`, `onOpenTerminal`, and all existing store subscriptions unchanged except for typed panel values.

- [ ] **Step 4: Replace the body composition and preserve both workspaces**

Use this structure in the body:

```vue
<div class="body">
  <SidePanel
    :active="activePanel"
    :width="sidebarWidth"
    :max-width="sidebarMaxWidth"
    :expanded="panelExpanded"
    @update:width="setSidebarWidth"
    @select-session="selectSession"
    @open-sftp="onOpenSftp"
    @open-terminal="onOpenTerminal"
  />

  <main class="main-area">
    <div v-show="workspace === 'terminal'" class="workspace-layer terminal-layer">
      <DockviewVue
        v-if="activeTerminal"
        :components="components"
        style="width: 100%; height: 100%"
      >
        <template #terminal="{ params }">
          <TerminalPane :session-id="params.sessionId" />
        </template>
      </DockviewVue>
      <div v-else class="empty">
        <div class="empty-content">
          <div class="empty-icon">⌬</div>
          <h2>RShell</h2>
          <p>从左侧「会话」面板新建或选择一个 SSH 会话</p>
          <el-button type="primary" @click="openNewSession">新建会话</el-button>
        </div>
      </div>
    </div>
    <div v-show="workspace === 'transfer'" class="workspace-layer transfer-layer">
      <div class="transfer-area">
        <TransferWorkspace
          v-if="activeTransferSession"
          :session-id="activeTransferSession"
          :sync-enabled="syncEnabled"
          style="flex: 1; min-height: 0"
        />
        <div v-else class="empty">
          <div class="empty-content">
            <div class="empty-icon">⇄</div>
            <h2>传输工作区</h2>
            <p>从左侧「会话」右键 → 打开 SFTP,或先连接一个会话</p>
            <el-button type="primary" :disabled="!activeTerminal" @click="onOpenSftp(activeTerminal!)">
              使用当前会话
            </el-button>
          </div>
        </div>
        <TransferPanel
          :expanded="transferPanelExpanded"
          @toggle="transferPanelExpanded = !transferPanelExpanded"
        />
      </div>
    </div>
  </main>
</div>
```

Add an `expanded: boolean` prop to `SidePanel`. Keep the component mounted so nested subview state and in-flight panel operations survive hiding, but make its rendered shell occupy zero width and hide overflow when `expanded` is false. The panel’s inline style should use the parent-controlled width and expansion state; this is a visual hide, not a destruction of the panel subtree.

Pass the new toolbar props/events:

```vue
<WorkspaceToolbar
  :workspace="workspace"
  :connection-state="currentConnectionState"
  :active-panel="activePanel"
  :sidebar-expanded="panelExpanded"
  :sync-enabled="syncEnabled"
  :transfer-panel-expanded="transferPanelExpanded"
  :on-new-session="openNewSession"
  @change-workspace="pickWorkspace"
  @select-panel="selectPanel"
  @toggle-sidebar="toggleSidebar"
  @sync-toggle="syncEnabled = !syncEnabled"
  @refresh="store.refresh()"
  @toggle-transfer-panel="transferPanelExpanded = !transferPanelExpanded"
/>
```

Keep the existing `CustomTitleBar` menu events mapped to the same handlers: call `selectPanel('keys' | 'settings' | 'tools')` for feature entries and call `toggleSidebar()` for the sidebar entry. The handler accepts an optional boolean so the existing no-argument menu event toggles while the toolbar event can explicitly set the requested state.

- [ ] **Step 5: Update layout tokens and shell styles**

In `src/styles/tokens.css`, replace the current geometry values:

```css
--rs-sidebar-w: 280px;
--rs-sidebar-w-min: 200px;
--rs-sidebar-w-max: 360px;
--rs-sidebar-resize-step: 8px;
--rs-sidebar-separator-w: 6px;
--rs-panel-header-h: 32px;
```

Retain the existing title, toolbar, tab, status, and transfer heights. In `App.vue` scoped styles, add:

```css
.workspace-layer { width:100%; height:100%; min-width:0; min-height:0; }
.terminal-layer, .transfer-layer { display:flex; flex-direction:column; }
```

Do not add a page-level scrollbar. Ensure `.body`, `.main-area`, and both workspace layers have `overflow:hidden` or their existing equivalent.

- [ ] **Step 6: Run focused tests and typecheck**

Run:

```bash
npm test -- tests/unit/AppLayout.spec.ts tests/unit/SidePanel.spec.ts tests/unit/WorkspaceToolbar.spec.ts tests/unit/workspaceLayout.spec.ts
npm run typecheck
```

Expected: all focused tests pass and `vue-tsc --noEmit` reports no errors.

- [ ] **Step 7: Commit the App integration**

```bash
git add src/App.vue src/styles/tokens.css tests/unit/AppLayout.spec.ts
 git commit -m "feat(ui): integrate resizable contextual sidebar"
```

Remove the accidental leading space before `git commit` when executing the command.

---

### Task 5: Verify the complete frontend build and real layout behavior

**Files:**
- Modify only if verification finds a concrete regression: the files from Tasks 1–4
- Test: all existing `tests/unit/**/*.spec.ts`

- [ ] **Step 1: Run the complete automated suite**

Run each command separately and record the result:

```bash
npm run typecheck
npm test
npm run build
```

Expected: all three commands exit 0; Vitest reports the existing tests plus the new layout tests with no failures; Vite emits `dist/` successfully.

- [ ] **Step 2: Start the actual Vite preview**

Use the project preview launcher rather than a background shell server. If `.claude/launch.json` is absent, create a minimal entry for the existing Vite command and port 51820, then start it with the browser preview tool.

Verify at 1280×800 and 1024×768:

- top bar contains terminal/transfer and five context buttons;
- no left ActivityBar is visible;
- default sidebar is 280px;
- dragging the separator changes width and never crosses 200/360px;
- double-clicking separator restores 280px;
- hiding sidebar gives the main area the full width;
- tools shows exactly one of quick commands/triggers/tunnels;
- settings shows exactly one of theme/plugins;
- switching context does not remove the terminal or transfer state.

Then resize below 900px and verify labels collapse to icons without clipping the core actions.

- [ ] **Step 3: Inspect browser console and accessibility semantics**

Use the browser console and accessibility snapshot to confirm:

- no runtime errors from unmocked Tauri calls in the running shell;
- sidebar separator has `role="separator"`, vertical orientation, and current min/max/value attributes;
- toolbar context buttons have accessible labels and visible focus rings;
- toggle button’s `aria-pressed` follows the visible sidebar state.

- [ ] **Step 4: Review the final diff for scope**

Run:

```bash
git status --short
git diff --stat HEAD~4..HEAD
git diff --check HEAD~4..HEAD
```

Expected: only the layout utility, frontend layout components, child embedded-header props, layout tests, and token/style changes are present; no `src-tauri/` or IPC contract files changed.

- [ ] **Step 5: Run the code simplifier review**

After automated and browser verification, invoke the project’s code-simplifier agent on the changed frontend files. Apply only simplifications that preserve the tested behavior, then rerun `npm run typecheck`, `npm test`, and `npm run build`.

- [ ] **Step 6: Commit verification-only fixes when needed**

If verification finds a concrete regression, fix it in the relevant file, rerun all three commands from Step 1, and commit the verified correction separately:

```bash
git add src tests/unit
git commit -m "fix(ui): polish workspace layout verification findings"
```

Do not commit `.superpowers/brainstorm/` artifacts.

---

## Plan self-review

- **Spec coverage:** Top navigation is Task 2; single contextual sidebar and nested tabs are Task 3; 280/200/360 resizing, keyboard semantics, and responsive bounds are Tasks 1, 3, and 4; workspace preservation is Task 4; embedded duplicate-title handling is Task 3; no IPC/backend changes are enforced in the file map and Task 5 scope check; automated and manual acceptance are Task 5.
- **Placeholder scan:** The plan contains no `TBD`, `TODO`, `待定`, or implementation-placeholder instructions. The test outline in Task 4 explicitly requires concrete stubs before committing; no comment placeholder may be left in code.
- **Type consistency:** `PanelKind`, `ToolSubview`, `SettingsSubview`, sidebar constants, and event names are defined before later tasks use them. `WorkspaceToolbar` emits `toggle-sidebar` with a boolean; `App.vue` consumes that boolean; `SidePanel` emits `update:width` with a number.
- **Implementation note:** The plan deliberately uses `pointermove` and `v-show` for the two highest-risk interaction details: cross-device separator input and preservation of terminal/transfer component state.
