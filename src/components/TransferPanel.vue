<script setup lang="ts">
/**
 * TransferPanel —— v2 重设计
 *
 * Xftp 底部传输面板:
 *   - 折叠态:28px 高的 [传输|日志] tab bar
 *   - 展开态:全宽列表面板,字段:名称·状态·进度条·大小·本地路径 ←→ 远程路径·速度·估计剩余·经过时间·暂停/恢复
 *
 * 进度条颜色映射到 --rs-progress-*
 * 状态点复用签名元素
 *
 * 数据来自后端真实传输队列快照。
 *
 * 暂停/恢复/取消/删除控制:
 *   - 仅对 `active` 任务渲染"暂停",仅对 `paused` 任务渲染"继续"。
 *   - `active`/`paused` 渲染"取消":调用 CancelTransfer 置终态 Cancelled。
 *   - 仅终态(done/failed/cancelled)渲染"删除":调用 RemoveTransfer 移除队列条目。
 *     移除只作用于队列列表,不会删除已传输的本地/远端文件。
 *   - 按钮调用期间由调用方控制,本组件只发出 pause/resume/cancel/remove 事件。
 *   - 进行中(`pendingTaskIds`)的按钮自动禁用,避免重复点击。
 *   - 失败提示由调用方写入 `actionError`,本组件原样展示,不做乐观更新。
 */
import { computed, ref } from "vue";

export type TransferPhase = "queued" | "active" | "paused" | "failed" | "done" | "cancelled";

export interface TransferItem {
  id: string;
  name: string;
  phase: TransferPhase;
  progress: number; // 0..1
  size: number;
  local: string;
  remote: string;
  speed: number; // bytes/sec
  error?: string | null;
}

const props = defineProps<{
  expanded: boolean;
  items: TransferItem[];
  /** 队列读取失败时的提示；非空时优先于空状态展示 */
  error?: string | null;
  /** 队列高度,折叠后不占空间 */
  height?: number;
  /** 暂停/恢复调用中的任务 ID；用于禁用对应按钮 */
  pendingTaskIds?: ReadonlySet<string>;
  /** 上一次 pause/resume 调用的错误；非空时在面板顶部展示一行 */
  actionError?: string | null;
}>();

const emit = defineEmits<{
  (e: "toggle"): void;
  (e: "pause", taskId: string): void;
  (e: "resume", taskId: string): void;
  (e: "cancel", taskId: string): void;
  (e: "remove", taskId: string): void;
  /** 队列生命周期批量操作：按终态分组移除条目 */
  (e: "remove-many", taskIds: string[]): void;
}>();

const tab = ref<"transfer" | "log">("transfer");

const merged = computed<TransferItem[]>(() => props.items);

function isPending(taskId: string): boolean {
  return props.pendingTaskIds?.has(taskId) ?? false;
}

function onPause(taskId: string, event: Event) {
  event.stopPropagation();
  if (isPending(taskId)) return;
  emit("pause", taskId);
}

function onResume(taskId: string, event: Event) {
  event.stopPropagation();
  if (isPending(taskId)) return;
  emit("resume", taskId);
}

function onCancel(taskId: string, event: Event) {
  event.stopPropagation();
  if (isPending(taskId)) return;
  emit("cancel", taskId);
}

function onRemove(taskId: string, event: Event) {
  event.stopPropagation();
  if (isPending(taskId)) return;
  emit("remove", taskId);
}

/** 终态（完成/失败/已取消）才允许从队列移除 */
function isRemovable(phase: TransferPhase): boolean {
  return phase === "done" || phase === "failed" || phase === "cancelled";
}

// ── 队列生命周期菜单 ──
// 批量操作只覆盖终态条目：活跃任务的拷贝循环仍在跑，必须先逐条取消。
type BulkAction = "clearDone" | "clearFailed" | "clearCancelled" | "clearAll";

const bulkMenuOpen = ref(false);

const removableIds = computed(() =>
  merged.value.filter((row) => isRemovable(row.phase)).map((row) => row.id),
);

const bulkTargets = computed<Record<BulkAction, string[]>>(() => {
  const byPhase = (phase: TransferPhase) =>
    merged.value.filter((row) => row.phase === phase).map((row) => row.id);
  return {
    clearDone: byPhase("done"),
    clearFailed: byPhase("failed"),
    clearCancelled: byPhase("cancelled"),
    clearAll: removableIds.value,
  };
});

const BULK_ACTIONS: { action: BulkAction; label: string }[] = [
  { action: "clearDone", label: "清除已完成" },
  { action: "clearFailed", label: "清除失败" },
  { action: "clearCancelled", label: "清除已取消" },
  { action: "clearAll", label: "清除全部已结束" },
];

