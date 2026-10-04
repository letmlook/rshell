<script setup lang="ts">
/**
 * TriggerEditor —— 切片 7.3
 *
 * 触发器列表和动作编辑；远端输出匹配后由后端执行动作。
 */
import { onBeforeUnmount, onMounted, ref } from "vue";
import { listTriggers, createTrigger, deleteTrigger, toggleTrigger } from "../ipc/client";
import { subscribeAppEvents } from "../ipc/events";
import type { Trigger, Uuid } from "../ipc/types";
import { confirmDialog } from "../utils/dialog";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

function triggerPattern(t: Trigger): string {
  const c = t.condition as { RegexAppear?: string; ExactMatch?: string };
  if (c.RegexAppear !== undefined) return c.RegexAppear;
  if (c.ExactMatch !== undefined) return `exact: ${c.ExactMatch}`;
  return "(none)";
}

const items = ref<Trigger[]>([]);
const newPattern = ref("");
const newText = ref("");
const loading = ref(false);
const error = ref<string | null>(null);
let unlisten: (() => void) | null = null;
let active = false;

async function refresh() {
  loading.value = true;
  try {
    items.value = await listTriggers();
    error.value = null;
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function add() {
  if (!newPattern.value || !newText.value) { error.value = "正则和发送文本均不能为空"; return; }
  try { await createTrigger({
    id: crypto.randomUUID() as Uuid,
    name: newPattern.value,
    enabled: true,
    condition: { RegexAppear: newPattern.value },
    action: { SendText: newText.value },
  });
  } catch (e) { error.value = String(e); return; }
  newPattern.value = "";
  newText.value = "";
  await refresh();
}

async function toggle(t: Trigger) {
  try { await toggleTrigger(t.id); } catch (e) { error.value = String(e); return; }
  await refresh();
}

async function remove(t: Trigger) {
  // R2-14：删除持久数据必须有确认流（与会话/密钥/快速命令/远程文件删除一致）。
  const ok = await confirmDialog({
    title: "删除触发器",
    message: `删除触发器「${t.name}」？此操作不可撤销。`,
    confirmText: "删除",
    danger: true,
  });
  if (!ok) return;
  try {
    await deleteTrigger(t.id);
  } catch (e) {
    error.value = String(e);
    return;
  }
  await refresh();
}

function actionLabel(t: Trigger): string {
  if (t.action === "Disconnect") return "disconnect";
  const a = t.action as {
    SendText?: string;
    ShowNotification?: string;
    Disconnect?: unknown;
    LogToFile?: string;
  };
  if (a.SendText !== undefined) return `send_text(${a.SendText.length} chars)`;
  if (a.ShowNotification !== undefined) return `notify: ${a.ShowNotification}`;
  if (a.LogToFile) return `log_to_file: ${a.LogToFile}`;
  return "(none)";
}

onMounted(async () => {
  active = true;
  await refresh();
  if (!active) return;
  const stop = await subscribeAppEvents((event) => { if (event === "TriggerListChanged") void refresh(); });
  if (active) unlisten = stop;
  else stop();
});
onBeforeUnmount(() => { active = false; unlisten?.(); unlisten = null; });
</script>

<template>
  <section class="trigger-editor">
    <header v-if="!props.embedded">
      <h3>触发器 ({{ items.length }})</h3>
      <el-button size="small" :loading="loading" @click="refresh">刷新</el-button>
    </header>
    <p v-if="error" class="error">{{ error }}</p>
    <el-form inline size="small" class="add-form" @submit.prevent="add">
      <el-form-item label="正则">
        <el-input v-model="newPattern" placeholder="^\\$" style="width: 120px" />
      </el-form-item>
      <el-form-item label="动作(发送文本)">
        <el-input v-model="newText" placeholder="clear" style="width: 140px" />
      </el-form-item>
      <el-form-item>
        <el-button type="primary" native-type="submit">添加</el-button>
      </el-form-item>
    </el-form>
    <el-table :data="items" stripe size="small" empty-text="暂无触发器">
      <el-table-column prop="name" label="名称" width="140" />
      <el-table-column :formatter="triggerPattern" label="正则" width="140" />
      <el-table-column :formatter="actionLabel" label="动作" />
      <el-table-column label="启用" width="80">
        <template #default="{ row }">
          <el-switch :model-value="row.enabled" @change="toggle(row)" />
        </template>
      </el-table-column>
      <el-table-column label="操作" width="80">
        <template #default="{ row }">
          <el-button size="small" type="danger" @click="remove(row)">删除</el-button>
        </template>
      </el-table-column>
    </el-table>
  </section>
</template>

<style scoped>
.trigger-editor {
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
.add-form {
  margin-bottom: 12px;
}
</style>
