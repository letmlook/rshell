<script setup lang="ts">
/**
 * MasterPasswordDialog —— 切片 6.3
 *
 * 启动时引导设置主密码。`MasterPasswordRequired` 事件（设计 §3.2 后端触发）
 * 让本组件弹出;用户输入两次新密码后调 `setup_master_password`。
 */
import { computed, ref } from "vue";
import { useSessionsStore } from "../stores/sessions";
import { setupMasterPassword } from "../ipc/client";

const sessionsStore = useSessionsStore();

const visible = computed(() => sessionsStore.masterPasswordRequired);
const password = ref("");
const confirm = ref("");
const submitting = ref(false);
const error = ref<string | null>(null);

async function submit() {
  if (password.value !== confirm.value) {
    error.value = "两次输入不一致";
    return;
  }
  submitting.value = true;
  error.value = null;
  try {
    await setupMasterPassword(password.value);
    password.value = "";
    confirm.value = "";
    sessionsStore.masterPasswordRequired = false;
  } catch (e) {
    error.value = String(e);
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <el-dialog
    :model-value="visible"
    title="设置主密码"
    width="420px"
    :show-close="false"
    :close-on-click-modal="false"
    :close-on-press-escape="false"
  >
    <p>主密码用于加密本地 SSH 私钥。请妥善保存,丢失后将无法恢复已存储的密钥。</p>
    <el-form label-width="100px" @submit.prevent="submit">
      <el-form-item label="主密码">
        <el-input v-model="password" type="password" show-password />
      </el-form-item>
      <el-form-item label="再次输入">
        <el-input v-model="confirm" type="password" show-password />
      </el-form-item>
      <p v-if="error" class="error">{{ error }}</p>
    </el-form>
    <template #footer>
      <el-button type="primary" :loading="submitting" @click="submit">设置</el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
.error {
  color: var(--el-color-danger);
  font-size: 12px;
  margin: 0;
}
</style>