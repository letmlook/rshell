import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import MasterPasswordPanel from "../../src/components/MasterPasswordPanel.vue";
import MasterPasswordLockOverlay from "../../src/components/MasterPasswordLockOverlay.vue";
import { useMasterPasswordStore } from "../../src/stores/masterPassword";

const {
  setupMasterPassword,
  verifyMasterPassword,
  changeMasterPassword,
  disableMasterPassword,
  lockMasterPassword,
  getMasterPasswordState,
} = vi.hoisted(() => ({
  setupMasterPassword: vi.fn(),
  verifyMasterPassword: vi.fn(),
  changeMasterPassword: vi.fn(),
  disableMasterPassword: vi.fn(),
  lockMasterPassword: vi.fn(),
  getMasterPasswordState: vi.fn(),
}));

vi.mock("../../src/ipc/client", () => ({
  setupMasterPassword,
  verifyMasterPassword,
  changeMasterPassword,
  disableMasterPassword,
  lockMasterPassword,
  getMasterPasswordState,
  ipcErrorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));

vi.mock("../../src/utils/dialog", () => ({
  confirmDialog: vi.fn().mockResolvedValue(true),
}));

// 用真实 Pinia（而不是 mock 掉 store）：面板的状态迁移
// （NotSet → Unlocked → Locked）正是这次要验证的东西。
function installPinia() {
  const pinia = createPinia();
  setActivePinia(pinia);
  return pinia;
}

/** 面板用的最小 stub 集：只需要能输入、能点按钮 */
const stubs = {
  // 表单必须把 submit 事件送回组件：面板的 @submit.prevent="onSetup" 才是真实入口
  "el-form": {
    emits: ["submit"],
    template: '<form @submit.prevent="$emit(\'submit\', $event)"><slot /></form>',
  },
  "el-form-item": { props: ["label"], template: '<div><slot /></div>' },
  "el-input": {
    props: ["modelValue", "type"],
    emits: ["update:modelValue"],
    template:
      '<input :type="type === \'password\' ? \'password\' : \'text\'" :value="modelValue" @input="$emit(\'update:modelValue\', $event.target.value)" />',
  },
  "el-button": {
    props: ["nativeType"],
    template: '<button :type="nativeType === \'submit\' ? \'submit\' : \'button\'"><slot /></button>',
  },
};

/** 遮罩用的是原生 input + button，不需要 Element Plus */
describe("MasterPasswordPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installPinia();
    getMasterPasswordState.mockResolvedValue("NotSet");
  });

  it("explains the scope boundary instead of claiming it protects the keychain", async () => {
    const wrapper = mount(MasterPasswordPanel, { global: { stubs } });
    await flushPromises();
    const text = wrapper.text();
    expect(text).toContain("锁定本应用");
    expect(text).toContain("不会加密");
    expect(text).toContain("钥匙串");
    wrapper.unmount();
  });

  it("sets the master password after both entries match", async () => {
    setupMasterPassword.mockResolvedValue(undefined);
    const wrapper = mount(MasterPasswordPanel, { global: { stubs } });
    await flushPromises();

    await wrapper.get('[data-test="master-password-new"]').setValue("correct horse");
    await wrapper.get('[data-test="master-password-confirm"]').setValue("correct horse");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(setupMasterPassword).toHaveBeenCalledWith("correct horse");
    expect(wrapper.get('[data-test="master-password-state"]').text()).toContain("已解锁");
    wrapper.unmount();
  });

  it("refuses a mismatched confirmation without calling the backend", async () => {
    const wrapper = mount(MasterPasswordPanel, { global: { stubs } });
    await flushPromises();

    await wrapper.get('[data-test="master-password-new"]').setValue("correct horse");
    await wrapper.get('[data-test="master-password-confirm"]').setValue("other horse");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(setupMasterPassword).not.toHaveBeenCalled();
    expect(wrapper.get('[data-test="master-password-notice"]').text()).toContain("不一致");
    wrapper.unmount();
  });

  it("rejects a too-short password without calling the backend", async () => {
    const wrapper = mount(MasterPasswordPanel, { global: { stubs } });
    await flushPromises();

    await wrapper.get('[data-test="master-password-new"]').setValue("short");
    await wrapper.get('[data-test="master-password-confirm"]').setValue("short");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(setupMasterPassword).not.toHaveBeenCalled();
    expect(wrapper.get('[data-test="master-password-notice"]').text()).toContain("8 个字符");
    wrapper.unmount();
  });

  it("offers lock / change / disable once a master password exists", async () => {
    getMasterPasswordState.mockResolvedValue("Unlocked");
    lockMasterPassword.mockResolvedValue(undefined);
    const wrapper = mount(MasterPasswordPanel, { global: { stubs } });
    await flushPromises();

    expect(wrapper.find('[data-test="master-password-lock"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="master-password-change"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="master-password-disable"]').exists()).toBe(true);
    // 已设置时不再显示「设置」表单
    expect(wrapper.find('[data-test="master-password-setup"]').exists()).toBe(false);

    await wrapper.get('[data-test="master-password-lock"]').trigger("click");
    await flushPromises();
    expect(lockMasterPassword).toHaveBeenCalled();
    expect(wrapper.get('[data-test="master-password-state"]').text()).toContain("已锁定");
    wrapper.unmount();
  });

  it("surfaces the backend error when the setup fails", async () => {
    setupMasterPassword.mockRejectedValue(new Error("master password file is locked"));
    const wrapper = mount(MasterPasswordPanel, { global: { stubs } });
    await flushPromises();

    await wrapper.get('[data-test="master-password-new"]').setValue("correct horse");
    await wrapper.get('[data-test="master-password-confirm"]').setValue("correct horse");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(wrapper.get('[data-test="master-password-error"]').text()).toContain(
      "master password file is locked",
    );
    // 失败后不能显示为已解锁
    expect(wrapper.get('[data-test="master-password-state"]').text()).toContain("未设置");
    wrapper.unmount();
  });
});

