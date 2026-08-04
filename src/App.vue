<script setup lang="ts">
/**
 * RShell 主布局 —— v2 重设计 (Xshell + Xftp 融合)
 *
 * 结构:
 *   ┌─────────────────────────────────────────────┐
 *   │ CustomTitleBar  (32px, 拖动 + 窗口控制)      │
 *   ├─────────────────────────────────────────────┤
 *   │ WorkspaceToolbar  (40px, switcher + 条件按钮)│
 *   ├──────────┬──────────────────────────────────┤
 *   │Activity  │ Workspace Content Area            │
 *   │Bar       │  - Terminal: Dockview 多终端      │
 *   │(48px)    │  - Transfer: 双窗格 + 传输面板    │
 *   ├──────────┴──────────────────────────────────┤
 *   │ StatusBar  (24px, 12 字段)                   │
 *   └─────────────────────────────────────────────┘
 *
 * 浮层:SessionCreateDialog / HostKeyMismatchDialog /
 *       MasterPasswordDialog
 */
import { onBeforeUnmount, onMounted, ref, markRaw, computed } from "vue";
import { DockviewVue } from "dockview-vue";
import "dockview-vue/dist/styles/dockview.css";
import TerminalPane from "./components/TerminalPane.vue";
import TransferWorkspace from "./components/transfer/TransferWorkspace.vue";
import SessionCreateDialog from "./components/SessionCreateDialog.vue";
import HostKeyMismatchDialog from "./components/HostKeyMismatchDialog.vue";
import MasterPasswordDialog from "./components/MasterPasswordDialog.vue";
import CustomTitleBar from "./components/CustomTitleBar.vue";
import SidePanel from "./components/SidePanel.vue";
import StatusBar from "./components/StatusBar.vue";
import WorkspaceToolbar, {
  type WorkspaceKind,
  type PanelKind,
} from "./components/WorkspaceToolbar.vue";
import TransferPanel from "./components/TransferPanel.vue";
import {
  DEFAULT_SIDEBAR_WIDTH,
  clampSidebarWidth,
  maxSidebarWidthForViewport,
} from "./utils/workspaceLayout";
import { useSessionsStore } from "./stores/sessions";
import { useHostKeyStore } from "./stores/hostKey";
import { useThemeStore } from "./stores/theme";
import type { Uuid } from "./ipc/types";

const store = useSessionsStore();
const hostKeyStore = useHostKeyStore();
const themeStore = useThemeStore();
const dialogVisible = ref(false);
const activeTerminal = ref<Uuid | null>(null);
const activePanel = ref<PanelKind>("sessions");
const panelExpanded = ref(true);

const workspace = ref<WorkspaceKind>("terminal");
const syncEnabled = ref(false);
const transferPanelExpanded = ref(true);
const activeTransferSession = ref<Uuid | null>(null);

const sidebarWidth = ref(DEFAULT_SIDEBAR_WIDTH);
const viewportWidth = ref(
  typeof window === "undefined" ? 1280 : window.innerWidth,
);

const sidebarMaxWidth = computed(() =>
  maxSidebarWidthForViewport(viewportWidth.value),
);

const components = markRaw({ TerminalPane: TerminalPane as never });

function setSidebarWidth(width: number) {
  sidebarWidth.value = clampSidebarWidth(width, sidebarMaxWidth.value);
}

function onViewportResize() {
  viewportWidth.value = window.innerWidth;
  sidebarWidth.value = clampSidebarWidth(
    sidebarWidth.value,
    sidebarMaxWidth.value,
  );
}

function selectPanel(panel: PanelKind) {
  activePanel.value = panel;
  panelExpanded.value = true;
}

function toggleSidebar(expanded?: boolean) {
  panelExpanded.value = expanded ?? !panelExpanded.value;
}

async function selectSession(id: Uuid) {
  activeTerminal.value = id;
  store.currentId = id;
  try {
    await store.connect(id);
  } catch (e) {
    console.error("connect failed", e);
  }
}

function openNewSession() {
  dialogVisible.value = true;
}

