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
});
