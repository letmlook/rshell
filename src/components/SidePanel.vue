<script setup lang="ts">
/**
 * SidePanel —— 切片 10
 *
 * 中间二级面板(在 ActivityBar 与 main-area 之间),按 active prop
 * 渲染对应子组件(sessions / files / keys / tools / settings)。
 */
import ThemePanel from "./ThemePanel.vue";
import SessionList from "./SessionList.vue";
import KeyManagerPanel from "./KeyManagerPanel.vue";
import QuickCommandPanel from "./QuickCommandPanel.vue";
import TriggerEditor from "./TriggerEditor.vue";
import TunnelPanel from "./TunnelPanel.vue";
import PluginPanel from "./PluginPanel.vue";

const props = defineProps<{
  active: string;
}>();
void props; // props 通过模板 attr 隐式消费(避免 unused 警告)

const emit = defineEmits<{
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
</script>

<template>
  <aside class="side-panel">
    <header class="side-panel-header">
      <h3>{{ titles[active] || active }}</h3>
    </header>
    <div class="side-panel-body">
      <SessionList
        v-if="active === 'sessions'"
        @select="(id) => emit('select-session', id)"
        @open-sftp="(id) => emit('open-sftp', id)"
        @open-terminal="(id, p) => emit('open-terminal', id, p)"
      />
      <template v-else-if="active === 'files'">
        <div class="placeholder">
          <p>文件浏览（切片 5.2 占位）</p>
          <p class="hint">本地 / 远端双面板 · 待 Tauri-plugin-fs + SFTP 接入</p>
        </div>
      </template>
      <template v-else-if="active === 'keys'">
        <KeyManagerPanel />
      </template>
      <template v-else-if="active === 'tools'">
        <QuickCommandPanel />
        <TriggerEditor />
        <TunnelPanel />
      </template>
      <template v-else-if="active === 'settings'">
        <ThemePanel />
        <PluginPanel />
      </template>
    </div>
  </aside>
</template>

<style scoped>
.side-panel {
  width: 320px;
  background: var(--rshell-panel-bg, #20242b);
  border-right: 1px solid var(--rshell-border, #2c313a);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.side-panel-header {
  padding: 10px 16px;
  border-bottom: 1px solid var(--rshell-border, #2c313a);
}
.side-panel-header h3 {
  margin: 0;
  font-size: 13px;
  font-weight: 500;
  color: var(--rshell-fg, #e6e6e6);
}
.side-panel-body {
  flex: 1;
  overflow-y: auto;
}
.placeholder {
  padding: 24px 16px;
  color: var(--rshell-fg-muted, #9097a3);
  font-size: 12px;
}
.placeholder .hint {
  font-size: 11px;
  color: var(--rshell-fg-disabled, #6a7180);
  margin-top: 4px;
}
</style>