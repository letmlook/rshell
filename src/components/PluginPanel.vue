<script setup lang="ts">
import { onMounted, ref } from "vue";
import { listPlugins, scanPlugins, loadPlugin, unloadPlugin } from "../ipc/client";
import type { PluginInfo } from "../ipc/types";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const items = ref<PluginInfo[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);

function stateLabel(p: PluginInfo): string {
  return ({ Discovered: "已发现", Loaded: "已加载", Active: "已启用", Disabled: "已禁用", Error: "错误" })[p.state];
}

async function refresh() {
  loading.value = true;
  error.value = null;
  try {
    items.value = await listPlugins();
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

onMounted(scan);
</script>

<template>
  <section class="plugin-panel">
    <header v-if="!props.embedded">
      <h3>插件 ({{ items.length }})</h3>
      <el-button-group size="small">
        <el-button @click="refresh" :loading="loading">刷新</el-button>
        <el-button @click="scan">扫描</el-button>
      </el-button-group>
    </header>

    <p class="hint">仅加载受信任的本地 WASM 插件。插件目录位于应用数据目录的 plugins 下。</p>
    <p v-if="error" class="error">{{ error }}</p>

    <el-empty v-if="items.length === 0" description="暂未发现插件" />
    <el-table v-else :data="items" stripe size="small">
      <el-table-column prop="id" label="Plugin ID" />
      <el-table-column prop="name" label="名称" />
      <el-table-column prop="version" label="版本" width="80" />
      <el-table-column label="状态" width="100">
        <template #default="{ row }">
          <el-tag size="small" :type="row.state === 'Active' ? 'success' : 'info'">
            {{ stateLabel(row) }}
          </el-tag>
        </template>
      </el-table-column>
      <el-table-column label="操作" width="220">
        <template #default="{ row }">
          <el-button v-if="row.state === 'Discovered' || row.state === 'Disabled'" size="small" @click="load(row.id)">加载</el-button>
          <el-button v-if="row.state === 'Loaded' || row.state === 'Active'" size="small" @click="unload(row.id)">卸载</el-button>
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
