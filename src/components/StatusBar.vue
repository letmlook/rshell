<script setup lang="ts">
/**
 * StatusBar —— 切片 10
 *
 * 窗口底部状态条:连接状态 + 当前会话 + 时钟(占位)。
 */
import { computed } from "vue";
import { useSessionsStore } from "../stores/sessions";

const store = useSessionsStore();

const statusText = computed(() => {
  const currentId = store.currentId;
  if (!currentId) return "未连接";
  return store.connectionState.get(currentId) ?? "disconnected";
});
</script>

<template>
  <footer class="status-bar">
    <span class="status-item">
      <span class="status-dot" :class="`status-${statusText.toLowerCase()}`" />
      {{ statusText }}
    </span>
    <span class="status-spacer" />
    <span class="status-item muted">RShell v0.1.0</span>
  </footer>
</template>

<style scoped>
.status-bar {
  height: 24px;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 0 12px;
  background: var(--rshell-statusbar-bg, #181b20);
  border-top: 1px solid var(--rshell-border, #2c313a);
  color: var(--rshell-fg-muted, #9097a3);
  font-size: 11px;
}
.status-item {
  display: flex;
  align-items: center;
  gap: 6px;
}
.status-spacer {
  flex: 1;
}
.status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #6a7180;
}
.status-dot.status-connected {
  background: #67c23a;
}
.status-dot.status-connecting {
  background: #e6a23c;
}
.status-dot.status-failed {
  background: #f56c6c;
}
.status-dot.status-disconnected {
  background: #6a7180;
}
.muted {
  color: #6a7180;
}
</style>