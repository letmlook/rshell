/**
 * 后端传输任务快照 → 面板行。
 *
 * 速度只在 Transferring 状态展示：终态下残留的最后一次采样速度没有意义，
 * 归零后 TransferPanel 的 fmtSpeed(0) 会显示 "—"。
 */
import type { TransferTaskInfo } from "../ipc/types";
import type { TransferItem, TransferPhase } from "../components/TransferPanel.vue";

const PHASE_BY_STATE: Record<TransferTaskInfo["state"], TransferPhase> = {
  Pending: "queued",
  Transferring: "active",
  Paused: "paused",
  Completed: "done",
  Failed: "failed",
  Cancelled: "cancelled",
};

export function toTransferItem(task: TransferTaskInfo): TransferItem {
  return {
    id: task.id,
    name: task.remote_path.split("/").pop() || task.remote_path,
    phase: PHASE_BY_STATE[task.state],
    progress: task.total_bytes ? task.bytes_transferred / task.total_bytes : 0,
    size: task.total_bytes,
    local: task.local_path,
    remote: task.remote_path,
    speed: task.state === "Transferring" ? task.speed_bps : 0,
    error: task.error_message,
  };
}
