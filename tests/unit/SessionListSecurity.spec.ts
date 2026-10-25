import { afterEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia } from "pinia";
import ElementPlus from "element-plus";
import SessionList from "../../src/components/SessionList.vue";
import { useSessionsStore } from "../../src/stores/sessions";
import type { SessionConfig } from "../../src/ipc/types";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const session: SessionConfig = {
  id: "saved-session", name: "Production SSH", folder_id: null,
  host: "example.test", port: 22, protocol: "SSH",
  auth_method: { Password: { username: "alice", has_password: true } }, serial_config: null,
};
const wrappers: ReturnType<typeof mount>[] = [];
afterEach(() => {
  wrappers.splice(0).forEach((w) => w.unmount());
  document.body.innerHTML = "";
  invoke.mockReset();
});
function setup() {
  const pinia = createPinia();
  const wrapper = mount(SessionList, { attachTo: document.body, global: { plugins: [pinia, ElementPlus] } });
  wrappers.push(wrapper);
  return { wrapper, store: useSessionsStore(pinia) };
}
function button(text: string) {
  return Array.from(document.body.querySelectorAll("button")).find((b) => b.textContent?.includes(text));
}

describe("session credential recovery", () => {
  it("shows persisted startup failures and retries after Keychain access is restored", async () => {
    let retried = false;
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") return retried ? [session] : [];
      if (command === "list_session_load_issues") return retried ? [] : [{ session_id: "legacy-id", kind: "CredentialMigrationFailed", message: "Could not load or migrate saved session", retryable: true }];
      if (command === "list_folders") return [];
      if (command === "retry_session_load") { retried = true; return; }
      throw new Error(`Unexpected command ${command}`);
    });
    const { wrapper, store } = setup();
    await store.refresh();
    await flushPromises();
    expect(wrapper.text()).toContain("legacy-id");
    expect(wrapper.text()).toContain("Could not load or migrate saved session");
    await store.refresh();
    expect(wrapper.text()).toContain("legacy-id");
    expect(button("重试加载")).toBeDefined();
    button("重试加载")!.click();
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("retry_session_load", null);
    expect(wrapper.text()).not.toContain("legacy-id");
    expect(wrapper.text()).toContain("Production SSH");
  });

  it("gives each load-issue kind its own cause instead of pointing every failure at the keychain", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") return [];
      if (command === "list_session_load_issues") return [
        { session_id: null, kind: "StorageUnavailable", message: "Could not read saved sessions.", retryable: true },
        { session_id: "legacy-id", kind: "CredentialMigrationFailed", message: "credential store unavailable", retryable: true },
      ];
      if (command === "list_folders") return [];
      throw new Error(`Unexpected command ${command}`);
    });
    const { wrapper, store } = setup();
    await store.refresh();
    await flushPromises();
    // 存储读不到：引导去查存储目录，而不是钥匙串
    expect(wrapper.text()).toContain("连接存储不可用");
    expect(wrapper.text()).toContain("会话存储目录");
    // 凭据迁移失败：引导去查钥匙串
    expect(wrapper.text()).toContain("凭据迁移失败");
    expect(wrapper.text()).toContain("钥匙串");
    // 后端 message 作为技术细节原样保留
    expect(wrapper.text()).toContain("Could not read saved sessions.");
    // 两类都可重试 → 有重试入口
    expect(button("重试加载")).toBeDefined();
  });

  it("does not offer a retry when the backend marked the issue as not retryable", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") return [];
      if (command === "list_session_load_issues") return [
        { session_id: "legacy-id", kind: "CredentialMigrationFailed", message: "The credential entry is missing.", retryable: false },
      ];
      if (command === "list_folders") return [];
      throw new Error(`Unexpected command ${command}`);
    });
    const { wrapper, store } = setup();
    await store.refresh();
    await flushPromises();
    expect(wrapper.text()).toContain("凭据迁移失败");
    expect(button("重试加载")).toBeUndefined();
  });

  it.each(["Password", "PublicKey", "KeyboardInteractive"] as const)("updates an existing %s session with only a newly entered secret", async (auth) => {
    const config: SessionConfig = { ...session, auth_method: auth === "PublicKey"
      ? { PublicKey: { username: "alice", key_path: "/tmp/id", has_passphrase: true } }
      : auth === "KeyboardInteractive" ? { KeyboardInteractive: { username: "alice", has_password: true } } : session.auth_method };
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") return [config];
      if (command === "list_session_load_issues") return [];
      if (command === "list_folders") return [];
      if (command === "update_session") return;
      throw new Error(`Unexpected command ${command}`);
    });
    const { wrapper, store } = setup();
    await store.refresh();
    await wrapper.find(".leaf").trigger("contextmenu");
    expect(button("更新凭据")).toBeDefined();
    button("更新凭据")!.click();
    await flushPromises();
    const password = document.body.querySelector<HTMLInputElement>('input[type="password"]')!;
    expect(password.value).toBe("");
    password.value = "replacement-secret";
    password.dispatchEvent(new Event("input", { bubbles: true }));
    await flushPromises();
    button("保存凭据")!.click();
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("update_session", {
      id: "saved-session", config, credential: { Set: { secret: "replacement-secret" } },
    });
    expect(JSON.stringify(store.items)).not.toContain("replacement-secret");
    expect(document.body.querySelector('input[type="password"]')).toBeNull();
    await wrapper.find(".leaf").trigger("contextmenu");
    button("更新凭据")!.click();
    await flushPromises();
    expect(document.body.querySelector<HTMLInputElement>('input[type="password"]')!.value).toBe("");
    expect(invoke.mock.calls.every(([command]) => ["list_sessions", "list_session_load_issues", "list_folders", "update_session"].includes(command))).toBe(true);
  });

  it("keeps a failed credential save visible and preserves the entered value until dismissal", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") return [session];
      if (command === "list_session_load_issues") return [];
      if (command === "update_session") throw new Error("credential store unavailable");
      throw new Error(`Unexpected command ${command}`);
    });
    const { wrapper, store } = setup();
    await store.refresh();
    await wrapper.find(".leaf").trigger("contextmenu");
    button("更新凭据")!.click();
    await flushPromises();
    expect(button("保存凭据")!.disabled).toBe(true);
    const input = document.body.querySelector<HTMLInputElement>('input[type="password"]')!;
    input.value = "unsaved-secret";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    await flushPromises();
    button("保存凭据")!.click();
    await flushPromises();
    expect(document.body.textContent).toContain("credential store unavailable");
    expect(document.body.querySelector<HTMLInputElement>('input[type="password"]')!.value).toBe("unsaved-secret");
    expect(JSON.stringify(store.items)).not.toContain("unsaved-secret");
    const updateCallsBeforeCancel = invoke.mock.calls.filter(([command]) => command === "update_session").length;
    button("取消")!.click();
    await flushPromises();
    expect(document.body.querySelector('input[type="password"]')).toBeNull();
    await wrapper.find(".leaf").trigger("contextmenu");
    button("更新凭据")!.click();
    await flushPromises();
    expect(document.body.querySelector<HTMLInputElement>('input[type="password"]')!.value).toBe("");
    expect(invoke.mock.calls.filter(([command]) => command === "update_session")).toHaveLength(updateCallsBeforeCancel);
  });
});
