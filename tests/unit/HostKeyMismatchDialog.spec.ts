import { beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";

const decideHostKey = vi.fn();
const cancelHostKey = vi.fn();
vi.mock("../../src/ipc/client", () => ({
  decideHostKey: (...args: unknown[]) => decideHostKey(...args),
  cancelHostKey: (...args: unknown[]) => cancelHostKey(...args),
}));
vi.mock("../../src/ipc/events", () => ({
  subscribeAppEvents: vi.fn(async () => () => {}),
}));

const { useHostKeyStore } = await import("../../src/stores/hostKey");
import HostKeyMismatchDialog from "../../src/components/HostKeyMismatchDialog.vue";

const dialogStub = {
  template: "<div><slot /></div>",
};
const buttonStub = {
  template: "<button type=\"button\" @click=\"$emit('click')\"><slot /></button>",
};

function mountDialog() {
  return mount(HostKeyMismatchDialog, {
    global: { stubs: { "el-dialog": dialogStub, "el-button": buttonStub } },
  });
}

describe("HostKeyMismatchDialog", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    decideHostKey.mockReset();
    cancelHostKey.mockReset();
  });

  it("renders nothing when there is no pending request", () => {
    expect(mountDialog().text()).toBe("");
  });

  it("surfaces the store error so a failed decision is visible", async () => {
    const store = useHostKeyStore();
    store.current = {
      decision_id: "44444444-4444-4444-4444-444444444444",
      host: "example.test",
      port: 22,
      key_type: "ssh-ed25519",
      expected: "",
      received: "SHA256:aaa",
      public_key_blob: "AAAA",
    };
    store.error = "Host key decision is no longer pending";

    const text = mountDialog().text();
    expect(text).toContain("example.test");
    expect(text).toContain("Host key decision is no longer pending");
  });

  it("explicit cancel closes without submitting an accepting decision", async () => {
    const store = useHostKeyStore();
    const id = "55555555-5555-5555-5555-555555555555";
    store.requests.set(id, {
      decision_id: id,
      host: "cancel.test",
      port: 22,
      key_type: "ssh-ed25519",
      expected: "",
      received: "SHA256:bbb",
      public_key_blob: "BBBB",
    });
    store.current = store.requests.get(id)!;
    cancelHostKey.mockResolvedValueOnce(undefined);
    const wrapper = mountDialog();
    const cancel = wrapper.findAll("button").find((button) => button.text().includes("取消连接"));
    expect(cancel).toBeDefined();
    await cancel!.trigger("click");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(cancelHostKey).toHaveBeenCalledWith(id);
    expect(decideHostKey).not.toHaveBeenCalled();
    expect(store.requests.has(id)).toBe(false);
  });
});
