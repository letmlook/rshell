// TransferQueue - 文件传输队列
// 设计规范 §8.2.4.4
// 标题: 传输队列 (N/M) ↑X/s ↓Y/s
// 任务行: 文件名 + 进度条 + 速度 + 操作按钮
// 状态颜色: 品牌色(传输中) / 绿(完成) / 红(失败)

import { useTransferStore } from "../store/useTransferStore";

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function formatSpeed(bps: number): string {
  return `${formatBytes(bps)}/s`;
}

export function TransferQueue() {
  const tasks = useTransferStore((s) => s.tasks);

  const total = tasks.length;
  const active = tasks.filter((t) => t.state === "active" || t.state === "queued").length;
  const totalUp = tasks
    .filter((t) => t.direction === "Upload" && t.state === "active")
    .reduce((sum, t) => sum + t.speed_bps, 0);
  const totalDown = tasks
    .filter((t) => t.direction === "Download" && t.state === "active")
    .reduce((sum, t) => sum + t.speed_bps, 0);

  return (
    <div className="transfer-queue">
      <div className="transfer-header">
        <span>
          传输队列 ({active}/{total})
        </span>
        <span className="transfer-header-meta">
          ↑ {formatSpeed(totalUp)} ↓ {formatSpeed(totalDown)}
        </span>
      </div>
      <div className="transfer-list">
        {tasks.length === 0 ? (
          <div
            style={{
              padding: 8,
              color: "var(--text-disabled)",
              fontSize: 12,
              textAlign: "center",
            }}
          >
            暂无传输任务
          </div>
        ) : (
          tasks.map((task) => {
            const progress =
              task.total_bytes > 0
                ? (task.bytes_transferred / task.total_bytes) * 100
                : 0;
            const barClass =
              task.state === "completed"
                ? "complete"
                : task.state === "failed"
                  ? "failed"
                  : "";
            return (
              <div key={task.task_id} className="transfer-item">
                <span>{task.direction === "Upload" ? "↑" : "↓"}</span>
                <span className="transfer-item-name" title={task.filename}>
                  {task.filename}
                </span>
                <div className="transfer-progress">
                  <div
                    className={`transfer-progress-bar ${barClass}`}
                    style={{ width: `${progress}%` }}
                  />
                </div>
                <span className="transfer-speed">
                  {task.state === "completed"
                    ? "完成"
                    : task.state === "failed"
                      ? "失败"
                      : task.state === "paused"
                        ? "已暂停"
                        : task.state === "queued"
                          ? "等待"
                          : formatSpeed(task.speed_bps)}
                </span>
                <div className="transfer-actions">
                  {task.state === "active" && (
                    <button className="transfer-btn" title="暂停">
                      ⏸
                    </button>
                  )}
                  {task.state === "paused" && (
                    <button className="transfer-btn" title="恢复">
                      ▶
                    </button>
                  )}
                  <button className="transfer-btn" title="取消">
                    ×
                  </button>
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}