describe("MasterPasswordLockOverlay", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installPinia();
    getMasterPasswordState.mockResolvedValue("Locked");
  });

  it("shows the lock overlay while the master password is locked", async () => {
    const store = useMasterPasswordStore();
    store.state = "Locked";
    const wrapper = mount(MasterPasswordLockOverlay);
    await flushPromises();
    expect(wrapper.find('[data-test="master-password-lock-overlay"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("RShell 已锁定");
    wrapper.unmount();
  });

  it("unlocks on a correct password and hides the overlay", async () => {
    verifyMasterPassword.mockResolvedValue(true);
    const store = useMasterPasswordStore();
    store.state = "Locked";
    const wrapper = mount(MasterPasswordLockOverlay);
    await flushPromises();

    await wrapper.get('[data-test="master-password-unlock-input"]').setValue("secret");
    await wrapper.get('[data-test="master-password-unlock"]').trigger("click");
    await flushPromises();

    expect(verifyMasterPassword).toHaveBeenCalledWith("secret");
    expect(wrapper.find('[data-test="master-password-lock-overlay"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("keeps the overlay and shows the reason when the password is wrong", async () => {
    verifyMasterPassword.mockResolvedValue(false);
    const store = useMasterPasswordStore();
    store.state = "Locked";
    const wrapper = mount(MasterPasswordLockOverlay);
    await flushPromises();

    await wrapper.get('[data-test="master-password-unlock-input"]').setValue("wrong");
    await wrapper.get('[data-test="master-password-unlock"]').trigger("click");
    await flushPromises();

    expect(wrapper.find('[data-test="master-password-lock-overlay"]').exists()).toBe(true);
    expect(wrapper.get('[data-test="master-password-unlock-error"]').text()).toContain("不正确");
    wrapper.unmount();
  });

  it("does not call the backend when the password field is empty", async () => {
    const store = useMasterPasswordStore();
    store.state = "Locked";
    const wrapper = mount(MasterPasswordLockOverlay);
    await flushPromises();

    await wrapper.get('[data-test="master-password-unlock"]').trigger("click");
    await flushPromises();

    expect(verifyMasterPassword).not.toHaveBeenCalled();
    expect(wrapper.get('[data-test="master-password-unlock-error"]').text()).toContain("请输入主密码");
    wrapper.unmount();
  });
});
