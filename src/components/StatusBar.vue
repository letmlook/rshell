<script setup lang="ts">
/**
 * StatusBar —— v2 重设计
 *
 * 窗口底部状态条，只显示当前会话可验证的状态。
 */
import { computed } from "vue";
import { useSessionsStore } from "../stores/sessions";

const props = defineProps<{
  workspace: "terminal" | "transfer";
}>();

const store = useSessionsStore();

const statusText = computed(() => {
  const id = store.currentId;
  if (!id) return "disconnected";
  return store.connectionState.get(id) ?? "disconnected";
});

const statusLabel = computed(() => {
  const s = statusText.value;
  if (s === "connected") return "已连接";
  if (s === "connecting") return "连接中";
  if (s === "failed") return "失败";
  return "未连接";
});

// SFTP 仅对 SSH 会话可用:传输工作区只在当前会话为 SSH 时显示 SFTP,
// 无会话或 Telnet/Serial 会话显示 "—"(PROB-24);终端工作区显示会话真实协议。
const protocolLabel = computed(() => {
  if (props.workspace === "transfer") {
    return store.current?.protocol === "SSH" ? "SFTP" : "—";
  }
  return store.current?.protocol ?? "—";
});
</script>

<template>
  <footer class="status-bar">
    <span class="status-item">
      <span class="rs-status-dot" :class="`rs-status-dot--${statusText}`" />
      {{ statusLabel }}
    </span>
    <span class="status-sep" aria-hidden="true">·</span>
    <span class="status-item">{{ store.current?.name || "—" }}</span>
    <span class="status-sep" aria-hidden="true">·</span>
    <span class="status-item">{{ protocolLabel }}</span>
    <span class="status-sep" aria-hidden="true">·</span>
    <span class="spacer" />
    <span class="status-item muted">RShell v0.1.0</span>
  </footer>
</template>

<style scoped>
.status-bar {
  height: var(--rs-statusbar-h);
  display: flex;
  align-items: center;
  gap: var(--rs-s-2);
  padding: 0 var(--rs-s-3);
  background: var(--rs-statusbar-bg, var(--rs-bg));
  border-top: 1px solid var(--rs-border);
  color: var(--rs-fg-muted);
  font-size: var(--rs-fs-xs);
  font-family: var(--rs-font-display);
  flex-shrink: 0;
}
.status-item {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.status-sep {
  opacity: 0.4;
  margin: 0 2px;
}
.spacer { flex: 1; }
.muted { color: var(--rs-fg-disabled); }
.rs-status-dot {
  width: 7px;
  height: 7px;
}
</style>
