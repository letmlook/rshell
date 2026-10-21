<script setup lang="ts">
/**
 * MasterPasswordLockOverlay —— 应用锁定遮罩
 *
 * 锁定态下盖住整个工作区：用户必须输入主密码才能继续使用应用。
 * 遮罩不隐藏终端的既有连接状态——它只是挡住交互入口，用户仍能从下方
 * 状态栏看到当前连接；主密码的作用是防止他人直接操作这台机器上的会话。
 */
import { nextTick, ref, watch } from "vue";
import { useMasterPasswordStore } from "../stores/masterPassword";

const store = useMasterPasswordStore();

const password = ref("");
const error = ref<string | null>(null);
const inputEl = ref<HTMLInputElement | null>(null);

async function unlock() {
  if (!password.value) {
    error.value = "请输入主密码";
    return;
  }
  error.value = null;
  const ok = await store.verify(password.value);
  if (ok) {
    password.value = "";
    error.value = null;
  } else {
    error.value = store.error ?? "主密码不正确";
  }
}

// 遮罩出现时把焦点交给密码框：否则用户第一下按键会落在被遮住的界面上
watch(
  () => store.isLocked,
  async (locked) => {
    if (!locked) {
      password.value = "";
      error.value = null;
      return;
    }
    await nextTick();
    inputEl.value?.focus();
  },
  { immediate: true },
);
</script>

<template>
  <div v-if="store.isLocked" class="lock-overlay" data-test="master-password-lock-overlay">
    <div class="lock-card">
      <h2>RShell 已锁定</h2>
      <p class="hint">输入主密码以继续使用本应用。</p>
      <input
        ref="inputEl"
        v-model="password"
        type="password"
        autocomplete="current-password"
        placeholder="主密码"
        data-test="master-password-unlock-input"
        @keyup.enter="unlock"
      />
      <p v-if="error" class="error" data-test="master-password-unlock-error">{{ error }}</p>
      <button
        type="button"
        class="unlock-btn"
        :disabled="store.loading"
        data-test="master-password-unlock"
        @click="unlock"
      >
        {{ store.loading ? "验证中…" : "解锁" }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.lock-overlay {
  position: fixed;
  inset: 0;
  z-index: 3000;
  display: flex;
  align-items: center;
  justify-content: center;
  background: color-mix(in srgb, var(--rs-bg, #101014) 88%, transparent);
  backdrop-filter: blur(6px);
}
.lock-card {
  width: min(360px, calc(100vw - 48px));
  padding: 24px;
  border-radius: 10px;
  background: var(--rs-bg-surface, #1b1b20);
  border: 1px solid var(--rs-border, #2c2c33);
  box-shadow: 0 18px 48px rgba(0, 0, 0, 0.45);
  text-align: center;
}
.lock-card h2 {
  margin: 0 0 6px;
  font-size: 16px;
  color: var(--rs-fg, #e8e8ea);
}
.hint {
  margin: 0 0 16px;
  font-size: 12px;
  color: var(--rs-fg-muted, #9a9aa2);
}
.lock-card input {
  width: 100%;
  box-sizing: border-box;
  padding: 8px 10px;
  font-size: 13px;
  color: var(--rs-fg, #e8e8ea);
  background: var(--rs-bg, #101014);
  border: 1px solid var(--rs-border, #2c2c33);
  border-radius: 6px;
  outline: none;
}
.lock-card input:focus {
  border-color: var(--rs-accent, #4c8dff);
}
.unlock-btn {
  width: 100%;
  margin-top: 12px;
  padding: 8px 12px;
  font-size: 13px;
  color: #fff;
  background: var(--rs-accent, #4c8dff);
  border: 0;
  border-radius: 6px;
  cursor: pointer;
}
.unlock-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.error {
  margin: 8px 0 0;
  font-size: 12px;
  color: var(--el-color-danger, #f56c6c);
}
</style>
