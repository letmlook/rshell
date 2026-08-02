<script setup lang="ts">
/**
 * WorkspaceToolbar —— v2 重设计
 *
 * 工作区工具栏 (40px),三个分区,从左到右:
 *   1. Workspace Switcher [⌨终端 | 📁传输]  —— segment control
 *   2. 通用按钮  —— 跟当前 workspace 无关(新建会话、刷新、设置)
 *   3. Workspace 条件按钮  —— Terminal/Transfer 各自的不同动作
 *
 * 所有图标 inline SVG,不依赖图标库(确保零额外依赖)。
 *
 * 设计 token 全部走 --rs-* 变量;无硬编码颜色。
 */
import { computed } from "vue";

export type WorkspaceKind = "terminal" | "transfer";
export type PanelKind = "sessions" | "files" | "keys" | "tools" | "settings";

const props = defineProps<{
  workspace: WorkspaceKind;
  connectionState: string;
  activePanel?: PanelKind;
  sidebarExpanded: boolean;
  /** Terminal 工具栏额外暴露的快捷动作(由 App.vue 传入) */
  onNewSession?: () => void;
  onConnect?: () => void;
  onDisconnect?: () => void;
  onFind?: () => void;
  onClearScreen?: () => void;
  onScreenshot?: () => void;
  onRecord?: () => void;
  /** Transfer 工具栏 */
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
  (e: "change-workspace", w: WorkspaceKind): void;
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

const statusLabel = computed(() => {
  const s = props.connectionState;
  if (!s || s === "disconnected") return "未连接";
  if (s === "connecting") return "连接中";
  if (s === "connected") return "已连接";
  if (s === "failed") return "失败";
  return s;
});

function pickWorkspace(w: WorkspaceKind) {
  emit("change-workspace", w);
}
</script>

<template>
  <div class="workspace-toolbar" role="toolbar" aria-label="工作区工具栏">
    <!-- 1. Workspace Switcher -->
    <div class="switcher" role="tablist">
      <button
        class="ws-btn"
        :class="{ 'is-active': workspace === 'terminal' }"
        role="tab"
        :aria-selected="workspace === 'terminal'"
        @click="pickWorkspace('terminal')"
      >
        <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true">
          <rect x="1.5" y="2.5" width="13" height="9" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" />
          <path d="M3.5 5 H12.5 M3.5 7 H10 M3.5 9 H8" stroke="currentColor" stroke-width="1.2" fill="none" stroke-linecap="round" />
        </svg>
        <span>终端</span>
      </button>
      <button
        class="ws-btn"
        :class="{ 'is-active': workspace === 'transfer' }"
        role="tab"
        :aria-selected="workspace === 'transfer'"
        @click="pickWorkspace('transfer')"
      >
        <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true">
          <path
            d="M2 4.5 V12.5 H14 V4.5 Z M2 4.5 L8 8.5 L14 4.5"
            fill="none"
            stroke="currentColor"
            stroke-width="1.2"
            stroke-linejoin="round"
          />
        </svg>
        <span>传输</span>
      </button>
    </div>

    <div class="sep" aria-hidden="true" />

    <!-- Context cluster: 上下文面板 (会话/文件/密钥/工具/设置) -->
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

    <!-- 2. 通用按钮 -->
    <div class="cluster">
      <button class="tb-btn" title="新建会话 (Ctrl+N)" aria-label="新建会话" @click="onNewSession">
        <svg width="14" height="14" viewBox="0 0 16 16">
          <path d="M8 3 V13 M3 8 H13" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
        </svg>
      </button>
    </div>

    <div class="sep" aria-hidden="true" />

    <!-- 3. Workspace 条件按钮 -->
    <template v-if="workspace === 'terminal'">
      <div class="cluster">
        <button class="tb-btn" title="连接" aria-label="连接" @click="onConnect">
          <svg width="14" height="14" viewBox="0 0 16 16">
            <path d="M11 3.5 L14 8 L11 12.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
            <path d="M14 8 H4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
            <path d="M2 12 V4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
          </svg>
        </button>
        <button class="tb-btn" title="断开" aria-label="断开" @click="onDisconnect">
          <svg width="14" height="14" viewBox="0 0 16 16">
            <path d="M11 3.5 L14 8 L11 12.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" transform="rotate(180 8 8)" />
            <path d="M14 8 H4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
            <path d="M2 4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
          </svg>
        </button>
      </div>
      <div class="sep" aria-hidden="true" />
      <div class="cluster">
        <button class="tb-btn" title="查找 (Ctrl+F)" aria-label="查找" @click="onFind">
          <svg width="14" height="14" viewBox="0 0 16 16">
            <circle cx="7" cy="7" r="4.5" fill="none" stroke="currentColor" stroke-width="1.4" />
            <path d="M10.5 10.5 L14 14" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
          </svg>
        </button>
        <button class="tb-btn" title="清屏 (Ctrl+L)" aria-label="清屏" @click="onClearScreen">
          <svg width="14" height="14" viewBox="0 0 16 16">
            <rect x="2" y="3" width="12" height="10" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" />
            <path d="M5 6 L11 6 M5 9 L9 9" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
          </svg>
        </button>
        <button class="tb-btn" title="截图" aria-label="截图" @click="onScreenshot">
          <svg width="14" height="14" viewBox="0 0 16 16">
            <rect x="1.5" y="3.5" width="13" height="9" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" />
            <circle cx="8" cy="8" r="2.4" fill="none" stroke="currentColor" stroke-width="1.2" />
          </svg>
        </button>
        <button class="tb-btn" title="录制" aria-label="录制" @click="onRecord">
          <svg width="14" height="14" viewBox="0 0 16 16">
            <circle cx="8" cy="8" r="4" fill="currentColor" />
          </svg>
        </button>
      </div>
    </template>

    <template v-else>
      <div class="cluster">
        <button
          class="tb-btn"
          :class="{ 'is-on': syncEnabled }"
          :title="syncEnabled ? '同步浏览(已开启)' : '同步浏览(关闭)'"
          aria-label="同步浏览"
          :aria-pressed="syncEnabled"
          @click="onSyncToggle"
        >
          <svg width="14" height="14" viewBox="0 0 16 16">
            <path d="M3 8 H13 M9 4 L13 8 L9 12" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
        <button class="tb-btn" title="上传" aria-label="上传" @click="onUpload">
          <svg width="14" height="14" viewBox="0 0 16 16">
            <path d="M8 12 V3 M4 7 L8 3 L12 7" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
        <button class="tb-btn" title="下载" aria-label="下载" @click="onDownload">
          <svg width="14" height="14" viewBox="0 0 16 16">
            <path d="M8 3 V12 M4 8 L8 12 L12 8" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
      </div>
      <div class="sep" aria-hidden="true" />
      <div class="cluster">
        <button class="tb-btn" title="新建文件夹" aria-label="新建文件夹" @click="onNewFolder">
          <svg width="14" height="14" viewBox="0 0 16 16">
            <path d="M2 4.5 V12.5 H14 V6 H8 L6.5 4.5 Z" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" />
            <path d="M8 8.5 V11.5 M7 10 H9" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
          </svg>
        </button>
        <button class="tb-btn danger" title="删除" aria-label="删除" @click="onDelete">
          <svg width="14" height="14" viewBox="0 0 16 16">
            <rect x="3" y="4.5" width="10" height="9" rx="0.5" fill="none" stroke="currentColor" stroke-width="1.2" />
            <path d="M5 4.5 V3.5 H11 V4.5 M2 4.5 H14" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
          </svg>
        </button>
        <button class="tb-btn" title="刷新" aria-label="刷新" @click="onRefresh">
          <svg width="14" height="14" viewBox="0 0 16 16">
            <path d="M13 8 A5 5 0 1 1 11.5 4.2" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
            <path d="M11.5 2.5 V5 H9" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
      </div>
      <div class="sep" aria-hidden="true" />
      <div class="cluster">
        <button
          class="tb-btn"
          :class="{ 'is-on': transferPanelExpanded }"
          title="传输队列"
          aria-label="传输队列"
          @click="onToggleTransferPanel"
        >
          <svg width="14" height="14" viewBox="0 0 16 16">
            <path d="M2 5 H14 M2 8 H14 M2 11 H10" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
          </svg>
        </button>
      </div>
    </template>

    <!-- 右:连接状态摘要(浮在最右,工具栏里其它操作不会越界) -->
    <div class="spacer" />
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
    <div class="status">
      <span class="rs-status-dot" :class="`rs-status-dot--${connectionState}`" aria-hidden="true" />
      <span class="status-text">{{ statusLabel }}</span>
    </div>
  </div>
</template>

<style scoped>
.workspace-toolbar {
  display: flex;
  align-items: center;
  height: var(--rs-toolbar-h);
  background: var(--rs-bg-panel);
  border-bottom: 1px solid var(--rs-border);
  padding: 0 var(--rs-s-2);
  gap: var(--rs-s-1);
  flex-shrink: 0;
  -webkit-app-region: no-drag;
}

/* ---- workspace switcher ---- */
.switcher {
  display: flex;
  align-items: center;
  gap: 0;
  background: var(--rs-bg-surface);
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-1);
  padding: 2px;
  height: 28px;
}
.ws-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 22px;
  padding: 0 10px;
  background: transparent;
  border: none;
  border-radius: var(--rs-radius-1);
  color: var(--rs-fg-muted);
  font-family: var(--rs-font-ui);
  font-size: var(--rs-fs-sm);
  cursor: pointer;
  position: relative;
  transition: color var(--rs-dur-fast) var(--rs-easing), background var(--rs-dur-fast) var(--rs-easing);
}
.ws-btn:hover { color: var(--rs-fg); }
.ws-btn.is-active {
  background: var(--rs-row-selected);
  color: var(--rs-fg);
}
.ws-btn.is-active::after {
  content: "";
  position: absolute;
  left: 12px;
  right: 12px;
  bottom: -4px;
  height: 2px;
  background: var(--rs-accent);
  border-radius: 1px;
}

