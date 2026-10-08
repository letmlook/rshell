<script setup lang="ts">
import { computed } from "vue";
import { useHostKeyStore, type HostKeyRequest } from "../stores/hostKey";

const store = useHostKeyStore();
const visibleRequests = computed<HostKeyRequest[]>(() => {
  const values = [...store.requests.values()];
  if (store.current && !values.some((request) => request.decision_id === store.current?.decision_id)) {
    values.unshift(store.current);
  }
  return values;
});
const visible = computed(() => visibleRequests.value.length > 0);
const errorFor = (request: HostKeyRequest) =>
  store.decisionErrors.get(request.decision_id) ?? store.error;
</script>

<template>
  <el-dialog
    :model-value="visible"
    :title="visibleRequests.some((request) => request.expected) ? '警告：主机密钥已改变' : '确认新主机密钥'"
    width="520px"
    :show-close="false"
    :close-on-click-modal="false"
    :close-on-press-escape="false"
  >
    <template v-for="request in visibleRequests" :key="request.decision_id">
      <section class="host-key-request" :data-decision-id="request.decision_id">
        <p>
          <strong>{{ request.host }}:{{ request.port }}</strong>
          的 SSH 服务器{{ request.expected ? '密钥与已保存记录不一致' : '密钥尚未保存' }}。
        </p>
        <dl class="key-info">
          <dt>算法</dt>
          <dd>{{ request.key_type }}</dd>
          <dt>收到的指纹 (SHA256)</dt>
          <dd class="mono">{{ request.received }}</dd>
          <template v-if="request.expected">
            <dt>原有指纹 (SHA256)</dt>
            <dd class="mono">{{ request.expected }}</dd>
          </template>
          <dt>公钥 blob</dt>
          <dd class="mono small">{{ request.public_key_blob }}</dd>
        </dl>
        <p class="warning">
          ⚠️ 连接前请确认以上指纹与服务器管理员公布的一致。指纹不一致可能意味着中间人攻击。
        </p>
        <p v-if="errorFor(request)" class="decision-error" role="alert">
          决策提交失败：{{ errorFor(request) }}。请修复后重试，或选择信任一次/取消连接。
        </p>
        <div class="request-actions">
          <el-button @click="store.reject(request.decision_id)">拒绝</el-button>
          <el-button type="primary" plain @click="store.trustOnce(request.decision_id)">信任一次</el-button>
          <el-button type="primary" @click="store.trustPermanent(request.decision_id)">永久信任</el-button>
          <el-button text @click="store.cancel(request.decision_id)">取消连接</el-button>
        </div>
      </section>
    </template>
  </el-dialog>
</template>

<style scoped>
.host-key-request + .host-key-request {
  border-top: 1px solid var(--el-border-color);
  margin-top: 16px;
  padding-top: 16px;
}
.request-actions {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}
.key-info {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: 4px 12px;
  margin: 12px 0;
}
.key-info dt {
  color: var(--el-text-color-secondary);
  font-weight: 500;
}
.key-info dd {
  margin: 0;
  word-break: break-all;
}
.mono {
  font-family: ui-monospace, "Cascadia Code", "Source Code Pro", monospace;
  font-size: 12px;
}
.mono.small {
  font-size: 11px;
  color: var(--el-text-color-secondary);
}
.warning {
  color: var(--el-color-warning);
  font-size: 12px;
  margin: 8px 0 0;
}
.decision-error {
  margin: 12px 0 0;
  padding: 8px 10px;
  border: 1px solid var(--el-color-danger);
  border-radius: 4px;
  color: var(--el-color-danger);
  font-size: 12px;
}
</style>
