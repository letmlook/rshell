<script setup lang="ts">
/**
 * QuickCommandPanel —— 切片 7.3
 *
 * 快速命令列表 + 执行（弹输入框 → 选目标会话 → 调 execute_quick_command）。
 */
import { onBeforeUnmount, onMounted, ref } from "vue";
import { createQuickCommand, deleteQuickCommand, listQuickCommands, executeQuickCommand } from "../ipc/client";
import { subscribeAppEvents } from "../ipc/events";
import { useSessionsStore } from "../stores/sessions";
import type { QuickCommand, Uuid } from "../ipc/types";
import { confirmDialog } from "../utils/dialog";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const items = ref<QuickCommand[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);
const sessions = useSessionsStore();
const newName = ref("");
const newCommand = ref("");
const sendEnter = ref(true);
const newScope = ref<"CurrentSession" | "AllSessions">("CurrentSession");
let unlisten: (() => void) | null = null;
let active = false;

async function refresh() {
  loading.value = true;
  error.value = null;
  try {
    items.value = await listQuickCommands();
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function execute(cmd: QuickCommand) {
  const connected = sessions.items.filter((item) => sessions.connectionState.get(item.id) === "connected").map((item) => item.id);
  let targets: Uuid[];
  if (cmd.scope === "CurrentSession") targets = sessions.currentId && connected.includes(sessions.currentId) ? [sessions.currentId] : [];
  else if (cmd.scope === "AllSessions") targets = connected;
  else targets = cmd.scope.SelectedSessions.filter((id) => connected.includes(id));
  if (targets.length === 0) {
    error.value = "没有可用的已连接目标会话";
    return;
  }
  try {
    await executeQuickCommand(cmd.id, targets);
  } catch (e) {
    error.value = String(e);
  }
}

async function create() {
  if (!newName.value.trim() || !newCommand.value.trim()) {
    error.value = "请输入名称和命令";
    return;
  }
  try {
    await createQuickCommand({
      id: crypto.randomUUID(), name: newName.value.trim(), command: newCommand.value,
      send_enter: sendEnter.value, description: "", scope: newScope.value,
      hotkey: null, group: null,
    });
    newName.value = "";
    newCommand.value = "";
    await refresh();
  } catch (e) { error.value = String(e); }
}

async function remove(cmd: QuickCommand) {
  const ok = await confirmDialog({
    title: "删除快速命令",
    message: `删除快速命令「${cmd.name}」？此操作不可撤销。`,
    confirmText: "删除",
    danger: true,
  });
  if (!ok) return;
  try {
    await deleteQuickCommand(cmd.id);
    await refresh();
  } catch (e) {
    error.value = String(e);
  }
}

onMounted(async () => {
  active = true;
  await refresh();
  if (!active) return;
  const stop = await subscribeAppEvents((event) => { if (event === "QuickCommandListChanged") void refresh(); });
  if (active) unlisten = stop;
  else stop();
});
onBeforeUnmount(() => { active = false; unlisten?.(); unlisten = null; });
</script>

<template>
  <section class="quick-commands">
    <header v-if="!props.embedded">
      <h3>快速命令 ({{ items.length }})</h3>
      <el-button size="small" :loading="loading" @click="refresh">刷新</el-button>
    </header>
    <p v-if="error" class="error">{{ error }}</p>
    <div class="creator">
      <el-input v-model="newName" placeholder="名称" aria-label="快速命令名称" />
      <el-input v-model="newCommand" placeholder="命令" aria-label="快速命令内容" />
      <el-select v-model="newScope" aria-label="命令目标">
        <el-option label="当前会话" value="CurrentSession" />
        <el-option label="所有已连接会话" value="AllSessions" />
      </el-select>
      <el-checkbox v-model="sendEnter">发送回车</el-checkbox>
      <el-button type="primary" @click="create">添加</el-button>
    </div>
    <el-empty v-if="items.length === 0" description="暂无快速命令" />
    <el-table v-else :data="items" stripe size="small">
      <el-table-column prop="name" label="名称" />
      <el-table-column prop="command" label="命令" />
      <el-table-column label="操作" width="140">
        <template #default="{ row }">
          <el-button size="small" type="primary" @click="execute(row)">执行</el-button>
          <el-button size="small" type="danger" text @click="remove(row)">删除</el-button>
        </template>
      </el-table-column>
    </el-table>
  </section>
</template>

<style scoped>
.quick-commands {
  padding: 12px;
}
header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
}
h3 {
  margin: 0;
  font-size: 14px;
}
.error {
  color: var(--el-color-danger);
  font-size: 12px;
}
.creator {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 12px;
}
.creator .el-input { width: 150px; }
.creator .el-select { width: 150px; }
</style>