function openPanel(name: PanelKind) {
  activePanel.value = name;
  panelExpanded.value = true;
}

function pickWorkspace(w: WorkspaceKind) {
  workspace.value = w;
  if (w === "transfer" && !activeTransferSession.value && store.currentId) {
    activeTransferSession.value = store.currentId;
  }
}

function onOpenSftp(id: Uuid) {
  workspace.value = "transfer";
  activeTransferSession.value = id;
}

function onOpenTerminal(_id: Uuid, _path: string) {
  workspace.value = "terminal";
}

const currentConnectionState = computed(() => {
  const id = activeTerminal.value;
  if (!id) return "disconnected";
  return store.connectionState.get(id) ?? "disconnected";
});

onMounted(async () => {
  window.addEventListener("resize", onViewportResize);
  await store.refresh();
  await store.subscribeEvents();
  await hostKeyStore.subscribeEvents();
  // 主题启动恢复:拉一次当前主题名,并订阅 ThemeChanged/ColorSchemeChanged
  // 事件以接收颜色集并写入 :root。仅在 ThemePanel 挂载时拉取会导致首屏
  // 应用不应用主题,因此在 App 启动时统一初始化。
  await themeStore.refresh();
  await themeStore.subscribeEvents();
});

onBeforeUnmount(() => {
  window.removeEventListener("resize", onViewportResize);
});
</script>

<template>
  <div class="rshell-shell">
    <CustomTitleBar
      @new-session="openNewSession"
      @toggle-sidebar="toggleSidebar"
      @open-key-manager="openPanel('keys')"
      @open-theme-panel="openPanel('settings')"
      @open-plugin-panel="openPanel('settings')"
      @open-transfer-queue="openPanel('tools')"
      @open-quick-commands="openPanel('tools')"
      @open-triggers="openPanel('tools')"
      @open-tunnels="openPanel('tools')"
      @about="openPanel('settings')"
    />

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
      @upload="() => {}"
      @download="() => {}"
      @new-folder="() => {}"
      @delete="() => {}"
      @refresh="store.refresh()"
      @toggle-transfer-panel="transferPanelExpanded = !transferPanelExpanded"
    />

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
        <!-- Terminal Workspace -->
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

        <!-- Transfer Workspace -->
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

    <StatusBar :workspace="workspace" />

    <SessionCreateDialog
      :visible="dialogVisible"
      @close="dialogVisible = false"
      @created="(id) => selectSession(id)"
    />
    <HostKeyMismatchDialog />
    <MasterPasswordDialog />
  </div>
</template>

<style scoped>
.rshell-shell {
  display: flex;
  flex-direction: column;
  width: 100vw;
  height: 100vh;
  margin: 0;
  padding: 0;
  background: var(--rs-bg);
  color: var(--rs-fg);
  font-family: var(--rs-font-ui);
  overflow: hidden;
  box-sizing: border-box;
}

.body {
  flex: 1 1 0;
  display: flex;
  min-height: 0;
  min-width: 0;
  margin: 0;
  padding: 0;
  overflow: hidden;
  box-sizing: border-box;
}

.main-area {
  flex: 1 1 0;
  background: var(--rs-bg);
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  margin: 0;
  padding: 0;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
}

.transfer-area {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  min-height: 0;
}

.workspace-layer { width: 100%; height: 100%; min-width: 0; min-height: 0; }
.terminal-layer,
.transfer-layer { display: flex; flex-direction: column; }

.empty {
  display: flex;
  align-items: center;
  justify-content: center;
  flex: 1;
  margin: 0;
  padding: 0;
}
.empty-content {
  text-align: center;
  color: var(--rs-fg-muted);
}
.empty-icon {
  font-size: 64px;
  color: var(--rs-accent);
  opacity: 0.6;
  margin-bottom: var(--rs-s-4);
}
.empty-content h2 {
  margin: 0 0 var(--rs-s-2);
  font-size: var(--rs-fs-2xl);
  font-weight: 500;
  color: var(--rs-fg);
}
.empty-content p {
  margin: 0 0 var(--rs-s-4);
  font-size: var(--rs-fs-md);
}
</style>