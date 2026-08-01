<script setup lang="ts">
/**
 * TransferPanel —— v2 重设计
 *
 * Xftp 底部传输面板:
 *   - 折叠态:28px 高的 [传输|日志] tab bar
 *   - 展开态:全宽列表面板,字段:名称·状态·进度条·大小·本地路径 ←→ 远程路径·速度·估计剩余·经过时间
 *
 * 进度条颜色映射到 --rs-progress-*
 * 状态点复用签名元素
 *
 * 数据:本地 mock(可由 WorkspaceToolbar 触发入队;本次不做真后端对接)
 */
import { computed, ref } from "vue";

export type TransferPhase = "queued" | "active" | "paused" | "failed" | "done";

export interface TransferItem {
  id: string;
  name: string;
  phase: TransferPhase;
  progress: number; // 0..1
  size: number;
  local: string;
  remote: string;
  speed: number; // bytes/sec
}

const props = defineProps<{
  expanded: boolean;
  items?: TransferItem[];
  /** 队列高度,折叠后不占空间 */
  height?: number;
}>();

const emit = defineEmits<{
  (e: "toggle"): void;
  (e: "pause", id: string): void;
  (e: "resume", id: string): void;
  (e: "cancel", id: string): void;
}>();

const tab = ref<"transfer" | "log">("transfer");

const internalItems = ref<TransferItem[]>([
  {
    id: "t-1",
    name: "machine_m01986.json",
    phase: "done",
    progress: 1,
    size: 117 * 1024,
    local: "C:\\code\\pmi\\datasave\\machines\\machine_m01986.json",
    remote: "/root/pmi/datasave/machines/machine_m01986.json",
    speed: 0,
  },
  {
    id: "t-2",
    name: "machine_m01986.json.bak",
    phase: "active",
    progress: 0.42,
    size: 134 * 1024,
    local: "C:\\code\\pmi\\datasave\\machines\\machine_m01986.json.bak",
    remote: "/root/pmi/datasave/machines/machine_m01986.json.bak",
    speed: 580 * 1024,
  },
  {
    id: "t-3",
    name: "machine_m01986.json.bak.1",
    phase: "paused",
    progress: 0.18,
    size: 186 * 1024,
    local: "C:\\code\\pmi\\datasave\\machines\\machine_m01986.json.bak.1",
    remote: "/root/pmi/datasave/machines/machine_m01986.json.bak.1",
    speed: 0,
  },
  {
    id: "t-4",
    name: "build-output-2026-07-12.tar.gz",
    phase: "failed",
    progress: 0.66,
    size: 2_400_000,
    local: "C:\\code\\pmi\\builds\\build-output-2026-07-12.tar.gz",
    remote: "/root/pmi/datasave/machines/build-output-2026-07-12.tar.gz",
    speed: 0,
  },
]);

const merged = computed<TransferItem[]>(() => props.items ?? internalItems.value);

function fmtSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function fmtSpeed(b: number): string {
  if (b <= 0) return "—";
  return `${fmtSize(b)}/s`;
}

function fmtRemaining(item: TransferItem): string {
  if (item.phase === "done") return "已完成";
  if (item.phase === "paused") return "已暂停";
  if (item.phase === "failed") return "失败";
  if (item.phase === "queued") return "排队中";
  if (item.speed <= 0) return "—";
  const left = (item.size * (1 - item.progress)) / item.speed;
  if (!isFinite(left) || left < 0) return "—";
  if (left < 60) return `${left.toFixed(0)} 秒`;
  if (left < 3600) return `${(left / 60).toFixed(0)} 分钟`;
  return `${(left / 3600).toFixed(1)} 小时`;
}

function phaseLabel(p: TransferPhase): string {
  return { queued: "排队", active: "传输中", paused: "已暂停", failed: "失败", done: "完成" }[p];
}

function phaseClass(p: TransferPhase): string {
  return {
    queued: "rs-status-dot--disconnected",
    active: "rs-status-dot--connecting",
    paused: "rs-status-dot--connecting",
    failed: "rs-status-dot--failed",
    done: "rs-status-dot--connected",
  }[p];
}
</script>

