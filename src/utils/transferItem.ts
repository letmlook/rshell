/**
 * 后端传输任务快照 → 面板行。
 *
 * 速度只在 Transferring 状态展示：终态下残留的最后一次采样速度没有意义，
 * 归零后 TransferPanel 的 fmtSpeed(0) 会显示 "—"。
 *
 * R2-T2：终态任务可携带 residue 信息（temp 路径、清理结果）；面板在失败 /
 * 已取消行上原样展示，不假装清理成功。
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
  const baseError = task.error_message ?? null;
  // R2-T2：cleanup 失败时把 temp 路径与清理失败原因一并展示，让用户能
  // 主动去处理残留 —— 不可静默吞掉「清理失败」。
  const residueSuffix =
    task.cleanup_status === "residue" && task.temp_path
      ? `（残留临时文件：${task.temp_path}）`
      : null;
  return {
    id: task.id,
    name: task.remote_path.split("/").pop() || task.remote_path,
    phase: PHASE_BY_STATE[task.state],
    progress: task.total_bytes ? task.bytes_transferred / task.total_bytes : 0,
    size: task.total_bytes,
    local: task.local_path,
    remote: task.remote_path,
    speed: task.state === "Transferring" ? task.speed_bps : 0,
    error: residueSuffix ? `${baseError ?? ""}${residueSuffix}` : baseError,
    cleanup_status: task.cleanup_status ?? null,
    temp_path: task.temp_path ?? null,
    commit_strategy: task.commit_strategy ?? null,
  };
}
