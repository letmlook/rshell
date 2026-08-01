<script setup lang="ts">
/**
 * QuickCommandPanel —— 切片 7.3
 *
 * 快速命令列表 + 执行（弹输入框 → 选目标会话 → 调 execute_quick_command）。
 */
import { onMounted, ref } from "vue";
import { listQuickCommands, executeQuickCommand } from "../ipc/client";
import type { Uuid } from "../ipc/types";

interface QuickCommand {
  id: Uuid;
  name: string;
  text: string;
  description?: string;
}

const items = ref<QuickCommand[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);
const sessionIds = ref<Uuid[]>([]);

async function refresh() {
  loading.value = true;
  error.value = null;
  try {
    items.value = (await listQuickCommands()) as unknown as QuickCommand[];
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function execute(cmd: QuickCommand) {
  if (sessionIds.value.length === 0) {
    error.value = "请先在主视图选择目标会话";
    return;
  }
  try {
    await executeQuickCommand(cmd.id, sessionIds.value);
  } catch (e) {
    error.value = String(e);
  }
}

onMounted(refresh);
</script>

<template>
  <section class="quick-commands">
    <header>
      <h3>快速命令 ({{ items.length }})</h3>
      <el-button size="small" :loading="loading" @click="refresh">刷新</el-button>
    </header>
    <p v-if="error" class="error">{{ error }}</p>
    <el-empty v-if="items.length === 0" description="暂无快速命令" />
    <el-table v-else :data="items" stripe size="small">
      <el-table-column prop="name" label="名称" />
      <el-table-column prop="text" label="命令" />
      <el-table-column label="操作" width="80">
        <template #default="{ row }">
          <el-button size="small" type="primary" @click="execute(row)">执行</el-button>
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
</style>