<template>
  <section class="transfer-panel" :class="{ 'is-expanded': expanded }">
    <header class="panel-bar" @click="emit('toggle')">
      <div class="tabs">
        <button
          class="tab"
          :class="{ 'is-active': tab === 'transfer' }"
          @click.stop="tab = 'transfer'"
        >
          传输 ({{ merged.length }})
        </button>
        <button
          class="tab"
          :class="{ 'is-active': tab === 'log' }"
          @click.stop="tab = 'log'"
        >
          日志
        </button>
      </div>
      <div class="spacer" />
      <button class="icon-btn" :title="expanded ? '折叠' : '展开'" aria-label="折叠/展开">
        <svg width="12" height="12" viewBox="0 0 16 16">
          <path
            v-if="expanded"
            d="M4 6 L8 10 L12 6"
            fill="none"
            stroke="currentColor"
            stroke-width="1.4"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
          <path
            v-else
            d="M4 10 L8 6 L12 10"
            fill="none"
            stroke="currentColor"
            stroke-width="1.4"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
        </svg>
      </button>
    </header>
    <div v-if="expanded && tab === 'transfer'" class="panel-body">
      <el-table :data="merged" size="small" empty-text="暂无传输任务" class="xfer-table">
        <el-table-column prop="name" label="名称" min-width="180" />
        <el-table-column label="状态" width="100">
          <template #default="{ row }">
            <span class="phase">
              <span class="rs-status-dot" :class="phaseClass(row.phase)" />
              {{ phaseLabel(row.phase) }}
            </span>
          </template>
        </el-table-column>
        <el-table-column label="进度" width="170">
          <template #default="{ row }">
            <div class="progress">
              <div
                class="progress-fill"
                :class="`is-${row.phase}`"
                :style="{ width: `${Math.round(row.progress * 100)}%` }"
              />
            </div>
            <span class="progress-label">{{ Math.round(row.progress * 100) }}%</span>
          </template>
        </el-table-column>
        <el-table-column label="大小" width="90">
          <template #default="{ row }">{{ fmtSize(row.size) }}</template>
        </el-table-column>
        <el-table-column label="本地路径" min-width="180">
          <template #default="{ row }">
            <code class="path">{{ row.local }}</code>
          </template>
        </el-table-column>
        <el-table-column label="↔" width="32" align="center">
          <template #default>↔</template>
        </el-table-column>
        <el-table-column label="远程路径" min-width="180">
          <template #default="{ row }">
            <code class="path">{{ row.remote }}</code>
          </template>
        </el-table-column>
        <el-table-column label="速度" width="90">
          <template #default="{ row }">{{ fmtSpeed(row.speed) }}</template>
        </el-table-column>
        <el-table-column label="估计剩余" width="100">
          <template #default="{ row }">{{ fmtRemaining(row) }}</template>
        </el-table-column>
        <el-table-column label="操作" width="140" fixed="right">
          <template #default="{ row }">
            <el-button
              v-if="row.phase === 'active'"
              size="small"
              link
              @click="emit('pause', row.id)"
            >暂停</el-button>
            <el-button
              v-else-if="row.phase === 'paused' || row.phase === 'queued'"
              size="small"
              link
              type="primary"
              @click="emit('resume', row.id)"
            >继续</el-button>
            <el-button
              size="small"
              link
              type="danger"
              @click="emit('cancel', row.id)"
            >取消</el-button>
          </template>
        </el-table-column>
      </el-table>
    </div>
    <div v-else-if="expanded && tab === 'log'" class="panel-body log">
      <p class="log-line"><span class="log-time">15:17:02</span> 已开始传输 <code>machine_m01986.json</code></p>
      <p class="log-line"><span class="log-time">15:17:03</span> 上传 117 KB 到 <code>/root/pmi/datasave/machines/</code></p>
      <p class="log-line"><span class="log-time">15:18:11</span> 任务完成,共耗时 1 分 9 秒</p>
    </div>
  </section>
</template>

<style scoped>
.transfer-panel {
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  background: var(--rs-bg-panel);
  border-top: 1px solid var(--rs-border);
  height: var(--rs-transfer-panel-h-collapsed);
  transition: height var(--rs-dur-mid) var(--rs-easing);
  overflow: hidden;
}
.transfer-panel.is-expanded {
  height: var(--rs-transfer-panel-h-expanded);
}

.panel-bar {
  display: flex;
  align-items: center;
  height: var(--rs-transfer-panel-h-collapsed);
  padding: 0 var(--rs-s-2);
  background: var(--rs-bg-surface);
  border-bottom: 1px solid var(--rs-border);
  cursor: pointer;
  flex-shrink: 0;
  user-select: none;
}

.tabs {
  display: flex;
  gap: 0;
}
.tab {
  background: transparent;
  border: none;
  color: var(--rs-fg-muted);
  font-family: var(--rs-font-ui);
  font-size: var(--rs-fs-xs);
  padding: 0 var(--rs-s-3);
  height: 100%;
  cursor: pointer;
  border-bottom: 2px solid transparent;
  transition: color var(--rs-dur-fast) var(--rs-easing),
    border-color var(--rs-dur-fast) var(--rs-easing);
}
.tab:hover { color: var(--rs-fg); }
.tab.is-active {
  color: var(--rs-fg);
  border-bottom-color: var(--rs-accent);
}

.spacer { flex: 1; }

.icon-btn {
  width: 24px;
  height: 24px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  border-radius: var(--rs-radius-1);
  color: var(--rs-fg-muted);
  cursor: pointer;
}
.icon-btn:hover { background: var(--rs-bg-surface-hover); color: var(--rs-fg); }

.panel-body {
  flex: 1;
  overflow: auto;
  min-height: 0;
}
.panel-body.log {
  padding: var(--rs-s-2) var(--rs-s-3);
  font-size: var(--rs-fs-xs);
  font-family: var(--rs-font-mono);
  color: var(--rs-fg-muted);
}
.log-line { margin: 2px 0; }
.log-time { color: var(--rs-fg-disabled); margin-right: var(--rs-s-2); }

.xfer-table {
  --el-table-bg-color: var(--rs-bg-panel);
  --el-table-tr-bg-color: var(--rs-bg-panel);
  --el-table-border-color: var(--rs-border);
  width: 100%;
}
.phase {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.rs-status-dot {
  width: 7px;
  height: 7px;
}
.path {
  font-family: var(--rs-font-mono);
  font-size: var(--rs-fs-xs);
  color: var(--rs-fg-muted);
}

.progress {
  width: 100px;
  height: 6px;
  background: var(--rs-bg-surface);
  border-radius: 3px;
  overflow: hidden;
  display: inline-block;
  vertical-align: middle;
  margin-right: 6px;
}
.progress-fill {
  height: 100%;
  background: var(--rs-progress-fill);
  transition: width var(--rs-dur-fast) var(--rs-easing);
}
.progress-fill.is-active { background: var(--rs-progress-fill); }
.progress-fill.is-done { background: var(--rs-progress-done); }
.progress-fill.is-paused { background: var(--rs-progress-paused); }
.progress-fill.is-failed { background: var(--rs-progress-failed); }
.progress-fill.is-queued { background: var(--rs-fg-disabled); }

.progress-label {
  font-family: var(--rs-font-mono);
  font-size: var(--rs-fs-xs);
  color: var(--rs-fg-muted);
}
</style>