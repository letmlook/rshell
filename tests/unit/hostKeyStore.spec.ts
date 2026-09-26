import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const decideHostKey = vi.fn();
const handlers: Array<(event: unknown) => void> = [];

vi.mock("../../src/ipc/client", () => ({
  decideHostKey: (...args: unknown[]) => decideHostKey(...args),
}));
vi.mock("../../src/ipc/events", () => ({
  subscribeAppEvents: vi.fn(async (handler: (event: unknown) => void) => {
    handlers.push(handler);
    return () => {};
  }),
}));

const { useHostKeyStore } = await import("../../src/stores/hostKey");

const NIL_UUID = "00000000-0000-0000-0000-000000000000";

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

  it("clears the dialog only after a successful decision", async () => {
    handlers.at(-1)!(mismatch("33333333-3333-3333-3333-333333333333"));
    const store = useHostKeyStore();
    decideHostKey.mockResolvedValueOnce(undefined);

    await store.reject();

    expect(store.current).toBeNull();
    expect(store.error).toBeNull();
    expect(store.history).toHaveLength(1);
  });
});
