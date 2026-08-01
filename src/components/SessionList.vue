<script setup lang="ts">
/**
 * SessionList —— 切片 3
 *
 * Element Plus el-table,排序/过滤/展开折叠 由 pinia store 管（设计 §5）。
 * 会话项点击 → emit('select', id) → 父组件触发 connect_session + 显示 TerminalPane。
 */
import { computed } from "vue";
import { useSessionsStore } from "../stores/sessions";
import type { Uuid } from "../ipc/types";

const emit = defineEmits<{ (e: "select", id: Uuid): void }>();

const store = useSessionsStore();

const filtered = computed(() => {
  const kw = store.searchKeyword.trim().toLowerCase();
  if (!kw) return store.items;
  return store.items.filter(
    (s) => s.name.toLowerCase().includes(kw) || s.host.toLowerCase().includes(kw),
  );
});

function stateLabel(id: Uuid) {
  return store.connectionState.get(id) ?? "disconnected";
}

function onRowClick(row: { id: Uuid }) {
  emit("select", row.id);
}
</script>

<template>
  <div class="session-list">
    <div class="header">
      <h3>会话 ({{ filtered.length }}/{{ store.items.length }})</h3>
      <el-button size="small" @click="store.refresh()" :loading="store.loading">刷新</el-button>
    </div>
    <el-input
      v-model="store.searchKeyword"
      size="small"
      placeholder="按名称 / 主机过滤"
      clearable
    />
    <p v-if="store.error" class="error">{{ store.error }}</p>
    <el-table
      :data="filtered"
      @row-click="onRowClick"
      stripe
      size="small"
      empty-text="暂无会话"
    >
      <el-table-column prop="name" label="名称" sortable />
      <el-table-column prop="host" label="主机" />
      <el-table-column prop="port" label="端口" width="70" />
      <el-table-column label="状态" width="90">
        <template #default="{ row }">
          <span :class="['state', `state-${stateLabel(row.id)}`]">
            {{ stateLabel(row.id) }}
          </span>
        </template>
      </el-table-column>
    </el-table>
  </div>
</template>

<style scoped>
.session-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
h3 {
  margin: 0;
  font-size: 14px;
}
.state {
  font-size: 11px;
  padding: 2px 6px;
  border-radius: 3px;
  text-transform: uppercase;
}
.state-connecting { color: var(--el-color-warning); }
.state-connected { color: var(--el-color-success); }
.state-failed { color: var(--el-color-danger); }
.state-disconnected { color: var(--el-text-color-placeholder); }
.error {
  color: var(--el-color-danger);
  font-size: 12px;
}
</style>