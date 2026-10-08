import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const decideHostKey = vi.fn();
const cancelHostKey = vi.fn();
const handlers: Array<(event: unknown) => void> = [];

vi.mock("../../src/ipc/client", () => ({
  decideHostKey: (...args: unknown[]) => decideHostKey(...args),
  cancelHostKey: (...args: unknown[]) => cancelHostKey(...args),
}));
vi.mock("../../src/ipc/events", () => ({
  subscribeAppEvents: vi.fn(async (handler: (event: unknown) => void) => {
    handlers.push(handler);
    return () => {};
  }),
}));

const { useHostKeyStore } = await import("../../src/stores/hostKey");

const NIL_UUID = "00000000-0000-0000-0000-000000000000";

function decisionState(decision_id: string, state: string) {
  return { HostKeyDecisionStateChanged: { decision_id, state } };
}

function mismatch(decision_id: string) {
  return {
    HostKeyMismatch: {
      decision_id,
      host: "example.test",
      port: 22,
      key_type: "ssh-ed25519",
      expected: "",
      received: "SHA256:aaa",
      public_key_blob: "AAAA",
    },
  };
}

describe("hostKey store", () => {
  beforeEach(async () => {
    setActivePinia(createPinia());
    handlers.length = 0;
    decideHostKey.mockReset();
    cancelHostKey.mockReset();
    const store = useHostKeyStore();
    await store.subscribeEvents();
  });

  it("opens the decision dialog for a real decision id", async () => {
    handlers.at(-1)!(mismatch("11111111-1111-1111-1111-111111111111"));
    expect(useHostKeyStore().current?.decision_id).toBe(
      "11111111-1111-1111-1111-111111111111",
    );
  });

  it("ignores a nil decision id so no doomed dialog is shown", async () => {
    handlers.at(-1)!(mismatch(NIL_UUID));
    expect(useHostKeyStore().current).toBeNull();
  });

  it("keeps the dialog open and records the error when the decision fails", async () => {
    handlers.at(-1)!(mismatch("22222222-2222-2222-2222-222222222222"));
    const store = useHostKeyStore();
    expect(store.current).not.toBeNull();

    decideHostKey.mockRejectedValueOnce("decision rejected");
    await store.trustPermanent();

    expect(store.current).not.toBeNull();
    expect(store.error).toContain("decision rejected");
    expect(store.history).toHaveLength(0);
    expect(decideHostKey).toHaveBeenCalledWith(
      "22222222-2222-2222-2222-222222222222",
      true,
      true,
    );
  });

  it("keeps concurrent requests in a map and cancelling one does not affect the other", async () => {
    const first = "44444444-4444-4444-4444-444444444444";
    const second = "55555555-5555-5555-5555-555555555555";
    handlers.at(-1)!(mismatch(first));
    handlers.at(-1)!(mismatch(second));
    const store = useHostKeyStore();
    cancelHostKey.mockResolvedValueOnce(undefined);

    expect(store.requests.size).toBe(2);
    await store.cancel(first);
    expect(cancelHostKey).toHaveBeenCalledWith(first);
    expect(store.requests.has(first)).toBe(false);
    expect(store.requests.has(second)).toBe(true);
    expect(store.current?.decision_id).toBe(second);
  });

  it("drops state events for unknown or expired ids without clearing valid decisions", () => {
    const valid = "66666666-6666-6666-6666-666666666666";
    handlers.at(-1)!(mismatch(valid));
    const store = useHostKeyStore();
    handlers.at(-1)!(decisionState("77777777-7777-7777-7777-777777777777", "Expired"));
    expect(store.requests.has(valid)).toBe(true);
    handlers.at(-1)!(decisionState(valid, "Expired"));
    expect(store.requests.has(valid)).toBe(false);
    expect(store.requests.size).toBe(0);
  });

  it("keeps the request available after permanent trust persistence fails so trust-once remains possible", async () => {
    const id = "88888888-8888-8888-8888-888888888888";
    handlers.at(-1)!(mismatch(id));
    const store = useHostKeyStore();
    decideHostKey.mockRejectedValueOnce("known_hosts is read-only");
    await store.trustPermanent();
    expect(store.requests.has(id)).toBe(true);
    expect(store.error).toContain("read-only");
    decideHostKey.mockResolvedValueOnce(undefined);
    await store.trustOnce();
    expect(decideHostKey).toHaveBeenLastCalledWith(id, true, false);
    expect(store.requests.has(id)).toBe(false);
  });

  it("double-cancel is idempotent: the second call neither dispatches nor surfaces an error", async () => {
    const id = "99999999-9999-9999-9999-999999999999";
    handlers.at(-1)!(mismatch(id));
    const store = useHostKeyStore();
    cancelHostKey.mockResolvedValueOnce(undefined);

    // 第一次取消：派发后端，移除请求，错误被清掉
    await store.cancel(id);
    expect(cancelHostKey).toHaveBeenCalledTimes(1);
    expect(store.requests.has(id)).toBe(false);
    expect(store.error).toBeNull();

    // 第二次取消：id 已不在 store，跳过派发、不写错误，意图已被满足
    await store.cancel(id);
    expect(cancelHostKey).toHaveBeenCalledTimes(1);
    expect(store.error).toBeNull();
  });

  it("cancel on an id removed by an Expired event is a no-op (no dispatch, no error)", async () => {
    const id = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    handlers.at(-1)!(mismatch(id));
    const store = useHostKeyStore();
    // 后端超时先把请求从 store 移除
    handlers.at(-1)!(decisionState(id, "Expired"));
    expect(store.requests.has(id)).toBe(false);

    // 残留 UI 触发的取消：不应再发到后端，也不应写错误
    await store.cancel(id);
    expect(cancelHostKey).not.toHaveBeenCalled();
    expect(store.error).toBeNull();
  });
});
