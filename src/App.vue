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
import { ElMessage } from "element-plus/es/components/message/index.mjs";
import { ElNotification } from "element-plus/es/components/notification/index.mjs";
import "dockview-vue/dist/styles/dockview.css";
import TerminalPane from "./components/TerminalPane.vue";
import TransferWorkspace from "./components/transfer/TransferWorkspace.vue";
import SessionCreateDialog from "./components/SessionCreateDialog.vue";
import HostKeyMismatchDialog from "./components/HostKeyMismatchDialog.vue";
import MasterPasswordDialog from "./components/MasterPasswordDialog.vue";
import CustomTitleBar from "./components/CustomTitleBar.vue";
import SidePanel, { type ToolSubview, type SettingsSubview } from "./components/SidePanel.vue";
import StatusBar from "./components/StatusBar.vue";
import WorkspaceToolbar, {
  type WorkspaceKind,
  type PanelKind,
} from "./components/WorkspaceToolbar.vue";
import TransferPanel, { type TransferItem } from "./components/TransferPanel.vue";
import { listTransfers } from "./ipc/client";
import { subscribeAppEvents } from "./ipc/events";
import {
  DEFAULT_SIDEBAR_WIDTH,
  clampSidebarWidth,
  maxSidebarWidthForViewport,
} from "./utils/workspaceLayout";
import { toTransferItem } from "./utils/transferItem";
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
const requestedToolSubview = ref<ToolSubview>("quick-commands");
const requestedSettingsSubview = ref<SettingsSubview>("theme");
const toolSubviewRequest = ref(0);
const settingsSubviewRequest = ref(0);
const panelExpanded = ref(true);

const workspace = ref<WorkspaceKind>("terminal");
const syncEnabled = ref(false);
const transferPanelExpanded = ref(true);
const activeTransferSession = ref<Uuid | null>(null);
const transferWorkspace = ref<InstanceType<typeof TransferWorkspace> | null>(null);
const transferCapabilities = ref({ upload: false, download: false, createFolder: false, delete: false, refresh: false, sync: false });
const transferItems = ref<TransferItem[]>([]);
/** 队列读取/订阅失败的提示；非空时 TransferPanel 就地展示（区别于"确实没有任务"） */
const transferLoadError = ref<string | null>(null);
let unlistenTransfers: (() => void) | null = null;
let mounted = false;

async function refreshTransfers() {
  try {
    transferItems.value = (await listTransfers()).map(toTransferItem);
    transferLoadError.value = null;
  } catch (error) {
    // 不能静默：面板必须能区分「读取失败」与「确实没有任务」
    transferLoadError.value = String(error);
    console.error("无法读取传输队列", error);
  }
}

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
  if (store.connectionState.get(id) === "connected" || store.connectionState.get(id) === "connecting") return;
  try {
    await store.connect(id);
  } catch (e) {
    ElMessage.error(`连接失败：${String(e)}`);
  }
}

function openNewSession() {
  dialogVisible.value = true;
}

function openPanel(name: PanelKind) {
  activePanel.value = name;
  panelExpanded.value = true;
}

function openToolSubview(name: ToolSubview) {
  requestedToolSubview.value = name;
  toolSubviewRequest.value++;
  openPanel("tools");
}

function openSettingsSubview(name: SettingsSubview) {
  requestedSettingsSubview.value = name;
  settingsSubviewRequest.value++;
  openPanel("settings");
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
  void selectSession(id);
}

function onOpenTerminal(_id: Uuid, _path: string) {
  workspace.value = "terminal";
  void selectSession(_id);
}

async function connectCurrent() {
  if (!store.currentId) return;
  try { await store.connect(store.currentId); }
  catch (error) { ElMessage.error(`连接失败：${String(error)}`); }
}

async function disconnectCurrent() {
  if (!store.currentId) return;
  try { await store.disconnect(store.currentId); }
  catch (error) { ElMessage.error(`断开失败：${String(error)}`); }
}

function terminalAction(action: "find" | "clear") {
  if (!activeTerminal.value) return;
  window.dispatchEvent(new CustomEvent("rshell:terminal-action", { detail: { sessionId: activeTerminal.value, action } }));
}

const currentConnectionState = computed(() => {
  const id = activeTerminal.value;
  if (!id) return "disconnected";
  return store.connectionState.get(id) ?? "disconnected";
});

