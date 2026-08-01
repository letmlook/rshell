<script setup lang="ts">
/**
 * TransferQueue —— 切片 5.2
 *
 * 监听 TransferProgress / Completed / Failed 事件,
 * 显示进度条 + 暂停/取消按钮。
 *
 * 切片 5.1 状态:后端 TransferProgress 仅在 transfer 完成时一次性 publish(原子
 * 完成模型);流式节流到 10Hz 留待切片 5+ 进一步工作。
 */
import { onMounted, onBeforeUnmount, ref } from "vue";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { cancelTransfer, pauseTransfer, resumeTransfer } from "../ipc/client";
import type { Uuid } from "../ipc/types";

interface TransferProgressEvent {
  task_id: string;
  bytes: number;
  total: number;
  speed_bps: number;
}
interface TransferFailedEvent { task_id: string; error: string }

interface Row {
  id: Uuid;
  bytes: number;
  total: number;
  speed: number;
  state: "running" | "completed" | "failed";
  error?: string;
}

const rows = ref<Row[]>([]);
const unlisten: UnlistenFn[] = [];

function pushOrUpdate(e: TransferProgressEvent, state: Row["state"]) {
  const id = e.task_id as Uuid;
  const idx = rows.value.findIndex((r) => r.id === id);
  const row: Row = {
    id,
    bytes: e.bytes,
    total: e.total,
    speed: e.speed_bps,
    state,
  };
  if (idx === -1) rows.value.push(row);
  else rows.value[idx] = row;
}

async function subscribe() {
  unlisten.push(
    await listen<{ kind?: string } & Record<string, unknown>>("rshell://event", (msg) => {
      const p = msg.payload;
      if (p.kind === "TransferProgress") {
        pushOrUpdate(p as unknown as TransferProgressEvent, "running");
      } else if (p.kind === "TransferCompleted") {
        pushOrUpdate({ task_id: String(p.task_id), bytes: 0, total: 0, speed_bps: 0 }, "completed");
      } else if (p.kind === "TransferFailed") {
        const f = p as unknown as TransferFailedEvent;
        pushOrUpdate({ task_id: f.task_id, bytes: 0, total: 0, speed_bps: 0 }, "failed");
        const idx = rows.value.findIndex((r) => r.id === (f.task_id as Uuid));
        if (idx !== -1) rows.value[idx].error = f.error;
      }
    }),
  );
}

onMounted(subscribe);
onBeforeUnmount(() => unlisten.forEach((u) => u()));
</script>

<template>
  <section class="transfer-queue">
    <h3>传输队列 ({{ rows.length }})</h3>
    <el-empty v-if="rows.length === 0" description="暂无传输任务" />
    <el-table v-else :data="rows" size="small">
      <el-table-column prop="id" label="Task ID" width="80" />
      <el-table-column label="进度">
        <template #default="{ row }">
          <el-progress
            :percentage="row.total ? Math.round((row.bytes / row.total) * 100) : 0"
            :status="row.state === 'failed' ? 'exception' : (row.state === 'completed' ? 'success' : undefined)"
          />
        </template>
      </el-table-column>
      <el-table-column label="操作" width="200">
        <template #default="{ row }">
          <el-button
            v-if="row.state === 'running'"
            size="small"
            @click="pauseTransfer(row.id)"
          >暂停</el-button>
          <el-button
            v-if="row.state === 'running'"
            size="small"
            type="danger"
            @click="cancelTransfer(row.id)"
          >取消</el-button>
          <el-button
            v-if="row.state !== 'running'"
            size="small"
            @click="resumeTransfer(row.id)"
          >重试</el-button>
        </template>
      </el-table-column>
    </el-table>
  </section>
</template>

<style scoped>
.transfer-queue {
  padding: 12px;
}
h3 {
  margin: 0 0 12px;
  font-size: 14px;
}
</style>