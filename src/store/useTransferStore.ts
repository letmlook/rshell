// src/store/useTransferStore.ts - 传输队列状态

import { create } from "zustand";
import type { TransferDirection, Uuid } from "../ipc/types";

export interface TransferTask {
  task_id: Uuid;
  filename: string;
  direction: TransferDirection;
  bytes_transferred: number;
  total_bytes: number;
  speed_bps: number;
  state: "queued" | "active" | "paused" | "completed" | "failed";
  error?: string;
}

interface TransferState {
  tasks: TransferTask[];
  upsertTask: (task: TransferTask) => void;
  updateProgress: (task_id: Uuid, bytes: number, total: number, speed_bps: number) => void;
  markCompleted: (task_id: Uuid) => void;
  markFailed: (task_id: Uuid, error: string) => void;
  removeTask: (task_id: Uuid) => void;
  clear: () => void;
}

export const useTransferStore = create<TransferState>((set) => ({
  tasks: [],
  upsertTask: (task) =>
    set((s) => {
      const idx = s.tasks.findIndex((t) => t.task_id === task.task_id);
      if (idx >= 0) {
        const next = [...s.tasks];
        next[idx] = { ...next[idx], ...task };
        return { tasks: next };
      }
      return { tasks: [...s.tasks, task] };
    }),
  updateProgress: (task_id, bytes, total, speed_bps) =>
    set((s) => ({
      tasks: s.tasks.map((t) =>
        t.task_id === task_id
          ? { ...t, bytes_transferred: bytes, total_bytes: total, speed_bps, state: "active" }
          : t,
      ),
    })),
  markCompleted: (task_id) =>
    set((s) => ({
      tasks: s.tasks.map((t) =>
        t.task_id === task_id ? { ...t, state: "completed", bytes_transferred: t.total_bytes } : t,
      ),
    })),
  markFailed: (task_id, error) =>
    set((s) => ({
      tasks: s.tasks.map((t) =>
        t.task_id === task_id ? { ...t, state: "failed", error } : t,
      ),
    })),
  removeTask: (task_id) =>
    set((s) => ({ tasks: s.tasks.filter((t) => t.task_id !== task_id) })),
  clear: () => set({ tasks: [] }),
}));