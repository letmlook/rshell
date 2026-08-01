<script setup lang="ts">
/**
 * RShell 主布局 —— 切片 1.3
 *
 * dockview-vue 容器承载两个面板:
 * - 左侧 session-list:Element Plus 列表 + 新建按钮
 * - 中央 terminal-pane 区域:每个 session 一个面板,选中后渲染 TerminalPane
 *
 * 切片 1.3 目标:点 session → connect → TerminalPane 挂载,Channel 接住字节。
 * 完整 dock 拖拽/布局序列化 (设计 §9 #1) 由 dockview-vue 自带能力承载。
 */
import { onMounted, ref, computed, markRaw } from "vue";
import { DockviewVue } from "dockview-vue";
import "dockview-vue/dist/styles/dockview.css";
import TerminalPane from "./components/TerminalPane.vue";
import SessionCreateDialog from "./components/SessionCreateDialog.vue";
import SessionList from "./components/SessionList.vue";
import ThemePanel from "./components/ThemePanel.vue";
import HostKeyMismatchDialog from "./components/HostKeyMismatchDialog.vue";
import TransferQueue from "./components/TransferQueue.vue";
import KeyManagerPanel from "./components/KeyManagerPanel.vue";
import MasterPasswordDialog from "./components/MasterPasswordDialog.vue";
import QuickCommandPanel from "./components/QuickCommandPanel.vue";
import TriggerEditor from "./components/TriggerEditor.vue";
import TunnelPanel from "./components/TunnelPanel.vue";
import PluginPanel from "./components/PluginPanel.vue";
import { useSessionsStore } from "./stores/sessions";
import { useHostKeyStore } from "./stores/hostKey";
import type { Uuid } from "./ipc/types";

const store = useSessionsStore();
const hostKeyStore = useHostKeyStore();
const dialogVisible = ref(false);
const activeTerminal = ref<Uuid | null>(null);

// markRaw 避免 Vue 把面板组件做成 reactive proxy —— dockview-vue 要求
// 组件引用稳定,代理会破坏面板 id 序列化（设计 §4.2 "布局 JSON 持久化"）。
// VueComponent 是 ComponentPublicInstanceConstructor,<script setup> 推导的
// DefineComponent 形态不直接匹配 —— 安全 cast 到宽类型。
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

const connectionLabel = computed(() => {
  if (!activeTerminal.value) return "未连接";
  const s = store.connectionState.get(activeTerminal.value) ?? "disconnected";
  return s;
});

onMounted(async () => {
  await store.refresh();
  await store.subscribeEvents();
  await hostKeyStore.subscribeEvents();
});
</script>

<template>
  <main class="rshell-shell">
    <header class="topbar">
      <h1>RShell</h1>
      <el-button type="primary" size="small" @click="dialogVisible = true">
        新建会话
      </el-button>
      <span class="status">当前: {{ connectionLabel }}</span>
    </header>

    <div class="body">
      <aside class="sidebar">
        <ThemePanel />
        <SessionList @select="selectSession" />
      </aside>

      <section class="main-area">
        <DockviewVue
          v-if="activeTerminal"
          :components="components"
          style="width: 100%; height: 100%"
        >
          <template #terminal="{ params }">
            <TerminalPane :session-id="params.sessionId" />
          </template>
        </DockviewVue>
        <div v-else class="empty">选中左侧会话开始</div>
      </section>
    </div>

    <SessionCreateDialog
      :visible="dialogVisible"
      @close="dialogVisible = false"
      @created="(id) => selectSession(id)"
    />

    <HostKeyMismatchDialog />
    <TransferQueue />
    <MasterPasswordDialog />
    <KeyManagerPanel />
    <QuickCommandPanel />
    <TriggerEditor />
    <TunnelPanel />
    <PluginPanel />
  </main>
</template>

<style scoped>
.rshell-shell {
  display: flex;
  flex-direction: column;
  height: 100vh;
  margin: 0;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
.topbar {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 8px 16px;
  border-bottom: 1px solid var(--el-border-color);
  background: var(--el-bg-color-page);
}
.topbar h1 {
  font-size: 18px;
  margin: 0;
}
.status {
  margin-left: auto;
  color: var(--el-text-color-secondary);
  font-size: 12px;
}
.body {
  flex: 1;
  display: flex;
  min-height: 0;
}
.sidebar {
  width: 240px;
  border-right: 1px solid var(--el-border-color);
  padding: 12px;
  overflow-y: auto;
}
.session-list {
  list-style: none;
  padding: 0;
  margin: 8px 0;
}
.session-list li {
  padding: 8px;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 4px;
  margin-bottom: 6px;
  cursor: pointer;
}
.session-list li:hover {
  background: var(--el-fill-color-light);
}
.session-list li.active {
  border-color: var(--el-color-primary);
  background: var(--el-color-primary-light-9);
}
.session-list .name {
  font-weight: 500;
}
.session-list .meta {
  color: var(--el-text-color-secondary);
  font-size: 12px;
}
.session-list .state {
  font-size: 11px;
  color: var(--el-text-color-placeholder);
  text-transform: uppercase;
}
.main-area {
  flex: 1;
  position: relative;
  background: #1e1e1e;
}
.empty {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: var(--el-text-color-secondary);
}
.error {
  color: var(--el-color-danger);
  font-size: 12px;
}
</style>