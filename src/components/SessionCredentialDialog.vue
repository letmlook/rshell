<script setup lang="ts">
import { computed, ref } from "vue";
import type { SessionConfig } from "../ipc/types";
import { useSessionsStore } from "../stores/sessions";

const props = defineProps<{ session: SessionConfig }>();
const emit = defineEmits<{ (e: "close"): void }>();
const store = useSessionsStore();
const secret = ref("");
const submitting = ref(false);
const error = ref<string | null>(null);
const label = computed(() => "PublicKey" in props.session.auth_method ? "新私钥口令" : "新密码");

function close() {
  secret.value = "";
  emit("close");
}
async function save() {
  if (!secret.value || submitting.value) return;
  submitting.value = true;
  error.value = null;
  try {
    await store.updateCredential(props.session.id, { secret: secret.value });
    close();
  } catch (e) {
    error.value = String(e);
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <el-dialog :model-value="true" title="更新凭据" width="440px" @close="close">
    <p>{{ session.name }} · {{ session.host }}</p>
    <p>重新输入凭据并保存到系统钥匙串，然后重新连接。</p>
    <el-form label-width="100px" @submit.prevent="save">
      <el-form-item :label="label">
        <el-input v-model="secret" type="password" show-password autocomplete="new-password" />
      </el-form-item>
      <p v-if="error" role="alert">{{ error }}</p>
    </el-form>
    <template #footer>
      <el-button @click="close">取消</el-button>
      <el-button type="primary" :loading="submitting" :disabled="!secret || submitting" @click="save">保存凭据</el-button>
    </template>
  </el-dialog>
</template>