onMounted(async () => {
  mounted = true;
  window.addEventListener("resize", onViewportResize);
  await store.refresh();
  if (!mounted) return;
  await store.subscribeEvents();
  if (!mounted) return;
  await hostKeyStore.subscribeEvents();
  if (!mounted) return;
  await themeStore.refresh();
  if (!mounted) return;
  await themeStore.subscribeEvents();
  if (!mounted) return;
  await refreshTransfers();
  if (!mounted) return;
  try {
    const stop = await subscribeAppEvents((event) => {
      if (event === "TransferQueueChanged" || (typeof event === "object" && event !== null && (
        "TransferCompleted" in event || "TransferFailed" in event || "TransferProgress" in event
      ))) void refreshTransfers();
      if (typeof event !== "string" && "TriggerFired" in event && event.TriggerFired.action_summary.startsWith("notify: ")) {
        ElNotification({ title: "触发器通知", message: event.TriggerFired.action_summary.slice(8) });
      }
      if (typeof event !== "string" && "TriggerActionFailed" in event) {
        ElNotification.error({ title: "触发器执行失败", message: event.TriggerActionFailed.error });
      }
    });
    if (mounted) unlistenTransfers = stop;
    else stop();
  } catch (error) {
    console.error("无法订阅传输队列", error);
    transferLoadError.value = String(error);
    ElNotification.error({
      title: "传输事件订阅失败",
      message: "传输进度与结果将不再自动刷新，请重启应用。",
    });
  }
});

onBeforeUnmount(() => {
  mounted = false;
  window.removeEventListener("resize", onViewportResize);
  unlistenTransfers?.();
  store.disposeEvents();
  hostKeyStore.disposeEvents();
  themeStore.disposeEvents();
});
</script>

<template>
  <div class="rshell-shell">
    <CustomTitleBar
      @new-session="openNewSession"
      @toggle-sidebar="toggleSidebar"
      @open-key-manager="openPanel('keys')"
      @open-theme-panel="openSettingsSubview('theme')"
      @open-plugin-panel="openSettingsSubview('plugins')"
      @open-transfer-queue="pickWorkspace('transfer'); transferPanelExpanded = true"
      @open-quick-commands="openToolSubview('quick-commands')"
      @open-triggers="openToolSubview('triggers')"
      @open-tunnels="openToolSubview('tunnels')"
    />

    <WorkspaceToolbar
      :workspace="workspace"
      :connection-state="currentConnectionState"
      :terminal-available="!!activeTerminal"
      :active-panel="activePanel"
      :sidebar-expanded="panelExpanded"
      :sync-enabled="syncEnabled"
      :transfer-panel-expanded="transferPanelExpanded"
      :can-upload="transferCapabilities.upload"
      :can-download="transferCapabilities.download"
      :can-create-folder="transferCapabilities.createFolder"
      :can-delete="transferCapabilities.delete"
      :can-refresh-files="transferCapabilities.refresh"
      :can-sync-files="transferCapabilities.sync"
      :on-new-session="openNewSession"
      :on-connect="connectCurrent"
      :on-disconnect="disconnectCurrent"
      :on-find="() => terminalAction('find')"
      :on-clear-screen="() => terminalAction('clear')"
      @change-workspace="pickWorkspace"
      @select-panel="selectPanel"
      @toggle-sidebar="toggleSidebar"
      @sync-toggle="syncEnabled = !syncEnabled"
      @upload="transferWorkspace?.upload()"
      @download="transferWorkspace?.download()"
      @new-folder="transferWorkspace?.createFolder()"
      @delete="transferWorkspace?.deleteSelected()"
      @refresh="transferWorkspace?.refresh()"
      @toggle-transfer-panel="transferPanelExpanded = !transferPanelExpanded"
    />

    <div class="body">
      <SidePanel
        :active="activePanel"
        :tool-subview="requestedToolSubview"
        :settings-subview="requestedSettingsSubview"
        :tool-subview-request="toolSubviewRequest"
        :settings-subview-request="settingsSubviewRequest"
        :active-session-id="store.currentId"
        :active-session-connected="!!store.currentId && store.connectionState.get(store.currentId) === 'connected'"
        :width="sidebarWidth"
        :max-width="sidebarMaxWidth"
        :expanded="panelExpanded"
        @update:width="setSidebarWidth"
        @select-session="selectSession"
        @new-session="openNewSession"
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
              ref="transferWorkspace"
              :session-id="activeTransferSession"
              :connected="store.connectionState.get(activeTransferSession) === 'connected'"
              :sync-enabled="syncEnabled"
              @capabilities="transferCapabilities = $event"
              @upload-queued="refreshTransfers"
              @download-queued="refreshTransfers"
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
              :items="transferItems"
              :error="transferLoadError"
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
