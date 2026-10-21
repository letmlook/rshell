import { afterEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import ElementPlus from "element-plus";
import { nextTick } from "vue";
import { createPinia, setActivePinia } from "pinia";
import SessionCreateDialog from "../../src/components/SessionCreateDialog.vue";
import { useSessionsStore } from "../../src/stores/sessions";

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

// 连接信息（主机/端口/用户名/认证方式/名称）此前没有任何编辑入口：
// 后端 UpdateSession 完整支持，界面却只能用「更新凭据」改密码。
describe("SessionCreateDialog 编辑模式", () => {
  const existing = {
    id: "session-1",
    name: "prod",
    folder_id: null,
    host: "old.example.test",
    port: 2222,
    protocol: "SSH",
    auth_method: { Password: { username: "alice", has_password: true } },
    serial_config: null,
  };

  /** 建好 pinia，把被编辑的连接放进 sessions store，再挂载对话框 */
  function mountEdit() {
    const pinia = createPinia();
    setActivePinia(pinia);
    useSessionsStore().items = [existing as never];
    return mount(SessionCreateDialog, {
      props: { visible: true, editId: "session-1" },
      attachTo: document.body,
      global: { plugins: [pinia, ElementPlus] },
    });
  }

  it("预填已有连接的主机与用户名", async () => {
    const wrapper = mountEdit();
    await nextTick();
    const inputs = Array.from(document.body.querySelectorAll<HTMLInputElement>("input.el-input__inner"));
    const host = inputs.find((input) => input.placeholder === "host or ip");
    expect(host?.value).toBe("old.example.test");
    const username = Array.from(document.body.querySelectorAll(".el-form-item"))
      .find((item) => item.querySelector("label")?.textContent?.includes("用户名"))
      ?.querySelector<HTMLInputElement>("input.el-input__inner");
    expect(username?.value).toBe("alice");
    wrapper.unmount();
  });

  it("凭据留空时保持现有凭据（Keep），不清空", async () => {
    invoke.mockImplementation(async (command: string) =>
      command === "update_session" ? null : command === "list_sessions" ? [existing] : [],
    );
    const wrapper = mountEdit();
    await nextTick();
    const inputs = Array.from(document.body.querySelectorAll<HTMLInputElement>("input.el-input__inner"));
    const host = inputs.find((input) => input.placeholder === "host or ip")!;
    host.value = "new.example.test";
    host.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();

    const saveButton = Array.from(document.body.querySelectorAll("button")).find((button) =>
      button.textContent?.includes("保存"),
    );
    expect(saveButton).toBeDefined();
    saveButton!.click();

    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("update_session", expect.any(Object)));
    const payload = invoke.mock.calls.find(([command]) => command === "update_session")![1];
    expect(payload.id).toBe("session-1");
    expect(payload.config.id).toBe("session-1");
    expect(payload.config.host).toBe("new.example.test");
    expect(payload.credential).toBe("Keep");
    wrapper.unmount();
  });

  it("填了新密码时替换凭据（Set）", async () => {
    invoke.mockImplementation(async (command: string) =>
      command === "update_session" ? null : command === "list_sessions" ? [existing] : [],
    );
    const wrapper = mountEdit();
    await nextTick();
    const password = document.body.querySelector<HTMLInputElement>('input[type="password"]')!;
    password.value = "brand-new-secret";
    password.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();

    const saveButton = Array.from(document.body.querySelectorAll("button")).find((button) =>
      button.textContent?.includes("保存"),
    );
    saveButton!.click();

    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("update_session", expect.any(Object)));
    const payload = invoke.mock.calls.find(([command]) => command === "update_session")![1];
    expect(payload.credential).toEqual({ Set: { secret: "brand-new-secret" } });
    expect(JSON.stringify(payload.config)).not.toContain("brand-new-secret");
    wrapper.unmount();
  });

  it("编辑模式不会新建连接", async () => {
    invoke.mockImplementation(async (command: string) =>
      command === "update_session" ? null : command === "list_sessions" ? [existing] : [],
    );
    const wrapper = mountEdit();
    await nextTick();
    const saveButton = Array.from(document.body.querySelectorAll("button")).find((button) =>
      button.textContent?.includes("保存"),
    );
    saveButton!.click();
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(invoke.mock.calls.some(([command]) => command === "create_session")).toBe(false);
    wrapper.unmount();
  });
});

