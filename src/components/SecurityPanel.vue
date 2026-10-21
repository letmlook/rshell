<script setup lang="ts">
/**
 * SecurityPanel —— 设置 → 安全
 *
 * 把两块安全相关的功能放在一起：
 * - 主密码（应用锁定）
 * - 已信任的主机密钥（查看/删除）
 *
 * 两者都是「谁能用这台机器上的会话」的问题，拆成两个侧栏入口反而割裂。
 */
import MasterPasswordPanel from "./MasterPasswordPanel.vue";
import HostKeyPanel from "./HostKeyPanel.vue";
</script>

<template>
  <div class="security-panel">
    <MasterPasswordPanel embedded />
    <hr class="divider" />
    <HostKeyPanel embedded />
  </div>
</template>

<style scoped>
.security-panel {
  display: flex;
  flex-direction: column;
}
.divider {
  margin: 0 12px;
  border: 0;
  border-top: 1px solid var(--rs-border);
}
</style>