/** 没有可清理的终态条目时菜单整体不可用，避免空操作入口 */
const canBulkClear = computed(() => removableIds.value.length > 0);

function runBulkAction(action: BulkAction) {
  bulkMenuOpen.value = false;
  const ids = bulkTargets.value[action];
  if (ids.length === 0) return;
  emit("remove-many", ids);
}

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
  if (item.phase === "cancelled") return "已取消";
  if (item.phase === "queued") return "排队中";
  if (item.speed <= 0) return "—";
  const left = (item.size * (1 - item.progress)) / item.speed;
  if (!isFinite(left) || left < 0) return "—";
  if (left < 60) return `${left.toFixed(0)} 秒`;
  if (left < 3600) return `${(left / 60).toFixed(0)} 分钟`;
  return `${(left / 3600).toFixed(1)} 小时`;
}

function phaseLabel(p: TransferPhase): string {
  return { queued: "排队", active: "传输中", paused: "已暂停", failed: "失败", done: "完成", cancelled: "已取消" }[p];
}

function phaseClass(p: TransferPhase): string {
  return {
    queued: "rs-status-dot--disconnected",
    active: "rs-status-dot--connecting",
    paused: "rs-status-dot--connecting",
    failed: "rs-status-dot--failed",
    done: "rs-status-dot--connected",
    cancelled: "rs-status-dot--disconnected",
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
      <div class="bulk-wrap">
        <button
          class="icon-btn"
          data-test="xfer-bulk-toggle"
          aria-label="队列管理"
          title="队列管理"
          :aria-expanded="bulkMenuOpen"
          :disabled="!canBulkClear"
          @click.stop="bulkMenuOpen = !bulkMenuOpen"
        >
          <svg width="12" height="12" viewBox="0 0 16 16">
            <path d="M2 4 H14 M2 8 H14 M2 12 H14" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
          </svg>
        </button>
        <ul
          v-if="bulkMenuOpen"
          class="bulk-menu"
          data-test="xfer-bulk-menu"
          role="menu"
        >
          <li
            v-for="item in BULK_ACTIONS"
            :key="item.action"
            role="menuitem"
            class="bulk-item"
            :data-test="`xfer-bulk-${item.action}`"
            :class="{ 'is-empty': bulkTargets[item.action].length === 0 }"
            tabindex="0"
            @click="runBulkAction(item.action)"
            @keydown.enter="runBulkAction(item.action)"
          >
            {{ item.label }}（{{ bulkTargets[item.action].length }}）
          </li>
        </ul>
      </div>
      <button class="icon-btn" :title="expanded ? '折叠' : '展开'" aria-label="折叠/展开" @click.stop="emit('toggle')">
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
    <p v-if="actionError" class="panel-load-error" role="alert" data-test="xfer-action-error">
      {{ actionError }}
    </p>
    <p v-else-if="error" class="panel-load-error" role="alert">
      传输队列读取失败：{{ error }}（下表可能不是最新状态）
    </p>
    <div v-if="expanded && tab === 'transfer'" class="panel-body">
      <div v-if="merged.length === 0" class="empty-state">暂无传输任务</div>
      <ul v-else class="xfer-list" role="list">
        <li
          v-for="row in merged"
          :key="row.id"
          class="xfer-row"
          :data-row-id="row.id"
        >
          <div class="xfer-col xfer-name" :title="row.name">{{ row.name }}</div>
          <div class="xfer-col xfer-phase">
            <span class="phase">
              <span class="rs-status-dot" :class="phaseClass(row.phase)" />
              {{ phaseLabel(row.phase) }}
            </span>
          </div>
          <div class="xfer-col xfer-progress">
            <div class="progress">
              <div
                class="progress-fill"
                :class="`is-${row.phase}`"
                :style="{ width: `${Math.round(row.progress * 100)}%` }"
              />
            </div>
            <span class="progress-label">{{ Math.round(row.progress * 100) }}%</span>
          </div>
          <div class="xfer-col xfer-size">{{ fmtSize(row.size) }}</div>
          <div class="xfer-col xfer-path"><code class="path" :title="row.local">{{ row.local }}</code></div>
          <div class="xfer-col xfer-arrow">↔</div>
          <div class="xfer-col xfer-path"><code class="path" :title="row.remote">{{ row.remote }}</code></div>
          <div class="xfer-col xfer-speed">{{ fmtSpeed(row.speed) }}</div>
          <div class="xfer-col xfer-remaining">{{ fmtRemaining(row) }}</div>
          <div class="xfer-col xfer-error" v-if="row.phase === 'failed'" role="alert">
            {{ row.error || '传输失败，请检查连接和文件权限后重试。' }}
          </div>
          <div class="xfer-col xfer-actions">
            <button
              v-if="row.phase === 'active'"
              type="button"
              class="xfer-action"
              data-test="xfer-pause"
              :data-task-id="row.id"
              aria-label="暂停传输"
              :disabled="isPending(row.id)"
              @click="onPause(row.id, $event)"
            >
              暂停
            </button>
            <button
              v-if="row.phase === 'paused'"
              type="button"
              class="xfer-action"
              data-test="xfer-resume"
              :data-task-id="row.id"
              aria-label="继续传输"
              :disabled="isPending(row.id)"
              @click="onResume(row.id, $event)"
            >
              继续
            </button>
            <button
              v-if="row.phase === 'active' || row.phase === 'paused'"
              type="button"
              class="xfer-action"
              data-test="xfer-cancel"
              :data-task-id="row.id"
              aria-label="取消传输"
              :disabled="isPending(row.id)"
              @click="onCancel(row.id, $event)"
            >
              取消
            </button>
            <button
              v-if="isRemovable(row.phase)"
              type="button"
              class="xfer-action is-danger"
              data-test="xfer-remove"
              :data-task-id="row.id"
              aria-label="从队列删除"
              :disabled="isPending(row.id)"
              @click="onRemove(row.id, $event)"
            >
              删除
            </button>
          </div>
        </li>
      </ul>
    </div>
    <div v-else-if="expanded && tab === 'log'" class="panel-body log">
      <p class="log-line">传输错误会显示在任务状态中。</p>
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
.panel-load-error {
  margin: 6px 10px 0;
  padding: 6px 8px;
  border: 1px solid var(--el-color-danger);
  border-radius: 4px;
  color: var(--el-color-danger);
  font-size: 12px;
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
.icon-btn:disabled { opacity: 0.4; cursor: not-allowed; }

.bulk-wrap {
  position: relative;
  display: flex;
  align-items: center;
}
.bulk-menu {
  position: absolute;
  top: calc(100% + 4px);
  right: 0;
  z-index: 30;
  margin: 0;
  padding: 4px 0;
  list-style: none;
  min-width: 156px;
  background: var(--rs-bg-surface);
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-1);
  box-shadow: 0 6px 18px rgb(0 0 0 / 32%);
}
.bulk-item {
  padding: 6px 12px;
  font-size: var(--rs-fs-xs);
  color: var(--rs-fg);
  cursor: pointer;
  white-space: nowrap;
}
.bulk-item:hover,
.bulk-item:focus-visible {
  background: var(--rs-bg-surface-hover);
  outline: none;
}
/* 该分组当前没有可清理条目：仍展示计数（说明为什么不可用），但不触发操作 */
.bulk-item.is-empty {
  color: var(--rs-fg-disabled);
  cursor: not-allowed;
}
.bulk-item.is-empty:hover {
  background: transparent;
}

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

.empty-state {
  padding: var(--rs-s-3);
  color: var(--rs-fg-muted);
  font-size: var(--rs-fs-xs);
}

.xfer-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
}
.xfer-row {
  display: grid;
  grid-template-columns: minmax(140px, 1.5fr) 96px 170px 80px minmax(140px, 1.4fr) 32px minmax(140px, 1.4fr) 80px 90px 170px;
  align-items: center;
  gap: var(--rs-s-2);
  padding: var(--rs-s-2) var(--rs-s-3);
  border-bottom: 1px solid var(--rs-border);
  font-size: var(--rs-fs-xs);
}
.xfer-row:last-child { border-bottom: none; }
.xfer-col {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--rs-fg);
}
.xfer-error {
  grid-column: 1 / -1;
  color: var(--el-color-danger);
  font-size: var(--rs-fs-xs);
  margin-top: var(--rs-s-1);
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

.xfer-actions {
  display: flex;
  justify-content: flex-end;
  gap: 4px;
}
.xfer-action {
  background: var(--rs-bg-surface);
  border: 1px solid var(--rs-border);
  color: var(--rs-fg);
  border-radius: var(--rs-radius-1);
  padding: 4px 10px;
  font-size: var(--rs-fs-xs);
  cursor: pointer;
}
.xfer-action:hover:not(:disabled) {
  background: var(--rs-bg-surface-hover);
  border-color: var(--rs-accent);
}
.xfer-action:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
/* 删除只影响队列条目，但与「取消」区分开，避免误点 */
.xfer-action.is-danger {
  color: var(--el-color-danger);
  border-color: var(--el-color-danger);
}
.xfer-action.is-danger:hover:not(:disabled) {
  background: var(--el-color-danger);
  border-color: var(--el-color-danger);
  color: #fff;
}
</style>