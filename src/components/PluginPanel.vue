<script setup lang="ts">
/**
 * PluginPanel —— 切片 9（最后一片）
 *
 * 仅做 IPC 接入（WasmSandbox 仍是 scaffold）：列表 + 启用/禁用 + 状态徽章。
 * 真实插件加载/执行留待后续切片（wasmtime 集成尚未实现）。
 */
import { onMounted, ref } from "vue";
import { listPlugins, scanPlugins, loadPlugin, unloadPlugin, enablePlugin, disablePlugin } from "../ipc/client";

interface PluginInfo {
  id: string;
  name: string;
  version: string;
  state: { Active?: unknown; Disabled?: unknown; Loaded?: unknown; Unloaded?: unknown };
  path: string;
}

const items = ref<PluginInfo[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);

function stateLabel(p: PluginInfo): string {
  const k = Object.keys(p.state || {})[0];
  return k || "—";
}

async function refresh() {
  loading.value = true;
  error.value = null;
  try {
    items.value = (await listPlugins()) as unknown as PluginInfo[];
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function scan() {
  try {
    await scanPlugins();
    await refresh();
  } catch (e) {
    error.value = String(e);
  }
}

async function enable(id: string) {
  try {
    await enablePlugin(id);
    await refresh();
  } catch (e) {
    error.value = String(e);
  }
}

async function disable(id: string) {
  try {
    await disablePlugin(id);
    await refresh();
  } catch (e) {
    error.value = String(e);
  }
}

async function load(id: string) {
  try {
    await loadPlugin(id);
    await refresh();
  } catch (e) {
    error.value = String(e);
  }
}

async function unload(id: string) {
  try {
    await unloadPlugin(id);
    await refresh();
  } catch (e) {
    error.value = String(e);
  }
}

onMounted(refresh);
</script>

<template>
  <section class="plugin-panel">
    <header>
      <h3>插件 ({{ items.length }})</h3>
      <el-button-group size="small">
        <el-button @click="refresh" :loading="loading">刷新</el-button>
        <el-button @click="scan">扫描</el-button>
      </el-button-group>
    </header>

    <p class="hint">
      ℹ️ WasmSandbox 当前为 scaffold；加载/执行调用返回
      <code>IpcError { kind: "internal" }</code>。
      实际 wasmtime 集成待后续切片。
    </p>
    <p v-if="error" class="error">{{ error }}</p>

    <el-empty v-if="items.length === 0" description="暂未发现插件" />
    <el-table v-else :data="items" stripe size="small">
      <el-table-column prop="id" label="Plugin ID" />
      <el-table-column prop="name" label="名称" />
      <el-table-column prop="version" label="版本" width="80" />
      <el-table-column label="状态" width="100">
        <template #default="{ row }">
          <el-tag size="small" :type="stateLabel(row) === 'Active' ? 'success' : 'info'">
            {{ stateLabel(row) }}
          </el-tag>
        </template>
      </el-table-column>
      <el-table-column label="操作" width="220">
        <template #default="{ row }">
          <el-button size="small" @click="load(row.id)">加载</el-button>
          <el-button size="small" @click="unload(row.id)">卸载</el-button>
          <el-button size="small" type="primary" @click="enable(row.id)">启用</el-button>
          <el-button size="small" type="danger" @click="disable(row.id)">禁用</el-button>
        </template>
      </el-table-column>
    </el-table>
  </section>
</template>

<style scoped>
.plugin-panel {
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
.hint {
  color: var(--el-text-color-secondary);
  font-size: 12px;
  margin: 0 0 12px;
  background: var(--el-fill-color-light);
  padding: 8px;
  border-radius: 4px;
}
.error {
  color: var(--el-color-danger);
  font-size: 12px;
}
</style>