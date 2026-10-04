/**
 * 传输日志环形缓冲 —— 传输面板「日志」tab 的唯一数据源。
 *
 * 之前该 tab 只有一行占位文案，用户看不到任何真实事件（见 TransferPanel
 * 日志分支）。这里把传输相关的真实动作（入队、控制、冲突、失败、完成）
 * 收进一个模块级 reactive 缓冲，由 App.vue / TransferWorkspace 写入、
 * TransferPanel 渲染。
 *
 * 为什么是模块级单例而不是 store/props：
 *   - 写入方分散在 App.vue（控制命令、事件流）与 TransferWorkspace（入队动作），
 *     走 props 只能一路透传事件再回抛，徒增耦合；
 *   - 日志是「本次运行期」的诊断流，不需要持久化，也不需要跨进程一致性，
 *     和 lastLocalDir 一样属于纯前端状态。
 *
 * 容量固定：只保留最近 MAX_TRANSFER_LOG_ENTRIES 条，避免长时间传输把
 * DOM 撑到几万行。丢弃最旧条目并在条目上标注被截断的条数，日志页明确告诉
 * 用户「前面还有 N 条已被丢弃」，而不是让用户以为那就是全部。
 */
import { ref, type Ref } from "vue";

export type TransferLogLevel = "info" | "success" | "warn" | "error";

export interface TransferLogEntry {
  /** 单调递增序号，作为 key 与「同一事件」去重依据 */
  seq: number;
  /** epoch 毫秒；展示时按本地时区格式化 */
  time: number;
  level: TransferLogLevel;
  message: string;
  /** 补充信息（远端/本地路径、原始错误串等），可空 */
  detail?: string;
}

export const MAX_TRANSFER_LOG_ENTRIES = 500;

const entries = ref<TransferLogEntry[]>([]);
/** 因超出容量被丢弃的最旧条目数（累计，不随清零重置以外的操作回退） */
const droppedCount = ref(0);
let nextSeq = 1;

/**
 * 追加一条日志。
 *
 * 容量满时丢最旧的一条并累加 droppedCount，调用方无需关心裁剪。
 */
export function appendTransferLog(
  level: TransferLogLevel,
  message: string,
  detail?: string,
): TransferLogEntry {
  const entry: TransferLogEntry = {
    seq: nextSeq++,
    time: Date.now(),
    level,
    message,
    detail: detail || undefined,
  };
  const next = entries.value.concat(entry);
  if (next.length > MAX_TRANSFER_LOG_ENTRIES) {
    const overflow = next.length - MAX_TRANSFER_LOG_ENTRIES;
    entries.value = next.slice(overflow);
    droppedCount.value += overflow;
  } else {
    entries.value = next;
  }
  return entry;
}

/** 清空日志。丢弃计数同时归零——用户主动清空后不应继续提示「更早还有 N 条」 */
export function clearTransferLog(): void {
  entries.value = [];
  droppedCount.value = 0;
}

export function useTransferLog(): {
  entries: Ref<TransferLogEntry[]>;
  droppedCount: Ref<number>;
  append: typeof appendTransferLog;
  clear: typeof clearTransferLog;
} {
  return { entries, droppedCount, append: appendTransferLog, clear: clearTransferLog };
}

/** 本地时区 HH:MM:SS；非法时间戳原样返回，不吞掉异常显示成 Invalid Date */
export function formatTransferLogTime(time: number): string {
  if (!Number.isFinite(time)) return "--:--:--";
  const d = new Date(time);
  if (Number.isNaN(d.getTime())) return "--:--:--";
  return [d.getHours(), d.getMinutes(), d.getSeconds()]
    .map((v) => String(v).padStart(2, "0"))
    .join(":");
}
