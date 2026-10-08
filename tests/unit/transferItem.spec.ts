import { describe, expect, it } from "vitest";
import { toTransferItem } from "../../src/utils/transferItem";
import type { TransferTaskInfo } from "../../src/ipc/types";

function task(overrides: Partial<TransferTaskInfo> = {}): TransferTaskInfo {
  return {
    id: "task-1",
    session_id: "session-1",
    direction: "Upload",
    local_path: "/Users/test/a.bin",
    remote_path: "/data/a.bin",
    state: "Transferring",
    bytes_transferred: 0,
    total_bytes: 0,
    speed_bps: 0,
    error_message: null,
    temp_path: null,
    cleanup_status: null,
    commit_strategy: null,
    ...overrides,
  };
}

describe("toTransferItem", () => {
  it("maps live backend speed and progress for a transferring task", () => {
    const row = toTransferItem(
      task({ bytes_transferred: 500, total_bytes: 1000, speed_bps: 1000 }),
    );
    expect(row.phase).toBe("active");
    expect(row.progress).toBeCloseTo(0.5);
    expect(row.speed).toBe(1000);
  });

  it("zeroes speed for terminal states so the panel shows an em dash", () => {
    expect(
      toTransferItem(
        task({ state: "Completed", bytes_transferred: 10, total_bytes: 10, speed_bps: 999 }),
      ).speed,
    ).toBe(0);
    expect(toTransferItem(task({ state: "Failed", error_message: "boom" })).phase).toBe("failed");
    expect(toTransferItem(task({ state: "Failed", error_message: "boom" })).error).toBe("boom");
  });

  it("never divides by zero when total is still unknown", () => {
    expect(
      toTransferItem(task({ bytes_transferred: 0, total_bytes: 0 })).progress,
    ).toBe(0);
  });

  it("uses the remote file name as display name", () => {
    expect(toTransferItem(task()).name).toBe("a.bin");
  });

  // ── R2-T2：residue / commit_strategy 透传 ──

  it("failed task with residue: error includes the temp path so the user knows where to look", () => {
    const row = toTransferItem(
      task({
        state: "Failed",
        error_message: "commit failed: server unreachable",
        cleanup_status: "residue",
        temp_path: "/remote/a.bin.partial-task-uuid",
      }),
    );
    expect(row.error).toContain("commit failed");
    expect(row.error).toContain("/remote/a.bin.partial-task-uuid");
    expect(row.cleanup_status).toBe("residue");
    expect(row.temp_path).toBe("/remote/a.bin.partial-task-uuid");
  });

  it("cleaned failure: error keeps the original cause, no residue appended", () => {
    const row = toTransferItem(
      task({
        state: "Failed",
        error_message: "boom",
        cleanup_status: "cleaned",
      }),
    );
    expect(row.error).toBe("boom");
    expect(row.cleanup_status).toBe("cleaned");
  });

  it("Completed task surfaces commit_strategy so the panel can tell the user what atomicity we got", () => {
    const row = toTransferItem(
      task({
        state: "Completed",
        cleanup_status: "cleaned",
        commit_strategy: "standard_rename",
      }),
    );
    expect(row.commit_strategy).toBe("standard_rename");
  });

  it("active task: cleanup_status / commit_strategy are null (nothing to report yet)", () => {
    const row = toTransferItem(task({ state: "Transferring" }));
    expect(row.cleanup_status).toBeNull();
    expect(row.temp_path).toBeNull();
    expect(row.commit_strategy).toBeNull();
  });
});
