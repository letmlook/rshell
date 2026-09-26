import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const disconnectSession = vi.fn();
const deleteSession = vi.fn();
const listSessions = vi.fn().mockResolvedValue([]);

vi.mock("../../src/ipc/client", () => ({
  listSessions: (...a: unknown[]) => listSessions(...a),
  createSession: vi.fn(),
  connectSession: vi.fn(),
  disconnectSession: (...a: unknown[]) => disconnectSession(...a),
  deleteSession: (...a: unknown[]) => deleteSession(...a),
}));
vi.mock("../../src/ipc/events", () => ({
  subscribeAppEvents: vi.fn(async () => () => {}),
}));

const { useSessionsStore } = await import("../../src/stores/sessions");

describe("sessions store error surfacing", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    disconnectSession.mockReset();
    deleteSession.mockReset();
    listSessions.mockResolvedValue([]);
  });

  it("records the disconnect failure so the session list can render it", async () => {
    const store = useSessionsStore();
    disconnectSession.mockRejectedValueOnce("backend refused");

    await expect(store.disconnect("session-1")).rejects.toThrow();
    expect(store.error).toContain("backend refused");
  });

  it("records the delete failure instead of failing silently", async () => {
    const store = useSessionsStore();
    deleteSession.mockRejectedValueOnce("not found");

    await expect(store.delete("session-1")).rejects.toThrow();
    expect(store.error).toContain("not found");
  });

  it("clears the previous error after a successful disconnect", async () => {
    const store = useSessionsStore();
    disconnectSession.mockRejectedValueOnce("boom");
    await store.disconnect("session-1").catch(() => {});
    expect(store.error).toContain("boom");

    disconnectSession.mockResolvedValueOnce(undefined);
    await store.disconnect("session-1");
    expect(store.error).toBeNull();
    expect(store.connectionState.get("session-1")).toBe("disconnected");
  });
});
