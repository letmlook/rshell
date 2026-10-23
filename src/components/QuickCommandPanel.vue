<script setup lang="ts">
/**
 * QuickCommandPanel —— 切片 7.3
 *
 * 快速命令列表 + 执行（弹输入框 → 选目标会话 → 调 execute_quick_command）。
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { createQuickCommand, deleteQuickCommand, listQuickCommands, executeQuickCommand } from "../ipc/client";
import { subscribeAppEvents } from "../ipc/events";
import { useSessionsStore } from "../stores/sessions";
import type { QuickCommand } from "../ipc/types";
import { confirmDialog } from "../utils/dialog";
import { formatHotkey, parseHotkey } from "../utils/hotkey";
import { resolveQuickCommandTargets } from "../utils/quickCommand";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const items = ref<QuickCommand[]>([]);

/**
 * 按「分组」聚合。
 *
 * 后端 `QuickCommand.group` 此前恒为 null（字段存在但没有任何界面能设置），
 * 现在分组名由用户填写：同名的命令聚在一起，没填的归到「未分组」。
 */
interface CommandGroup {
  label: string;
  commands: QuickCommand[];
}

const groups = computed<CommandGroup[]>(() => {
  const map = new Map<string, QuickCommand[]>();
  for (const cmd of items.value) {
    const label = cmd.group?.trim() || "未分组";
    if (!map.has(label)) map.set(label, []);
    map.get(label)!.push(cmd);
  }
  return Array.from(map.entries()).map(([label, commands]) => ({ label, commands }));
});
const loading = ref(false);
const error = ref<string | null>(null);
const sessions = useSessionsStore();
const newName = ref("");
const newCommand = ref("");
const sendEnter = ref(true);
const newScope = ref<"CurrentSession" | "AllSessions" | "SelectedSessions">("CurrentSession");
/** 「选定会话」目标：勾选的会话 id */
const newScopeSessions = ref<string[]>([]);
/** 热键：形如 Ctrl+Shift+1；留空表示不绑定 */
const newHotkey = ref("");
/** 说明：这条命令做什么（后端字段此前恒为空串） */
const newDescription = ref("");
/** 分组名：留空表示不分组（后端字段此前恒为 null） */
const newGroup = ref("");
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
  const targets = resolveQuickCommandTargets(cmd, {
    items: sessions.items,
    currentId: sessions.currentId,
    connectionState: sessions.connectionState,
  });
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
  if (newScope.value === "SelectedSessions" && newScopeSessions.value.length === 0) {
    error.value = "请至少选择一个目标会话";
    return;
  }
  // 热键写错时当场说明，不静默存一个永远不触发的绑定
  const hotkey = newHotkey.value.trim();
  if (hotkey && !parseHotkey(hotkey)) {
    error.value = `热键写法无法识别：「${hotkey}」（例：Ctrl+Shift+1）`;
    return;
  }
  // 后端 QuickCommandScope 有三种变体，「选定会话」此前界面无法创建。
  const scope: QuickCommand["scope"] =
    newScope.value === "SelectedSessions"
      ? { SelectedSessions: newScopeSessions.value }
      : newScope.value;
  try {
    await createQuickCommand({
      id: crypto.randomUUID(), name: newName.value.trim(), command: newCommand.value,
      send_enter: sendEnter.value,
      description: newDescription.value.trim(),
      scope,
      hotkey: hotkey || null,
      group: newGroup.value.trim() || null,
    });
    newName.value = "";
    newCommand.value = "";
    newScopeSessions.value = [];
    newHotkey.value = "";
    newDescription.value = "";
    newGroup.value = "";
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
      <el-select v-model="newScope" aria-label="命令目标" data-test="quick-command-scope">
        <el-option label="当前会话" value="CurrentSession" />
        <el-option label="所有已连接会话" value="AllSessions" />
        <el-option label="选定会话" value="SelectedSessions" />
      </el-select>
      <el-select
        v-if="newScope === 'SelectedSessions'"
        v-model="newScopeSessions"
        multiple
        placeholder="选择目标会话"
        aria-label="选定会话"
        style="width: 180px"
        data-test="quick-command-targets"
      >
        <el-option
          v-for="session in sessions.items"
          :key="session.id"
          :label="session.name"
          :value="session.id"
        />
      </el-select>
      <el-checkbox v-model="sendEnter">发送回车</el-checkbox>
      <el-input
        v-model="newHotkey"
        placeholder="热键（例 Ctrl+Shift+1）"
        aria-label="快速命令热键"
        style="width: 170px"
        data-test="quick-command-hotkey"
      />
      <el-input
        v-model="newGroup"
        placeholder="分组（可留空）"
        aria-label="快速命令分组"
        style="width: 130px"
        data-test="quick-command-group"
      />
      <el-input
        v-model="newDescription"
        placeholder="说明（可留空）"
        aria-label="快速命令说明"
        style="width: 150px"
        data-test="quick-command-description"
      />
      <el-button type="primary" @click="create">添加</el-button>
    </div>
    <el-empty v-if="items.length === 0" description="暂无快速命令" :image-size="60" />
    <div v-else class="groups" data-test="quick-command-groups">
      <section v-for="group in groups" :key="group.label" class="group">
        <h4 v-if="group.label !== '未分组'">{{ group.label }}</h4>
        <el-table :data="group.commands" stripe size="small">
          <el-table-column prop="name" label="名称" />
          <el-table-column prop="command" label="命令" />
          <el-table-column label="说明" min-width="120">
            <template #default="{ row }">
              <span :title="row.description">{{ row.description || "—" }}</span>
            </template>
          </el-table-column>
          <el-table-column label="热键" width="130">
            <template #default="{ row }">
              <span v-if="row.hotkey">{{ formatHotkey(parseHotkey(row.hotkey)!) }}</span>
              <span v-else>—</span>
            </template>
          </el-table-column>
          <el-table-column label="操作" width="140">
            <template #default="{ row }">
              <el-button size="small" type="primary" @click="execute(row)">执行</el-button>
              <el-button size="small" type="danger" text @click="remove(row)">删除</el-button>
            </template>
          </el-table-column>
        </el-table>
      </section>
    </div>
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
.groups .group {
  margin-bottom: 12px;
}
.groups .group h4 {
  margin: 0 0 4px;
  font-size: 12px;
  color: var(--rs-fg-muted);
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
