<script setup lang="ts">
/**
 * TransferPanel —— Xftp 底部传输/日志面板
 *
 *   - 折叠态:28px 高的 [传输|日志] tab bar
 *   - 展开态:全宽列表面板,字段:名称·状态·进度条·大小·本地路径 ←→ 远程路径·速度·剩余·操作
 *   - 顶部边缘可拖动改高度(见 startResize),双击边缘恢复默认高度
 *
 * 进度条颜色映射到 --rs-progress-*;状态点复用签名元素。
 * 数据来自后端真实传输队列快照(utils/transferItem.ts 映射),日志来自
 * utils/transferLog.ts 的真实事件流——两者都不允许假数据兜底。
 *
 * 暂停/恢复/取消/重试/删除控制:
 *   - 仅对 `active` 任务渲染"暂停",仅对 `paused` 任务渲染"继续"。
 *   - `active`/`paused` 渲染"取消":调用 CancelTransfer 置终态 Cancelled。
 *   - `failed`/`cancelled` 渲染"重试":调用 RetryTransfer 从零创建新任务,
 *     原任务保留其终态（不会自动覆盖 —— staged lifecycle 提交前会再检一次）。
 *   - 仅终态(done/failed/cancelled)渲染"删除":调用 RemoveTransfer 移除队列条目。
 *     移除只作用于队列列表,不会删除已传输的本地/远端文件。
 *   - 按钮调用期间由调用方控制,本组件只发出 pause/resume/cancel/retry/remove 事件。
 *   - 进行中(`pendingTaskIds`)的按钮自动禁用,避免重复点击。
 *   - 失败提示由调用方写入 `actionError`,本组件原样展示,不做乐观更新。
 */
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { formatTransferLogTime, type TransferLogEntry, type TransferLogLevel } from "../utils/transferLog";

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
  /** R2-T2：终态下的清理结果标签（`"cleaned"` / `"residue"`）。活跃任务为 null。 */
  cleanup_status?: string | null;
  /** R2-T2：staging temp 路径 —— 终态下若 cleanup 失败则给出，否则为 null。 */
  temp_path?: string | null;
  /** R2-T2：提交策略标签（`"posix_rename"` / `"standard_rename"`）。仅 Completed 终态有值。 */
  commit_strategy?: string | null;
}

/** 展开态默认高度;与 tokens.css 的 --rs-transfer-panel-h-expanded 保持一致 */
const DEFAULT_EXPANDED_HEIGHT = 220;
/** 拖动下限:再小就读不到「名称 + 操作」两列 */
const MIN_EXPANDED_HEIGHT = 120;
/** 拖动上限时给上方文件窗格保留的高度,避免把窗格压到看不见 */
const MIN_FILES_PANE_HEIGHT = 140;

const props = defineProps<{
  expanded: boolean;
  items: TransferItem[];
  /** 队列读取失败时的提示；非空时优先于空状态展示 */
  error?: string | null;
  /** 展开态高度(px)；不传时用 DEFAULT_EXPANDED_HEIGHT。拖动后由父组件回写 */
  height?: number;
  /** 暂停/恢复调用中的任务 ID；用于禁用对应按钮 */
  pendingTaskIds?: ReadonlySet<string>;
  /** 上一次 pause/resume 调用的错误；非空时在面板顶部展示一行 */
  actionError?: string | null;
  /** 日志页数据（真实事件流，非占位文案） */
  logs?: TransferLogEntry[];
  /** 因超出环形缓冲容量而被丢弃的更早日志条数 */
  droppedLogs?: number;
}>();

const emit = defineEmits<{
  (e: "toggle"): void;
  (e: "pause", taskId: string): void;
  (e: "resume", taskId: string): void;
  (e: "cancel", taskId: string): void;
  (e: "remove", taskId: string): void;
  /** R2-T2：从零重试 —— 调用 retryTransfer(task_id)；仅终态任务显示。 */
  (e: "retry", taskId: string): void;
  /** 队列生命周期批量操作：按终态分组移除条目 */
  (e: "remove-many", taskIds: string[]): void;
  /** 拖动边缘后回写展开态高度 */
  (e: "update:height", px: number): void;
  /** 请求清空日志 */
  (e: "clear-log"): void;
}>();

const tab = ref<"transfer" | "log">("transfer");

const merged = computed<TransferItem[]>(() => props.items);
const logEntries = computed<TransferLogEntry[]>(() => props.logs ?? []);

const effectiveHeight = computed(() => Math.max(MIN_EXPANDED_HEIGHT, props.height ?? DEFAULT_EXPANDED_HEIGHT));

