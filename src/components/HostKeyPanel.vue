<script setup lang="ts">
/**
 * HostKeyPanel —— 已信任的主机密钥
 *
 * 后端 `DeleteHostKey` 早已实现，但此前界面上**没有任何入口**：known_hosts
 * 存在应用自己的数据目录里，用户不直接编辑它，主机密钥变了也没有地方清理。
 *
 * 这里是唯一的查看/删除入口：列出已信任条目，删除走应用内确认窗。
 */
import { onMounted, ref } from "vue";
import { deleteHostKey, ipcErrorMessage, listHostKeys } from "../ipc/client";
import { confirmDialog } from "../utils/dialog";
import type { HostKeyEntry } from "../ipc/types";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const entries = ref<HostKeyEntry[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);
const notice = ref<string | null>(null);

async function refresh() {
  loading.value = true;
  try {
    entries.value = await listHostKeys();
    error.value = null;
  } catch (e) {
    error.value = ipcErrorMessage(e);
  } finally {
    loading.value = false;
  }
}

/** 时间戳 → 可读形式；解析不了就如实显示原值 */
function formatSeen(value: string): string {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleString();
}

async function remove(entry: HostKeyEntry) {
  const confirmed = await confirmDialog({
    title: "删除已信任的主机密钥",
    message: `删除后，下次连接 ${entry.host}:${entry.port} 会重新弹出主机密钥确认。`,
    confirmText: "删除",
    danger: true,
  });
  if (!confirmed) return;
  try {
    await deleteHostKey(entry.host, entry.port);
    notice.value = `已删除 ${entry.host}:${entry.port} 的主机密钥`;
    error.value = null;
    await refresh();
  } catch (e) {
    error.value = ipcErrorMessage(e);
  }
}

onMounted(async () => {
  await refresh();
});
</script>

<template>
  <section class="host-key-panel">
    <h3 v-if="!props.embedded">已信任的主机密钥</h3>

    <p v-if="error" class="error" data-test="host-key-error">{{ error }}</p>
    <p v-if="notice && !error" class="notice" data-test="host-key-notice">{{ notice }}</p>

    <div class="head">
      <span class="count">{{ entries.length }} 条</span>
      <el-button size="small" :loading="loading" data-test="host-key-refresh" @click="refresh">
        刷新
      </el-button>
    </div>

    <el-empty v-if="entries.length === 0" description="没有已信任的主机密钥" :image-size="60" />
    <ul v-else class="key-list" data-test="host-key-list">
      <li v-for="entry in entries" :key="`${entry.host}:${entry.port}`">
        <div class="key-main">
          <span class="host">{{ entry.host }}:{{ entry.port }}</span>
          <span class="type">{{ entry.key_type }}</span>
          <span class="fp">{{ entry.fingerprint }}</span>
          <span class="seen">上次连接：{{ formatSeen(entry.last_seen) }}</span>
        </div>
        <el-button
          size="small"
          type="danger"
          plain
          :data-test="`host-key-delete-${entry.host}-${entry.port}`"
          @click="remove(entry)"
        >
          删除
        </el-button>
      </li>
    </ul>

    <p class="hint">
      删除只影响本应用的 known_hosts（应用数据目录内），不会改动用户 <code>~/.ssh/known_hosts</code>。
    </p>
  </section>
</template>

<style scoped>
.host-key-panel {
  padding: 12px;
}
.host-key-panel h3 {
  margin: 0 0 12px;
  font-size: 14px;
}
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 8px;
}
.count {
  font-size: 12px;
  color: var(--rs-fg-muted);
}
.key-list {
  list-style: none;
  margin: 0 0 10px;
  padding: 0;
  border: 1px solid var(--rs-border);
  border-radius: 6px;
  max-height: 240px;
  overflow-y: auto;
}
.key-list li {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border-bottom: 1px solid var(--rs-border);
}
.key-list li:last-child {
  border-bottom: 0;
}
.key-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.host {
  font-size: 12px;
}
.type {
  font-size: 10px;
  color: var(--rs-fg-muted);
}
.fp {
  font-size: 10px;
  color: var(--rs-fg-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.seen {
  font-size: 10px;
  color: var(--rs-fg-disabled);
}
.hint {
  margin: 0;
  font-size: 11px;
  line-height: 1.6;
  color: var(--rs-fg-muted);
}
.error {
  color: var(--el-color-danger);
  font-size: 12px;
}
.notice {
  color: var(--el-color-success);
  font-size: 12px;
}
</style>
