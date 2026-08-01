<script setup lang="ts">
/**
 * HostKeyMismatchDialog —— 切片 4
 *
 * 设计 §4.3 流程 B：握手期间弹窗显示指纹对比 + 三按钮（信任一次/永久/拒绝）。
 * 用户决策后通过 `decide_host_key` 唤醒阻塞的 SshHandler oneshot。
 */
import { computed } from "vue";
import { useHostKeyStore } from "../stores/hostKey";

const store = useHostKeyStore();
const visible = computed(() => store.current !== null);
</script>

<template>
  <el-dialog
    :model-value="visible"
    title="主机密钥不匹配"
    width="520px"
    :show-close="false"
    :close-on-click-modal="false"
    :close-on-press-escape="false"
  >
    <template v-if="store.current">
      <p>
        <strong>{{ store.current.host }}:{{ store.current.port }}</strong>
        的 SSH 服务器密钥不在已知主机列表中。
      </p>
      <dl class="key-info">
        <dt>算法</dt>
        <dd>{{ store.current.key_type }}</dd>
        <dt>收到的指纹 (SHA256)</dt>
        <dd class="mono">{{ store.current.received }}</dd>
        <dt>公钥 blob</dt>
        <dd class="mono small">{{ store.current.public_key_blob }}</dd>
      </dl>
      <p class="warning">
        ⚠️ 连接前请确认以上指纹与服务器管理员公布的一致。指纹不一致可能意味着中间人攻击。
      </p>
    </template>
    <template #footer>
      <el-button @click="store.reject()">拒绝</el-button>
      <el-button type="primary" plain @click="store.trustOnce()">信任一次</el-button>
      <el-button type="primary" @click="store.trustPermanent()">永久信任</el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
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
</style>