const panelStyle = computed(() =>
  props.expanded ? { height: `${effectiveHeight.value}px` } : {},
);

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

/** R2-T2：终态任务（failed / cancelled）可重试。Completed 不重试——
 * 「重试」是「重新传一份」，不是「重看结果」。 */
function onRetry(taskId: string, event: Event) {
  event.stopPropagation();
  if (isPending(taskId)) return;
  emit("retry", taskId);
}

/** 终态（完成/失败/已取消）才允许从队列移除 */
function isRemovable(phase: TransferPhase): boolean {
  return phase === "done" || phase === "failed" || phase === "cancelled";
}

/**
 * 某一行在当前相位下会渲染几个操作按钮。
 * 活跃/暂停 = 暂停(继续) + 取消；
 * 终态 failed/cancelled = 重试 + 删除（done 仅删除）。
 */
function actionCount(phase: TransferPhase): number {
  if (phase === "active" || phase === "paused") return 2;
  if (phase === "failed" || phase === "cancelled") return 2;
  if (isRemovable(phase)) return 1;
  return 0;
}

const ACTION_BUTTON_WIDTH = 48;
const ACTION_GAP = 4;

/**
 * 操作列宽度按**当前队列里最多的按钮数**算，而不是写死。
 *
 * 写死 170px 时，终态行只有一个「删除」按钮却占着 2 个按钮的宽度，整张表
 * 右侧空一大块。改成按实际按钮数计算后，列宽既贴合内容，各行又仍然对齐
 * （用全表最大值，而不是每行各自撑开——那样前面的列会错位）。
 */
const actionsColumnWidth = computed(() => {
  const max = merged.value.reduce((acc, row) => Math.max(acc, actionCount(row.phase)), 0);
  if (max === 0) return `${ACTION_BUTTON_WIDTH}px`;
  return `${max * ACTION_BUTTON_WIDTH + (max - 1) * ACTION_GAP + 8}px`;
});

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

// ── 日志 ──
const LOG_LEVEL_LABEL: Record<TransferLogLevel, string> = {
  info: "信息",
  success: "完成",
  warn: "警告",
  error: "失败",
};

const logBodyRef = ref<HTMLElement | null>(null);
/** 用户是否贴在底部：手动上滚查看历史后不再被新日志拽回底部 */
const stickToBottom = ref(true);

function onLogScroll() {
  const el = logBodyRef.value;
  if (!el) return;
  stickToBottom.value = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
}

function scrollLogToBottom() {
  const el = logBodyRef.value;
  if (!el) return;
  el.scrollTop = el.scrollHeight;
}

watch(
  () => [logEntries.value.length, tab.value, props.expanded] as const,
  async () => {
    if (tab.value !== "log" || !props.expanded || !stickToBottom.value) return;
    await nextTick();
    scrollLogToBottom();
  },
);

function onLogTabActivated() {
  tab.value = "log";
  stickToBottom.value = true;
  void nextTick().then(scrollLogToBottom);
}

// ── 拖动高度 ──
// 拖动条位于面板顶边，与文件窗格的下边缘重合：往下拖 = 面板变矮。
// 监听挂在 window 上，鼠标在面板外释放也要收尾，否则会粘住光标。
const resizing = ref(false);
const rootEl = ref<HTMLElement | null>(null);
let resizeStartY = 0;
let resizeStartHeight = 0;

function maxResizableHeight(): number {
  const parent = rootEl.value?.parentElement as HTMLElement | null;
  const parentHeight = parent?.clientHeight ?? 0;
  if (parentHeight <= 0) {
    return typeof window === "undefined" ? DEFAULT_EXPANDED_HEIGHT : Math.round(window.innerHeight * 0.8);
  }
  return Math.max(MIN_EXPANDED_HEIGHT, parentHeight - MIN_FILES_PANE_HEIGHT);
}

function clampHeight(px: number): number {
  return Math.round(Math.max(MIN_EXPANDED_HEIGHT, Math.min(maxResizableHeight(), px)));
}

function onResizeMove(e: MouseEvent) {
  if (!resizing.value) return;
  // 向上拖（clientY 变小）面板应变高
  const next = clampHeight(resizeStartHeight - (e.clientY - resizeStartY));
  emit("update:height", next);
}

function endResize() {
  if (!resizing.value) return;
  resizing.value = false;
  window.removeEventListener("mousemove", onResizeMove);
  window.removeEventListener("mouseup", endResize);
}