// 键盘交互认证在后端完整实现（AuthMethod::KeyboardInteractive + SSH 客户端的
// 交互式认证流程），此前界面没有这个认证方式，该能力无法使用。
describe("SessionCreateDialog 键盘交互认证", () => {
  it("可以选择键盘交互认证并提交对应的 auth_method", async () => {
    invoke.mockImplementation(async (command: string) =>
      command === "create_session" ? "new-session-id" : [],
    );
    const wrapper = mount(SessionCreateDialog, {
      props: { visible: true },
      attachTo: document.body,
      global: { plugins: [createPinia(), ElementPlus] },
    });
    await nextTick();

    // 切到「键盘交互」
    const interactive = Array.from(document.body.querySelectorAll(".el-radio-button"))
      .find((item) => item.textContent?.includes("键盘交互"));
    expect(interactive, "认证方式里必须有「键盘交互」").toBeDefined();
    (interactive!.querySelector("input") as HTMLInputElement).click();
    await nextTick();

    const inputs = Array.from(document.body.querySelectorAll<HTMLInputElement>("input.el-input__inner"));
    const host = inputs.find((input) => input.placeholder === "host or ip")!;
    host.value = "example.test";
    host.dispatchEvent(new Event("input", { bubbles: true }));
    const username = Array.from(document.body.querySelectorAll(".el-form-item"))
      .find((item) => item.querySelector("label")?.textContent?.includes("用户名"))
      ?.querySelector<HTMLInputElement>("input.el-input__inner")!;
    username.value = "alice";
    username.dispatchEvent(new Event("input", { bubbles: true }));
    const password = document.body.querySelector<HTMLInputElement>('input[type="password"]')!;
    password.value = "interactive-secret";
    password.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();

    const createButton = Array.from(document.body.querySelectorAll("button")).find((button) =>
      button.textContent?.includes("创建"),
    );
    createButton!.click();
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("create_session", expect.any(Object)));

    const { config, credential } = invoke.mock.calls.find(([command]) => command === "create_session")![1];
    expect(config.auth_method).toEqual({
      KeyboardInteractive: { username: "alice", has_password: true },
    });
    expect(credential).toEqual({ secret: "interactive-secret" });
    expect(JSON.stringify(config)).not.toContain("interactive-secret");
    wrapper.unmount();
  });
});

// 连接分组：SessionConfig.folder_id 指向分组实体，新建/编辑连接时可以选组。
describe("SessionCreateDialog 连接分组", () => {
  it("创建连接时带上选中的分组 id", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "create_session") return "new-session-id";
      if (command === "list_folders") {
        return [
          { id: "folder-1", name: "生产" },
          { id: "folder-2", name: "测试" },
        ];
      }
      return [];
    });
    const wrapper = mount(SessionCreateDialog, {
      props: { visible: true },
      attachTo: document.body,
      global: { plugins: [createPinia(), ElementPlus] },
    });
    await nextTick();

    const folderSelect = document.body.querySelector('[data-test="session-folder"]');
    expect(folderSelect, "新建连接必须能选分组").not.toBeNull();

    // Element Plus 的下拉是 teleport 出去的 popper：先点开再选
    const folderInput = document.body.querySelector<HTMLInputElement>(
      "[data-test='session-folder'] input",
    );
    expect(folderInput).not.toBeNull();
    folderInput!.click();
    await nextTick();
    await nextTick();

    const option = Array.from(
      document.body.querySelectorAll<HTMLElement>(".el-select-dropdown__item"),
    ).find((item) => item.textContent?.includes("生产"));
    expect(option, "分组下拉里应有「生产」").toBeDefined();
    option!.click();
    await nextTick();

    const inputs = Array.from(document.body.querySelectorAll<HTMLInputElement>("input.el-input__inner"));
    const host = inputs.find((input) => input.placeholder === "host or ip")!;
    host.value = "example.test";
    host.dispatchEvent(new Event("input", { bubbles: true }));
    const username = Array.from(document.body.querySelectorAll(".el-form-item"))
      .find((item) => item.querySelector("label")?.textContent?.includes("用户名"))
      ?.querySelector<HTMLInputElement>("input.el-input__inner")!;
    username.value = "alice";
    username.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();

    const createButton = Array.from(document.body.querySelectorAll("button")).find((button) =>
      button.textContent?.includes("创建"),
    );
    createButton!.click();
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("create_session", expect.any(Object)));

    const { config } = invoke.mock.calls.find(([command]) => command === "create_session")![1];
    expect(config.folder_id).toBe("folder-1");
    wrapper.unmount();
  });
});

