import { afterEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import ElementPlus from "element-plus";
import { nextTick } from "vue";
import { createPinia } from "pinia";
import SessionCreateDialog from "../../src/components/SessionCreateDialog.vue";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

afterEach(() => {
  document.body.innerHTML = "";
  invoke.mockReset();
});

describe("SessionCreateDialog", () => {
  it("submits an SSH password only in the separate command credential", async () => {
    invoke.mockImplementation(async (command: string) => command === "create_session" ? "new-session-id" : []);
    const wrapper = mount(SessionCreateDialog, {
      props: { visible: true },
      attachTo: document.body,
      global: { plugins: [createPinia(), ElementPlus] },
    });
    await nextTick();
    const inputs = Array.from(document.body.querySelectorAll<HTMLInputElement>("input.el-input__inner"));
    const host = inputs.find((input) => input.placeholder === "host or ip");
    const password = inputs.find((input) => input.type === "password");
    expect(host).toBeDefined();
    expect(password).toBeDefined();
    const username = Array.from(document.body.querySelectorAll(".el-form-item"))
      .find((item) => item.querySelector("label")?.textContent?.includes("用户名"))
      ?.querySelector<HTMLInputElement>("input.el-input__inner");
    expect(username).toBeDefined();
    host!.value = "example.test";
    host!.dispatchEvent(new Event("input", { bubbles: true }));
    username!.value = "alice";
    username!.dispatchEvent(new Event("input", { bubbles: true }));
    password!.value = "sample-secret-password";
    password!.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();
    const createButton = Array.from(document.body.querySelectorAll("button")).find((button) => button.textContent?.includes("创建"));
    expect(createButton).toBeDefined();
    createButton!.click();
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("create_session", expect.any(Object)));
    const { config, credential } = invoke.mock.calls.find(([command]) => command === "create_session")![1];
    expect(config.auth_method).toEqual({ Password: { username: "alice", has_password: true } });
    expect(JSON.stringify(config)).not.toContain("sample-secret-password");
    expect(credential).toEqual({ secret: "sample-secret-password" });
    await vi.waitFor(() => expect(password!.value).toBe(""));
    wrapper.unmount();
  });

  it.each(["cancel", "dialog-close", "parent-hide"])("clears password after %s and reopening", async (dismissal) => {
    const wrapper = mount(SessionCreateDialog, {
      props: { visible: true }, attachTo: document.body,
      global: { plugins: [createPinia(), ElementPlus] },
    });
    await nextTick();
    const password = document.body.querySelector<HTMLInputElement>('input[type="password"]')!;
    password.value = "dismissed-secret";
    password.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();
    if (dismissal === "cancel") {
      Array.from(document.body.querySelectorAll("button")).find((b) => b.textContent?.includes("取消"))!.click();
    } else if (dismissal === "dialog-close") {
      document.body.querySelector<HTMLButtonElement>(".el-dialog__headerbtn")!.click();
    } else {
      await wrapper.setProps({ visible: false });
    }
    await nextTick();
    await wrapper.setProps({ visible: false });
    await wrapper.setProps({ visible: true });
    expect(document.body.querySelector<HTMLInputElement>('input[type="password"]')!.value).toBe("");
    wrapper.unmount();
  });

  it("marks an intentionally empty SSH password as absent and sends no credential", async () => {
    invoke.mockImplementation(async (command: string) => command === "create_session" ? "new-session-id" : []);
    const wrapper = mount(SessionCreateDialog, {
      props: { visible: true },
      attachTo: document.body,
      global: { plugins: [createPinia(), ElementPlus] },
    });
    await nextTick();
    const host = document.body.querySelector<HTMLInputElement>('input[placeholder="host or ip"]');
    const username = Array.from(document.body.querySelectorAll(".el-form-item"))
      .find((item) => item.querySelector("label")?.textContent?.includes("用户名"))
      ?.querySelector<HTMLInputElement>("input.el-input__inner");
    expect(host).toBeDefined();
    expect(username).toBeDefined();
    host!.value = "example.test";
    host!.dispatchEvent(new Event("input", { bubbles: true }));
    username!.value = "alice";
    username!.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();
    const createButton = Array.from(document.body.querySelectorAll("button")).find((button) => button.textContent?.includes("创建"));
    createButton!.click();
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("create_session", expect.any(Object)));
    const { config, credential } = invoke.mock.calls.find(([command]) => command === "create_session")![1];
    expect(config.auth_method).toEqual({ Password: { username: "alice", has_password: false } });
    expect(credential).toBeNull();
    wrapper.unmount();
  });

  it("creates a managed-key session with the key id in config and passphrase only in credential", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_keys") {
        return [{ id: "key-1", name: "deploy", key_type: "ED25519", fingerprint: "SHA256:abc", has_passphrase: true }];
      }
      if (command === "create_session") return "managed-session-id";
      return [];
    });
    const wrapper = mount(SessionCreateDialog, {
      props: { visible: true },
      attachTo: document.body,
      global: { plugins: [createPinia(), ElementPlus] },
    });
    await nextTick();
    const host = document.body.querySelector<HTMLInputElement>('input[placeholder="host or ip"]')!;
    host.value = "example.test";
    host.dispatchEvent(new Event("input", { bubbles: true }));
    const username = Array.from(document.body.querySelectorAll(".el-form-item"))
      .find((item) => item.querySelector("label")?.textContent?.includes("用户名"))!
      .querySelector<HTMLInputElement>("input.el-input__inner")!;
    username.value = "alice";
    username.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("list_keys", null));

    // 切到托管密钥认证
    const managedRadio = document.body.querySelector<HTMLInputElement>(
      'input.el-radio-button__original-radio[value="managed-key"]',
    );
    expect(managedKeyRadioVisible(managedRadio)).toBe(true);
    managedRadio!.click();
    await nextTick();

    // 密钥口令输入框出现（所选密钥 has_passphrase=true）；第一个密钥自动选中
    const passphrase = document.body.querySelector<HTMLInputElement>('input[type="password"]')!;
    expect(passphrase).toBeDefined();
    passphrase.value = "managed-key-pass";
    passphrase.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();

    const createButton = Array.from(document.body.querySelectorAll("button")).find((button) => button.textContent?.includes("创建"))!;
    createButton.click();
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("create_session", expect.any(Object)));
    const { config, credential } = invoke.mock.calls.find(([command]) => command === "create_session")![1];
    expect(config.auth_method).toEqual({
      PublicKey: { username: "alice", key_path: "", has_passphrase: true, managed_key_id: "key-1" },
    });
    expect(JSON.stringify(config)).not.toContain("managed-key-pass");
    expect(credential).toEqual({ secret: "managed-key-pass" });
    wrapper.unmount();
  });

  it("creates a managed-key session without credential when the key has no passphrase", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_keys") {
        return [{ id: "key-2", name: "plain", key_type: "ED25519", fingerprint: "SHA256:def", has_passphrase: false }];
      }
      if (command === "create_session") return "managed-session-id";
      return [];
    });
    const wrapper = mount(SessionCreateDialog, {
      props: { visible: true },
      attachTo: document.body,
      global: { plugins: [createPinia(), ElementPlus] },
    });
    await nextTick();
    const host = document.body.querySelector<HTMLInputElement>('input[placeholder="host or ip"]')!;
    host.value = "example.test";
    host.dispatchEvent(new Event("input", { bubbles: true }));
    const username = Array.from(document.body.querySelectorAll(".el-form-item"))
      .find((item) => item.querySelector("label")?.textContent?.includes("用户名"))!
      .querySelector<HTMLInputElement>("input.el-input__inner")!;
    username.value = "alice";
    username.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("list_keys", null));
    document.body.querySelector<HTMLInputElement>(
      'input.el-radio-button__original-radio[value="managed-key"]',
    )!.click();
    await nextTick();

    const createButton = Array.from(document.body.querySelectorAll("button")).find((button) => button.textContent?.includes("创建"))!;
    createButton.click();
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("create_session", expect.any(Object)));
    const { config, credential } = invoke.mock.calls.find(([command]) => command === "create_session")![1];
    expect(config.auth_method).toEqual({
      PublicKey: { username: "alice", key_path: "", has_passphrase: false, managed_key_id: "key-2" },
    });
    expect(credential).toBeNull();
    wrapper.unmount();
  });

  it("blocks submission when managed-key mode is selected with no key available", async () => {
    invoke.mockImplementation(async (command: string) => command === "create_session" ? "should-not-happen" : []);
    const wrapper = mount(SessionCreateDialog, {
      props: { visible: true },
      attachTo: document.body,
      global: { plugins: [createPinia(), ElementPlus] },
    });
    await nextTick();
    const host = document.body.querySelector<HTMLInputElement>('input[placeholder="host or ip"]')!;
    host.value = "example.test";
    host.dispatchEvent(new Event("input", { bubbles: true }));
    const username = Array.from(document.body.querySelectorAll(".el-form-item"))
      .find((item) => item.querySelector("label")?.textContent?.includes("用户名"))!
      .querySelector<HTMLInputElement>("input.el-input__inner")!;
    username.value = "alice";
    username.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("list_keys", null));
    document.body.querySelector<HTMLInputElement>(
      'input.el-radio-button__original-radio[value="managed-key"]',
    )!.click();
    await nextTick();

    const createButton = Array.from(document.body.querySelectorAll("button")).find((button) => button.textContent?.includes("创建"))!;
    createButton.click();
    await nextTick();
    expect(invoke).not.toHaveBeenCalledWith("create_session", null);
    expect(document.body.textContent).toContain("请选择托管密钥");
    wrapper.unmount();
  });
});

function managedKeyRadioVisible(radio: Element | null): boolean {
  return radio !== null;
}
