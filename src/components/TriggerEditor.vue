<script setup lang="ts">
/**
 * TriggerEditor —— 切片 7.3
 *
 * 触发器列表 + 正则/动作编辑。SendText 触发器的远端真发已在切片 7.1 标为
 * 已知缺口（!Send 障碍），本切片 UI 仅展示 + 配置元数据。
 */
import { onMounted, ref } from "vue";
import { listTriggers, createTrigger, deleteTrigger, toggleTrigger } from "../ipc/client";
import type { Trigger, Uuid } from "../ipc/types";

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

async function refresh() {
  loading.value = true;
  try {
    items.value = (await listTriggers()) as unknown as Trigger[];
  } finally {
    loading.value = false;
  }
}

async function add() {
  if (!newPattern.value) return;
  await createTrigger({
    id: crypto.randomUUID() as Uuid,
    name: newPattern.value,
    enabled: true,
    condition: { RegexAppear: newPattern.value },
    action: { SendText: newText.value },
  } as unknown as Trigger);
  newPattern.value = "";
  newText.value = "";
  await refresh();
}

async function toggle(t: Trigger) {
  await toggleTrigger(t.id);
  await refresh();
}

async function remove(t: Trigger) {
  await deleteTrigger(t.id);
  await refresh();
}

function actionLabel(t: Trigger): string {
  const a = t.action as {
    SendText?: string;
    ShowNotification?: string;
    Disconnect?: unknown;
    LogToFile?: string;
  };
  if (a.SendText !== undefined) return `send_text(${a.SendText.length} chars)`;
  if (a.ShowNotification !== undefined) return `notify: ${a.ShowNotification}`;
  if (a.Disconnect) return "disconnect";
  if (a.LogToFile) return `log_to_file: ${a.LogToFile}`;
  return "(none)";
}

onMounted(refresh);
</script>

<template>
  <section class="trigger-editor">
    <header v-if="!props.embedded">
      <h3>触发器 ({{ items.length }})</h3>
      <el-button size="small" :loading="loading" @click="refresh">刷新</el-button>
    </header>
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