/* ---- separator ---- */
.sep {
  width: 1px;
  height: 18px;
  background: var(--rs-border);
  margin: 0 var(--rs-s-1);
}

/* ---- cluster (按钮分组) ---- */
.cluster {
  display: flex;
  align-items: center;
  gap: 2px;
}

.tb-btn {
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: 1px solid transparent;
  border-radius: var(--rs-radius-1);
  color: var(--rs-fg-muted);
  cursor: pointer;
  transition: background var(--rs-dur-fast) var(--rs-easing),
    color var(--rs-dur-fast) var(--rs-easing),
    border-color var(--rs-dur-fast) var(--rs-easing);
  padding: 0;
}
.tb-btn:hover {
  background: var(--rs-bg-surface-hover);
  color: var(--rs-fg);
}
.tb-btn:active {
  background: var(--rs-row-selected);
}
.tb-btn.is-on {
  background: var(--rs-row-selected);
  color: var(--rs-accent);
  border-color: var(--rs-border);
}
.tb-btn.danger:hover {
  color: var(--rs-p-danger);
}

.spacer { flex: 1 1 0; }

.status {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 var(--rs-s-2);
  color: var(--rs-fg-muted);
  font-size: var(--rs-fs-xs);
  font-family: var(--rs-font-display);
}
.rs-status-dot {
  width: 8px;
  height: 8px;
}

.context-cluster { display: flex; align-items: center; gap: 2px; min-width: 0; }
.context-btn { width: auto; min-width: 28px; height: 28px; padding: 0 10px; gap: 4px; white-space: nowrap; }
.context-btn.is-on { color: var(--rs-accent); background: var(--rs-row-selected); }
.tb-btn:focus-visible,
.ws-btn:focus-visible { outline: 2px solid var(--rs-accent); outline-offset: -2px; }
@media (max-width: 900px) {
  .context-label { display: none; }
  .status-text { max-width: 90px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
}
</style>