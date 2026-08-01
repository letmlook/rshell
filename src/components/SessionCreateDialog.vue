<script setup lang="ts">
/**
 * SessionCreateDialog —— 切片 1.3
 *
 * Element Plus 表单收集最小 SSH 会话配置,提交调 invoke('create_session')
 * （设计 §1.3 边界铁律 2:前端只通过 invoke 接触后端）。
 *
 * 字段仅做切片 1 最小可用:host / port / username / password;
 * 切片 3 增 SSH 密钥选择、文件夹、超时等。
 */
import { ref } from "vue";
import { useSessionsStore } from "../stores/sessions";
import type { SessionConfig, Uuid } from "../ipc/types";

const props = defineProps<{ visible: boolean }>();
const emit = defineEmits<{ (e: "close"): void; (e: "created", id: Uuid): void }>();

const store = useSessionsStore();

const form = ref({
  name: "",
  host: "",
  port: 22,
  username: "",
  password: "",
});

const submitting = ref(false);
const error = ref<string | null>(null);

async function submit() {
  submitting.value = true;
  error.value = null;
  try {
    const cfg: SessionConfig = {
      id: crypto.randomUUID() as Uuid,
      name: form.value.name || `${form.value.username}@${form.value.host}`,
      folder_id: null,
      host: form.value.host,
      port: form.value.port,
      protocol: "SSH",
      auth_method: {
        Password: {
          username: form.value.username,
          password: form.value.password,
        },
      },
    };
    const id = await store.create(cfg);
    emit("created", id);
    emit("close");
  } catch (e) {
    error.value = String(e);
  } finally {
    submitting.value = false;
  }
}

function onUpdateVisible(v: boolean) {
  if (!v) emit("close");
}
</script>

<template>
  <el-dialog
    :model-value="props.visible"
    title="新建 SSH 会话"
    width="480px"
    @update:model-value="onUpdateVisible"
    @close="emit('close')"
  >
    <el-form label-width="80px" @submit.prevent="submit">
      <el-form-item label="名称">
        <el-input v-model="form.name" placeholder="可留空,用 host 自动命名" />
      </el-form-item>
      <el-form-item label="主机" required>
        <el-input v-model="form.host" placeholder="host or ip" />
      </el-form-item>
      <el-form-item label="端口">
        <el-input-number v-model="form.port" :min="1" :max="65535" />
      </el-form-item>
      <el-form-item label="用户名" required>
        <el-input v-model="form.username" />
      </el-form-item>
      <el-form-item label="密码">
        <el-input v-model="form.password" type="password" show-password />
      </el-form-item>
      <p v-if="error" style="color: var(--el-color-danger)">{{ error }}</p>
    </el-form>
    <template #footer>
      <el-button @click="emit('close')">取消</el-button>
      <el-button type="primary" :loading="submitting" @click="submit">创建</el-button>
    </template>
  </el-dialog>
</template>