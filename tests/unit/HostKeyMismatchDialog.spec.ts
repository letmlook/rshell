import { beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";

vi.mock("../../src/ipc/client", () => ({ decideHostKey: vi.fn() }));
vi.mock("../../src/ipc/events", () => ({
  subscribeAppEvents: vi.fn(async () => () => {}),
}));

const { useHostKeyStore } = await import("../../src/stores/hostKey");
import HostKeyMismatchDialog from "../../src/components/HostKeyMismatchDialog.vue";

const dialogStub = {
  template: "<div><slot /><footer><slot name=\"footer\" /></footer></div>",
};

function mountDialog() {
  return mount(HostKeyMismatchDialog, {
    global: { stubs: { "el-dialog": dialogStub, "el-button": true } },
  });
}

describe("HostKeyMismatchDialog", () => {
  beforeEach(() => setActivePinia(createPinia()));

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
});
