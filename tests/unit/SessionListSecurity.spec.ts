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
      if (command === "list_session_load_issues") return retried ? [] : [{ session_id: "legacy-id", message: "Could not load or migrate saved session" }];
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

  it.each(["Password", "PublicKey", "KeyboardInteractive"] as const)("updates an existing %s session with only a newly entered secret", async (auth) => {
    const config: SessionConfig = { ...session, auth_method: auth === "PublicKey"
      ? { PublicKey: { username: "alice", key_path: "/tmp/id", has_passphrase: true } }
      : auth === "KeyboardInteractive" ? { KeyboardInteractive: { username: "alice", has_password: true } } : session.auth_method };
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") return [config];
      if (command === "list_session_load_issues") return [];
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
    expect(invoke.mock.calls.every(([command]) => ["list_sessions", "list_session_load_issues", "update_session"].includes(command))).toBe(true);
  });

  it("keeps a failed credential save visible and clears the new input on dismissal", async () => {
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
    expect(JSON.stringify(store.items)).not.toContain("unsaved-secret");
    button("取消")!.click();
    await flushPromises();
    expect(document.body.querySelector('input[type="password"]')).toBeNull();
    await wrapper.find(".leaf").trigger("contextmenu");
    button("更新凭据")!.click();
    await flushPromises();
    expect(document.body.querySelector<HTMLInputElement>('input[type="password"]')!.value).toBe("");
  });
});
