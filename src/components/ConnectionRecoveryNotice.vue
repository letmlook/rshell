<script setup lang="ts">
import type { CoreErrorKind, Uuid } from "../ipc/types";

const props = defineProps<{
  sessionId: Uuid;
  kind: CoreErrorKind;
  message: string;
}>();

const emit = defineEmits<{
  (event: "update-credential", sessionId: Uuid): void;
  (event: "retry-connect", sessionId: Uuid): void;
  (event: "retry-load", sessionId: Uuid): void;
}>();

const copy: Record<CoreErrorKind, { text: string; action: string; event: "update-credential" | "retry-connect" | "retry-load" }> = {
  credential_missing: { text: "凭据条目缺失", action: "更新凭据", event: "update-credential" },
  credential_inaccessible: {
    text: "凭据存储不可访问：请修复钥匙串访问后重试",
    action: "重试连接",
    event: "retry-connect",
  },
  credential_save_failed: { text: "凭据保存失败", action: "更新凭据", event: "update-credential" },
  credential_migration_failed: { text: "旧配置迁移失败", action: "重试加载", event: "retry-load" },
  auth_failed: { text: "认证被拒绝", action: "重新编辑凭据", event: "update-credential" },
  connection: { text: "网络连接失败", action: "重试连接", event: "retry-connect" },
  host_key_mismatch: { text: "主机密钥未确认", action: "重新连接", event: "retry-connect" },
  host_key_trust_persistence_failed: {
    text: "永久信任保存失败",
    action: "重试连接",
    event: "retry-connect",
  },
};

function act() {
  const action = copy[props.kind].event;
  if (action === "update-credential") emit("update-credential", props.sessionId);
  else if (action === "retry-connect") emit("retry-connect", props.sessionId);
  else emit("retry-load", props.sessionId);
}
</script>

<template>
  <section class="connection-recovery" role="alert" :data-error-kind="kind">
    <strong>{{ copy[kind].text }}</strong>
    <span>{{ message }}</span>
    <button type="button" :data-test="`recovery-${copy[kind].event}`" @click="act">
      {{ copy[kind].action }}
    </button>
  </section>
</template>

<style scoped>
.connection-recovery {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--rs-s-2);
  margin: var(--rs-s-1) 0;
  padding: var(--rs-s-2);
  color: var(--rs-p-danger);
  border: 1px solid var(--rs-p-danger);
  border-radius: var(--rs-radius-1);
  font-size: var(--rs-fs-xs);
}
.connection-recovery span {
  color: var(--rs-fg-muted);
}
.connection-recovery button {
  cursor: pointer;
}
</style>
