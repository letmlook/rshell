<script setup lang="ts">
/**
 * SyncInputPanel —— 同步输入
 *
 * 开启后，用户在**任意**终端里键入的内容会同时发到选定的一组会话，
 * 用于在多台主机上重复执行同一条命令。
 *
 * 语义要点：
 * - 源会话不会收到双份字节（后端在广播时排除源会话）。
 * - 某个目标发送失败会发布 `SyncInputSendFailed` 事件，界面据此提示，
 *   而不是静默吞掉。
 * - 同步会话组只存在于后端内存中：重启应用后自动关闭。
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { getSyncInputSessions, toggleSyncInput } from "../ipc/client";
import { ipcErrorMessage } from "../ipc/client";
import { subscribeAppEvents } from "../ipc/events";
import { useSessionsStore } from "../stores/sessions";
import type { Uuid } from "../ipc/types";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const sessions = useSessionsStore();

const selected = ref<Uuid[]>([]);
const active = ref<Uuid[]>([]);
const error = ref<string | null>(null);
const loading = ref(false);
const notice = ref<string | null>(null);

let unlisten: (() => void) | null = null;
let activeFlag = false;

const isActive = computed(() => active.value.length > 0);

function nameOf(id: Uuid): string {
  const found = sessions.items.find((s) => s.id === id);
  return found ? found.name : id.slice(0, 8);
}

function stateOf(id: Uuid): string {
  return sessions.connectionState.get(id) ?? "disconnected";
}

async function refresh() {
  loading.value = true;
  try {
    active.value = await getSyncInputSessions();
    error.value = null;
  } catch (e) {
    error.value = ipcErrorMessage(e);
  } finally {
    loading.value = false;
  }
}

async function enable() {
  if (selected.value.length === 0) {
    error.value = "请至少选择一个会话";
    return;
  }
  loading.value = true;
  try {
    await toggleSyncInput(selected.value);
    active.value = await getSyncInputSessions();
    error.value = null;
    notice.value = `同步输入已开启：${active.value.length} 个会话`;
  } catch (e) {
    error.value = ipcErrorMessage(e);
  } finally {
    loading.value = false;
  }
}

async function disable() {
  loading.value = true;
  try {
    await toggleSyncInput([]);
    active.value = [];
    selected.value = [];
    error.value = null;
    notice.value = "同步输入已关闭";
  } catch (e) {
    error.value = ipcErrorMessage(e);
  } finally {
    loading.value = false;
  }
}

onMounted(async () => {
  activeFlag = true;
  await sessions.refresh();
  await refresh();
  if (!activeFlag) return;
  const stop = await subscribeAppEvents((event) => {
    if (typeof event === "string") return;
    if ("SyncInputSessionsChanged" in event) {
      active.value = event.SyncInputSessionsChanged.session_ids;
    } else if ("SyncInputSendFailed" in event) {
      const { session_id, error: reason } = event.SyncInputSendFailed;
      error.value = `同步到会话「${nameOf(session_id)}」失败：${reason}`;
    }
  });
  if (activeFlag) unlisten = stop;
  else stop();
});

onBeforeUnmount(() => {
  activeFlag = false;
  unlisten?.();
  unlisten = null;
});
</script>

<template>
  <section class="sync-panel">
    <h3 v-if="!props.embedded">同步输入</h3>

    <p v-if="error" class="error" data-test="sync-error">{{ error }}</p>
    <p v-if="notice && !error" class="notice" data-test="sync-notice">{{ notice }}</p>

    <p v-if="isActive" class="state" data-test="sync-active">
      同步输入已开启，当前同步 {{ active.length }} 个会话：{{ active.map(nameOf).join("、") }}
    </p>
    <p v-else class="state off" data-test="sync-inactive">同步输入未开启</p>

    <ul class="session-list">
      <li v-for="session in sessions.items" :key="session.id">
        <label>
          <input
            type="checkbox"
            :value="session.id"
            :checked="selected.includes(session.id)"
            :data-test="`sync-session-${session.id}`"
            @change="(e) => {
              const checked = (e.target as HTMLInputElement).checked;
              selected = checked
                ? [...selected, session.id]
                : selected.filter((id) => id !== session.id);
            }"
          />
          <span class="name">{{ session.name }}</span>
          <span class="meta">{{ session.host }}:{{ session.port }}</span>
          <span class="state-chip" :class="stateOf(session.id)">{{ stateOf(session.id) }}</span>
        </label>
      </li>
    </ul>
    <el-empty v-if="sessions.items.length === 0" description="没有可同步的会话" :image-size="60" />

    <div class="actions">
      <el-button
        type="primary"
        :loading="loading"
        :disabled="selected.length === 0"
        data-test="sync-enable"
        @click="enable"
      >
        开启同步输入
      </el-button>
      <el-button
        :loading="loading"
        :disabled="!isActive"
        data-test="sync-disable"
        @click="disable"
      >
        关闭同步输入
      </el-button>
    </div>

    <p class="hint">
      开启后，在任意终端键入的内容会同时发到上面勾选的会话；当前正在输入的会话不会收到重复内容。
      同步会话组只保存在内存中，重启应用后自动关闭。
    </p>
  </section>
</template>

<style scoped>
.sync-panel {
  padding: 12px;
}
.sync-panel h3 {
  margin: 0 0 12px;
  font-size: 14px;
}
.state {
  margin: 0 0 10px;
  font-size: 12px;
  color: var(--rs-fg);
}
.state.off {
  color: var(--rs-fg-muted);
}
.session-list {
  list-style: none;
  margin: 0 0 12px;
  padding: 0;
  max-height: 260px;
  overflow-y: auto;
  border: 1px solid var(--rs-border);
  border-radius: 6px;
}
.session-list li {
  border-bottom: 1px solid var(--rs-border);
}
.session-list li:last-child {
  border-bottom: 0;
}
.session-list label {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  font-size: 12px;
  cursor: pointer;
}
.session-list .name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.session-list .meta {
  color: var(--rs-fg-muted);
  font-size: 11px;
}
.state-chip {
  font-size: 10px;
  padding: 1px 6px;
  border-radius: 8px;
  background: var(--rs-bg-surface-hover);
  color: var(--rs-fg-muted);
}
.state-chip.connected {
  color: var(--el-color-success);
}
.actions {
  display: flex;
  gap: 8px;
  margin-bottom: 10px;
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
