<script setup lang="ts">
/**
 * TunnelPanel —— 切片 8
 *
 * 隧道管理三栏:本地(Local Forward)/远端(Remote Forward)/动态(Dynamic SOCKS)。
 * 切片 8 受限:`direct-tcpip` 未实现(docs/08 #3),仅做 IPC 接入与 UI。
 * 真实转发留待后续 rshell-protocol 接入 russh-direct-tcpip channel。
 */
import { onMounted, ref } from "vue";
import { listTunnels, createTunnel, closeTunnel } from "../ipc/client";
import type { Uuid, PortForwardRule, ActiveTunnelInfo } from "../ipc/types";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const items = ref<ActiveTunnelInfo[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);

const draftType = ref<"Local" | "Remote" | "Dynamic">("Local");
const draftSession = ref<Uuid | null>(null);
const draftBind = ref("127.0.0.1:8080");
const draftTarget = ref("localhost:80");

async function refresh() {
  loading.value = true;
  try {
    items.value = (await listTunnels()) as unknown as ActiveTunnelInfo[];
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

function parseEndpoint(ep: string): { host: string; port: number } {
  const idx = ep.lastIndexOf(":");
  if (idx < 0) return { host: ep, port: 0 };
  return { host: ep.slice(0, idx), port: parseInt(ep.slice(idx + 1), 10) || 0 };
}

async function add() {
  if (!draftSession.value) {
    error.value = "请先在主视图选择会话";
    return;
  }
  const bind = parseEndpoint(draftBind.value);
  const target = parseEndpoint(draftTarget.value);
  // 设计 §4.2 的 PortForwardRule 是单一 struct,通过 direction 字段区分。
  // direct-tcpip 转发实现是后续切片(本次仅 IPC 接入)。
  const rule: PortForwardRule = {
    bind_address: bind.host,
    bind_port: bind.port,
    remote_host: target.host,
    remote_port: target.port,
    direction: draftType.value,
  };
  try {
    await createTunnel(draftSession.value, rule);
    await refresh();
  } catch (e) {
    error.value = String(e);
  }
}

async function remove(id: Uuid) {
  try {
    await closeTunnel(id);
    await refresh();
  } catch (e) {
    error.value = String(e);
  }
}

onMounted(refresh);
</script>

<template>
  <section class="tunnel-panel">
    <header v-if="!props.embedded">
      <h3>端口转发 ({{ items.length }})</h3>
      <el-button size="small" :loading="loading" @click="refresh">刷新</el-button>
    </header>
    <p v-if="error" class="error">{{ error }}</p>

    <el-form inline size="small" class="add-form" @submit.prevent="add">
      <el-form-item label="类型">
        <el-select v-model="draftType" style="width: 110px">
          <el-option label="Local" value="Local" />
          <el-option label="Remote" value="Remote" />
          <el-option label="Dynamic" value="Dynamic" />
        </el-select>
      </el-form-item>
      <el-form-item label="会话 ID">
        <el-input v-model="draftSession" placeholder="uuid" style="width: 180px" />
      </el-form-item>
      <el-form-item label="监听">
        <el-input v-model="draftBind" placeholder="host:port" style="width: 140px" />
      </el-form-item>
      <el-form-item v-if="draftType !== 'Dynamic'" label="目标">
        <el-input v-model="draftTarget" placeholder="host:port" style="width: 140px" />
      </el-form-item>
      <el-form-item>
        <el-button type="primary" native-type="submit">新建</el-button>
      </el-form-item>
    </el-form>

    <el-empty v-if="items.length === 0" description="暂无隧道" />
    <el-table v-else :data="items" stripe size="small">
      <el-table-column prop="id" label="Tunnel ID" width="120" />
      <el-table-column label="状态" width="100">
        <template #default="{ row }">
          {{ String(Object.keys(row.state || {})[0] || "—") }}
        </template>
      </el-table-column>
      <el-table-column label="操作" width="80">
        <template #default="{ row }">
          <el-button size="small" type="danger" @click="remove(row.id)">关闭</el-button>
        </template>
      </el-table-column>
    </el-table>
  </section>
</template>

<style scoped>
.tunnel-panel {
  padding: 12px;
}
header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
}
h3 {
  margin: 0;
  font-size: 14px;
}
.add-form {
  margin-bottom: 12px;
}
.error {
  color: var(--el-color-danger);
  font-size: 12px;
}
</style>