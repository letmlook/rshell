<script setup lang="ts">
/**
 * RShell 主布局 —— 切片 10 全新设计
 *
 * 结构(对应 tauri.conf.json decorations: false):
 *   ┌─────────────────────────────────────────────┐
 *   │ CustomTitleBar  (36px, 含拖动区 + 窗口控制)  │
 *   ├──────┬─────────────┬──────────────────────────┤
 *   │Activ-│ SidePanel   │ main-area               │
 *   │ityBar│ (二级面板,  │  - Dockview 多终端       │
 *   │ (220)│ 可折叠)     │  - 状态栏                │
 *   ├──────┴─────────────┴──────────────────────────┤
 *   │ StatusBar (24px)                             │
 *   └─────────────────────────────────────────────┘
 *
 * 浮层:HostKeyMismatchDialog / TransferQueue /
 *       MasterPasswordDialog / SessionCreateDialog
 */
import { onMounted, ref, markRaw } from "vue";
import { DockviewVue } from "dockview-vue";
import "dockview-vue/dist/styles/dockview.css";
import TerminalPane from "./components/TerminalPane.vue";
import SessionCreateDialog from "./components/SessionCreateDialog.vue";
import HostKeyMismatchDialog from "./components/HostKeyMismatchDialog.vue";
import TransferQueue from "./components/TransferQueue.vue";
import MasterPasswordDialog from "./components/MasterPasswordDialog.vue";
import CustomTitleBar from "./components/CustomTitleBar.vue";
import ActivityBar from "./components/ActivityBar.vue";
import SidePanel from "./components/SidePanel.vue";
import StatusBar from "./components/StatusBar.vue";
import { useSessionsStore } from "./stores/sessions";
import { useHostKeyStore } from "./stores/hostKey";
import type { Uuid } from "./ipc/types";

const store = useSessionsStore();
const hostKeyStore = useHostKeyStore();
const dialogVisible = ref(false);
const activeTerminal = ref<Uuid | null>(null);
const activePanel = ref("sessions");
const panelExpanded = ref(true);

const components = markRaw({ TerminalPane: TerminalPane as never });

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

function openPanel(name: string) {
  activePanel.value = name;
  panelExpanded.value = true;
}

function toggleSidebar() {
  panelExpanded.value = !panelExpanded.value;
}

onMounted(async () => {
  await store.refresh();
  await store.subscribeEvents();
  await hostKeyStore.subscribeEvents();
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

    <div class="body">
      <ActivityBar
        v-model:active="activePanel"
        v-model:expanded="panelExpanded"
      />
      <SidePanel
        v-if="panelExpanded"
        :active="activePanel"
        @select-session="selectSession"
      />

      <main class="main-area">
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
      </main>
    </div>

    <StatusBar />

    <SessionCreateDialog
      :visible="dialogVisible"
      @close="dialogVisible = false"
      @created="(id) => selectSession(id)"
    />
    <HostKeyMismatchDialog />
    <TransferQueue />
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
  background: var(--rshell-bg, #1e2228);
  color: var(--rshell-fg, #e6e6e6);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", sans-serif;
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
  background: var(--rshell-bg, #1e2228);
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

.empty {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  margin: 0;
  padding: 0;
}
.empty-content {
  text-align: center;
  color: var(--rshell-fg-muted, #9097a3);
}
.empty-icon {
  font-size: 64px;
  color: var(--el-color-primary);
  opacity: 0.6;
  margin-bottom: 16px;
}
.empty-content h2 {
  margin: 0 0 8px;
  font-size: 20px;
  font-weight: 500;
  color: var(--rshell-fg, #e6e6e6);
}
.empty-content p {
  margin: 0 0 20px;
  font-size: 13px;
}
</style>