function startResize(e: MouseEvent) {
  if (!props.expanded) return;
  e.preventDefault();
  resizing.value = true;
  resizeStartY = e.clientY;
  resizeStartHeight = effectiveHeight.value;
  window.addEventListener("mousemove", onResizeMove);
  window.addEventListener("mouseup", endResize);
}

/** 双击拖动条 = 恢复默认高度；给手滑的用户一个不需要精确拖拽的退路 */
function resetHeight() {
  emit("update:height", DEFAULT_EXPANDED_HEIGHT);
}

onBeforeUnmount(() => {
  if (typeof window === "undefined") return;
  window.removeEventListener("mousemove", onResizeMove);
  window.removeEventListener("mouseup", endResize);
});

const columns = [
  "名称",
  "状态",
  "进度",
  "大小",
  "本地路径",
  "",
  "远程路径",
  "速度",
  "剩余",
  "操作",
];
</script>

<template>
  <section
    ref="rootEl"
    class="transfer-panel"
    :class="{ 'is-expanded': expanded, 'is-resizing': resizing }"
    data-test="xfer-panel"
    :style="panelStyle"
    :data-actions-width="actionsColumnWidth"
  >
    <!-- 顶边拖动条：与上方文件窗格的下边缘重合 -->
    <div
      v-if="expanded"
      class="resize-grip"
      data-test="xfer-resize"
      role="separator"
      aria-orientation="horizontal"
      :aria-valuenow="effectiveHeight"
      aria-valuemin="120"
      title="拖动调整高度，双击恢复默认"
      @mousedown="startResize"
      @dblclick="resetHeight"
    >
      <span class="resize-grip-bar" />
    </div>

    <header class="panel-bar" @click="emit('toggle')">
      <div class="tabs">
        <button
          class="tab"
          :class="{ 'is-active': tab === 'transfer' }"
          data-test="xfer-tab-transfer"
          @click.stop="tab = 'transfer'"
        >
          传输 ({{ merged.length }})
        </button>
        <button
          class="tab"
          :class="{ 'is-active': tab === 'log' }"
          data-test="xfer-tab-log"
          @click.stop="onLogTabActivated"
        >
          日志{{ logEntries.length ? ` (${logEntries.length})` : "" }}
        </button>
      </div>
      <div class="spacer" />
      <button
        v-if="tab === 'log' && logEntries.length > 0"
        class="text-btn"
        data-test="xfer-log-clear"
        title="清空日志"
        @click.stop="emit('clear-log')"
      >
        清空
      </button>
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
      <template v-else>
        <!-- 列标题行：随内容横向滚动，纵向滚动时吸顶 -->
        <div class="xfer-row is-header" aria-hidden="true">
          <div
            v-for="(col, i) in columns"
            :key="i"
            class="xfer-col xfer-h"
            :class="{ 'is-blank': !col }"
          >
            {{ col }}
          </div>
        </div>
        <ul class="xfer-list" role="list" data-test="xfer-list">
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
              <!-- R2-T2：失败 / 已取消的任务可重试 —— 从零创建新任务，
                   原任务保留其终态。Completed 不重试（避免「重看结果」误解）。 -->
              <button
                v-if="row.phase === 'failed' || row.phase === 'cancelled'"
                type="button"
                class="xfer-action"
                data-test="xfer-retry"
                :data-task-id="row.id"
                aria-label="重试传输"
                :disabled="isPending(row.id)"
                @click="onRetry(row.id, $event)"
              >
                重试
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
      </template>
    </div>
    <div
      v-else-if="expanded && tab === 'log'"
      ref="logBodyRef"
      class="panel-body log"
      data-test="xfer-log-body"
      @scroll="onLogScroll"
    >
      <p v-if="droppedLogs" class="log-dropped" data-test="xfer-log-dropped">
        更早还有 {{ droppedLogs }} 条记录已超出缓冲被丢弃。
      </p>
      <p v-if="logEntries.length === 0" class="log-empty" data-test="xfer-log-empty">
        暂无日志记录。传输入队、暂停/取消、目标冲突与失败都会记在这里。
      </p>
      <ul v-else class="log-list" data-test="xfer-log-list">
        <li
          v-for="entry in logEntries"
          :key="entry.seq"
          class="log-row"
          :class="`is-${entry.level}`"
          :data-test="`xfer-log-${entry.seq}`"
        >
          <span class="log-time">{{ formatTransferLogTime(entry.time) }}</span>
          <span class="log-level">{{ LOG_LEVEL_LABEL[entry.level] }}</span>
          <span class="log-msg">
            {{ entry.message }}
            <span v-if="entry.detail" class="log-detail">— {{ entry.detail }}</span>
          </span>
        </li>
      </ul>
    </div>
  </section>
