import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const disconnectSession = vi.fn();
const deleteSession = vi.fn();
const createSession = vi.fn();
const listSessions = vi.fn().mockResolvedValue([]);

vi.mock("../../src/ipc/client", () => ({
  listSessions: (...a: unknown[]) => listSessions(...a),
  listSessionLoadIssues: vi.fn().mockResolvedValue([]),
  retrySessionLoad: vi.fn(),
  updateSession: vi.fn(),
  createSession: (...a: unknown[]) => createSession(...a),
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

/**
 * 复制会话：后端 CreateSession 直接采用 config.id（重复 id 会被拒为
 * "Session already exists"），所以复制必须自带新 id；凭据存钥匙串、不随
 * 配置复制，因此入队时 credential 必须是 null。
 */
describe("sessions store duplicate", () => {
  const base = {
    id: "11111111-1111-4111-8111-111111111111",
    name: "prod-web",
    folder_id: null,
    host: "10.0.0.9",
    port: 22,
    auth_method: { Password: { username: "ops", has_password: true } },
    protocol: "SSH",
  };

  beforeEach(() => {
    setActivePinia(createPinia());
    createSession.mockReset();
    listSessions.mockResolvedValue([base]);
  });

  it("creates a new entry with a fresh id and the same connection info", async () => {
    createSession.mockResolvedValueOnce("new-id");
    const store = useSessionsStore();
    await store.refresh();

    const newId = await store.duplicate(base.id);

    expect(newId).toBe("new-id");
    expect(createSession).toHaveBeenCalledOnce();
    const [config, credential] = createSession.mock.calls[0] as unknown as [typeof base, unknown];
    expect(config.id).not.toBe(base.id);
    expect(config.host).toBe(base.host);
    expect(config.port).toBe(base.port);
    expect(config.auth_method).toEqual(base.auth_method);
    expect(config.name).toBe("prod-web 副本");
    // 凭据不复制：钥匙串里的明文不回读，也就没有「复制凭据」这回事
    expect(credential).toBeNull();
  });

  it("名称撞车时自动加序号，不会静默覆盖已有会话", async () => {
    listSessions.mockResolvedValue([base, { ...base, id: "22222222-2222-4222-8222-222222222222", name: "prod-web 副本" }]);
    createSession.mockResolvedValueOnce("new-id-2");
    const store = useSessionsStore();
    await store.refresh();

    await store.duplicate(base.id);

    const [config] = createSession.mock.calls[0] as unknown as [typeof base];
    expect(config.name).toBe("prod-web 副本 2");
  });

  it("源会话不存在时报错，不用假数据兜底", async () => {
    const store = useSessionsStore();
    await store.refresh();

    await expect(store.duplicate("does-not-exist")).rejects.toThrow(/不存在/);
    expect(createSession).not.toHaveBeenCalled();
  });
});
