<script setup lang="ts">
/**
 * SidePanel —— 切片 10
 *
 * 中间二级面板(在 ActivityBar 与 main-area 之间),按 active prop
 * 渲染对应子组件(sessions / files / keys / tools / settings)。
 *
 * 切片 11: 单一上下文面板 —— 同一时间只渲染一个子组件,
 * tools / settings 内部由嵌套 subview-tabs 切换子视图;
 * 内嵌时给子组件传 embedded=true 抑制它们的外层 header。
 */
import { onBeforeUnmount, ref } from "vue";
import {
  DEFAULT_SIDEBAR_WIDTH,
  MAX_SIDEBAR_WIDTH,
  MIN_SIDEBAR_WIDTH,
  clampSidebarWidth,
  resizeSidebarWithKey,
} from "../utils/workspaceLayout";
import ThemePanel from "./ThemePanel.vue";
import SessionList from "./SessionList.vue";
import KeyManagerPanel from "./KeyManagerPanel.vue";
import QuickCommandPanel from "./QuickCommandPanel.vue";
import TriggerEditor from "./TriggerEditor.vue";
import TunnelPanel from "./TunnelPanel.vue";
import PluginPanel from "./PluginPanel.vue";

export type ToolSubview = "quick-commands" | "triggers" | "tunnels";
export type SettingsSubview = "theme" | "plugins";

const props = withDefaults(
  defineProps<{
    active: string;
    width?: number;
    maxWidth?: number;
    expanded?: boolean;
  }>(),
  {
    width: DEFAULT_SIDEBAR_WIDTH,
    maxWidth: MAX_SIDEBAR_WIDTH,
    expanded: true,
  },
);

const emit = defineEmits<{
  (e: "update:width", width: number): void;
  (e: "select-session", id: string): void;
  (e: "open-sftp", id: string): void;
  (e: "open-terminal", id: string, path: string): void;
}>();

const titles: Record<string, string> = {
  sessions: "会话",
  files: "文件浏览",
  keys: "SSH 密钥",
  tools: "快速命令与触发器",
  settings: "主题与插件",
};

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
</script>

<template>
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
        @open-terminal="(id, p) => emit('open-terminal', id, p)"
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
</template>

<style scoped>
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
.panel-reset { background:transparent; border:0; color:var(--rs-fg-muted); cursor:pointer; padding:0 6px; }
.panel-reset:hover { color:var(--rs-fg); }
.placeholder {
  padding: 24px 16px;
  color: var(--rs-fg-muted);
  font-size: var(--rs-fs-xs);
}
.placeholder .hint {
  font-size: var(--rs-fs-xs);
  color: var(--rs-fg-disabled);
  margin-top: 4px;
}
</style>