</template>

<style scoped>
.transfer-panel {
  position: relative;
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  background: var(--rs-bg-panel);
  border-top: 1px solid var(--rs-border);
  height: var(--rs-transfer-panel-h-collapsed);
  transition: height var(--rs-dur-mid) var(--rs-easing);
  overflow: hidden;
}
/* 拖动时关掉高度过渡，否则高度会追着鼠标跑，手感发飘 */
.transfer-panel.is-resizing {
  transition: none;
  user-select: none;
  cursor: row-resize;
}

.resize-grip {
  position: absolute;
  top: -3px;
  left: 0;
  right: 0;
  height: 7px;
  cursor: row-resize;
  z-index: 5;
}
.resize-grip-bar {
  position: absolute;
  top: 3px;
  left: 50%;
  transform: translateX(-50%);
  width: 40px;
  height: 1px;
  background: var(--rs-border);
  opacity: 0;
  transition: opacity var(--rs-dur-fast) var(--rs-easing);
}
.resize-grip:hover .resize-grip-bar {
  opacity: 1;
  background: var(--rs-accent);
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

.text-btn {
  background: transparent;
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-1);
  color: var(--rs-fg-muted);
  font-family: var(--rs-font-ui);
  font-size: var(--rs-fs-xs);
  padding: 2px 8px;
  margin-right: var(--rs-s-1);
  cursor: pointer;
}
.text-btn:hover { color: var(--rs-fg); border-color: var(--rs-accent); }

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
/* 行与列标题共用同一套列宽；操作列宽由 JS 按实际按钮数算好后写在 data 属性上，
   min-width 让窄面板走横向滚动而不是把列挤扁 */
.xfer-row {
  display: grid;
  grid-template-columns: minmax(140px, 1.5fr) 96px 170px 80px minmax(140px, 1.4fr) 32px minmax(140px, 1.4fr) 80px 90px var(--xfer-actions-w, 104px);
  align-items: center;
  gap: var(--rs-s-2);
  padding: var(--rs-s-2) var(--rs-s-3);
  border-bottom: 1px solid var(--rs-border);
  font-size: var(--rs-fs-xs);
  min-width: 940px;
}
.xfer-row:last-child { border-bottom: none; }
.xfer-row.is-header {
  position: sticky;
  top: 0;
  z-index: 2;
  padding-top: 4px;
  padding-bottom: 4px;
  background: var(--rs-bg-surface);
  color: var(--rs-fg-muted);
  font-size: var(--rs-fs-xs);
  border-bottom: 1px solid var(--rs-border);
}
.xfer-h.is-blank { min-width: 0; }
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

/* ── 日志页 ── */
.panel-body.log {
  padding: var(--rs-s-1) 0;
  font-size: var(--rs-fs-xs);
  font-family: var(--rs-font-mono);
  color: var(--rs-fg-muted);
}
.log-list {
  list-style: none;
  margin: 0;
  padding: 0;
}
.log-row {
  display: grid;
  grid-template-columns: 64px 48px minmax(0, 1fr);
  gap: var(--rs-s-2);
  align-items: baseline;
  padding: 2px var(--rs-s-3);
}
.log-row:hover { background: var(--rs-row-hover); }
.log-time { color: var(--rs-fg-disabled); }
.log-level {
  font-size: var(--rs-fs-xs);
  text-align: center;
  border-radius: 2px;
  border: 1px solid var(--rs-border);
  color: var(--rs-fg-muted);
}
.log-row.is-success .log-level { color: var(--rs-progress-done); border-color: var(--rs-progress-done); }
.log-row.is-warn .log-level { color: var(--rs-progress-paused); border-color: var(--rs-progress-paused); }
.log-row.is-error .log-level { color: var(--rs-p-danger); border-color: var(--rs-p-danger); }
.log-msg {
  color: var(--rs-fg);
  font-family: var(--rs-font-ui);
  white-space: pre-wrap;
  word-break: break-word;
}
.log-detail {
  color: var(--rs-fg-muted);
  font-family: var(--rs-font-mono);
}
.log-empty,
.log-dropped {
  margin: 0;
  padding: var(--rs-s-2) var(--rs-s-3);
  font-family: var(--rs-font-ui);
}
.log-dropped {
  color: var(--rs-fg-disabled);
  border-bottom: 1px dashed var(--rs-border);
}
</style>
