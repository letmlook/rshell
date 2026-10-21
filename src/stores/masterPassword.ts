/**
 * masterPassword pinia store —— 应用锁定
 *
 * 主密码的语义边界（见 docs/08）：**只用于应用锁定与验证**，不是凭据保险库，
 * 也不加密 macOS 钥匙串里的会话凭据。
 *
 * 后端持久化的只有盐与加密后的验证令牌；派生密钥永不落盘，因此重启后状态是
 * `Locked`，必须重新验证才能解锁。本 store 持有这份状态并在锁定时驱动遮罩。
 */
import { defineStore } from "pinia";
import { computed, ref } from "vue";
import {
  changeMasterPassword,
  disableMasterPassword,
  getMasterPasswordState,
  lockMasterPassword,
  setupMasterPassword,
  verifyMasterPassword,
  type MasterPasswordState,
} from "../ipc/client";
import { subscribeAppEvents } from "../ipc/events";
import { ipcErrorMessage } from "../ipc/client";

export const useMasterPasswordStore = defineStore("masterPassword", () => {
  const state = ref<MasterPasswordState>("NotSet");
  const loading = ref(false);
  const error = ref<string | null>(null);

  let unlisten: (() => void) | null = null;
  let subscriptionGeneration = 0;

  /** 已设置主密码（无论是否解锁） */
  const isSet = computed(() => state.value !== "NotSet");
  /** 处于锁定态：需要用户输入主密码才能继续使用应用 */
  const isLocked = computed(() => state.value === "Locked");

  async function refresh() {
    loading.value = true;
    try {
      state.value = await getMasterPasswordState();
      error.value = null;
    } catch (e) {
      error.value = ipcErrorMessage(e);
    } finally {
      loading.value = false;
    }
  }

  /** 设置主密码（仅首次）。后端写盘失败会报错，不会假装成功 */
  async function setup(password: string) {
    loading.value = true;
    try {
      await setupMasterPassword(password);
      state.value = "Unlocked";
      error.value = null;
      return true;
    } catch (e) {
      error.value = ipcErrorMessage(e);
      return false;
    } finally {
      loading.value = false;
    }
  }

  /** 验证主密码。返回是否通过（失败不抛错，由调用方展示） */
  async function verify(password: string) {
    loading.value = true;
    try {
      const ok = await verifyMasterPassword(password);
      state.value = ok ? "Unlocked" : "Locked";
      error.value = ok ? null : "主密码不正确";
      return ok;
    } catch (e) {
      error.value = ipcErrorMessage(e);
      return false;
    } finally {
      loading.value = false;
    }
  }

  async function change(oldPassword: string, newPassword: string) {
    loading.value = true;
    try {
      await changeMasterPassword(oldPassword, newPassword);
      state.value = "Unlocked";
      error.value = null;
      return true;
    } catch (e) {
      error.value = ipcErrorMessage(e);
      return false;
    } finally {
      loading.value = false;
    }
  }

  /** 停用主密码：必须提供当前密码 */
  async function disable(password: string) {
    loading.value = true;
    try {
      await disableMasterPassword(password);
      state.value = "NotSet";
      error.value = null;
      return true;
    } catch (e) {
      error.value = ipcErrorMessage(e);
      return false;
    } finally {
      loading.value = false;
    }
  }

  /** 立即锁定：清除内存中的派生密钥 */
  async function lock() {
    loading.value = true;
    try {
      await lockMasterPassword();
      state.value = "Locked";
      error.value = null;
      return true;
    } catch (e) {
      error.value = ipcErrorMessage(e);
      return false;
    } finally {
      loading.value = false;
    }
  }

  async function subscribeEvents() {
    if (unlisten) return;
    const generation = ++subscriptionGeneration;
    const stop = await subscribeAppEvents((event) => {
      if (typeof event === "string") return;
      if ("MasterPasswordChanged" in event) {
        // is_set=false 表示主密码已被停用；is_set=true 既可能是设置也可能是锁定，
        // 两者都要重新查一次真实状态，不能凭事件猜。
        void refresh();
      } else if ("MasterPasswordVerified" in event) {
        state.value = event.MasterPasswordVerified.success ? "Unlocked" : "Locked";
      }
    });
    if (generation === subscriptionGeneration) unlisten = stop;
    else stop();
  }

  function disposeEvents() {
    subscriptionGeneration++;
    unlisten?.();
    unlisten = null;
  }

  return {
    state,
    loading,
    error,
    isSet,
    isLocked,
    refresh,
    setup,
    verify,
    change,
    disable,
    lock,
    subscribeEvents,
    disposeEvents,
  };
});