// 串口设备枚举：用户不该靠手打 /dev/cu.usbserial-XXXX（插拔一次名字就变）。
describe("SessionCreateDialog 串口设备枚举", () => {
  it("列出系统可用串口并可选中", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "create_session") return "new-session-id";
      if (command === "list_serial_ports") return ["/dev/cu.usbserial-A1", "/dev/cu.Bluetooth-Incoming-Port"];
      if (command === "list_folders") return [];
      return [];
    });
    const wrapper = mount(SessionCreateDialog, {
      props: { visible: true },
      attachTo: document.body,
      global: { plugins: [createPinia(), ElementPlus] },
    });
    await nextTick();

    // 切到「串口」协议：Element Plus 的下拉要先点开才会渲染到 body
    const protocolInput = document.body.querySelector<HTMLInputElement>(
      ".el-select__wrapper input, .el-select input",
    );
    expect(protocolInput, "协议下拉要有输入框").not.toBeNull();
    protocolInput!.click();
    await nextTick();
    await nextTick();
    const serialOption = Array.from(
      document.body.querySelectorAll<HTMLElement>(".el-select-dropdown__item"),
    ).find((item) => item.textContent?.includes("串口"));
    expect(serialOption).toBeDefined();
    serialOption!.click();
    await nextTick();
    // 串口枚举是异步 IPC：等它落地
    await flushPromises();

    const portSelect = document.body.querySelector('[data-test="session-serial-port"]');
    expect(portSelect, "串口协议下必须有设备选择器").not.toBeNull();
    expect(document.body.textContent).toContain("/dev/cu.usbserial-A1");
    wrapper.unmount();
  });

  it("枚举失败时仍可手工填写设备路径，并说明原因", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_serial_ports") throw new Error("available_ports failed");
      if (command === "list_folders") return [];
      return [];
    });
    const wrapper = mount(SessionCreateDialog, {
      props: { visible: true },
      attachTo: document.body,
      global: { plugins: [createPinia(), ElementPlus] },
    });
    await nextTick();

    const protocolInput = document.body.querySelector<HTMLInputElement>(
      ".el-select__wrapper input, .el-select input",
    );
    expect(protocolInput).not.toBeNull();
    protocolInput!.click();
    await nextTick();
    await nextTick();
    const serialOption = Array.from(
      document.body.querySelectorAll<HTMLElement>(".el-select-dropdown__item"),
    ).find((item) => item.textContent?.includes("串口"));
    expect(serialOption).toBeDefined();
    serialOption!.click();
    await nextTick();
    await flushPromises();

    expect(document.body.querySelector('[data-test="session-serial-ports-error"]')).not.toBeNull();
    expect(document.body.textContent).toContain("available_ports failed");
    // 仍然保留手工输入框
    expect(
      document.body.querySelector('[data-test="session-serial-port"]'),
    ).not.toBeNull();
    wrapper.unmount();
  });
});
