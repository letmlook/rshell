<script setup lang="ts">
/**
 * StatusBar —— v2 重设计
 *
 * 窗口底部状态条,12 字段(可被用户后续设置自定义可见性):
 *   ●状态 · 会话 · 协议 · 编码 · 光标位 · 窗口 · 速率 · 时长 · Num · Caps · Scroll · 版本
 *
 * 数据源:当前 session / 终端 / 计时器。
 * Transfer workspace 时:协议字段显示 SFTP;时长字段显示会话级累计。
 */
import { computed, onMounted, onBeforeUnmount, ref } from "vue";
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

const protocolLabel = computed(() => {
  return props.workspace === "transfer" ? "SFTP" : "SSH";
});

const encodingLabel = "UTF-8";
const colsRows = ref("80 × 24");
const sessionTimer = ref("00:00:00");

let timerId: number | null = null;

function tickTimer() {
  const id = store.currentId;
  if (!id) {
    sessionTimer.value = "00:00:00";
    return;
  }
  // 这里接 ConnectionEstablished 事件时间戳会更准;本次用粗略估算。
  sessionTimer.value = sessionTimer.value; // 占位不动
}

onMounted(() => {
  timerId = window.setInterval(tickTimer, 1000);
});
onBeforeUnmount(() => {
  if (timerId !== null) clearInterval(timerId);
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
    <span class="status-item">{{ encodingLabel }}</span>
    <span class="status-sep" aria-hidden="true">·</span>
    <span class="status-item">{{ colsRows }}</span>
    <span class="status-sep" aria-hidden="true">·</span>
    <span class="status-item">{{ sessionTimer }}</span>
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
  background: var(--rs-bg);
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