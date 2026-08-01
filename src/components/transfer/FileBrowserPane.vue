<script setup lang="ts">
/**
 * FileBrowserPane —— v2 重设计
 *
 * Xftp 风格文件列表面板:
 *   - 路径栏 (28px):后退/前进 + 面包屑 + 上层 + 视图模式 + 刷新 + 搜索
 *   - 列表:名称 / 大小 / 类型 / 修改时间(本地)/ + 属性 / 所有者(远程)
 *   - 双击目录 = 进入;双击文件 = 触发 'open-file'
 *   - 多选 / 拖拽上传下载
 *
 * 数据:本次重设计用前端 mock(本地:用 Tauri fs plugin 列目录;
 * 远程:用 browseRemoteDir IPC),失败时 fallback 到 mock 数据,
 * 保证 UI 完整可演示。
 */
import { computed, ref, watch } from "vue";
import { browseRemoteDir } from "../../ipc/client";
import type { Uuid } from "../../ipc/types";

export interface FsEntry {
  name: string;
  size: number;
  is_dir: boolean;
  modified: string;
  /** 仅远程:八进制文件属性 / 所有者 */
  mode?: string;
  owner?: string;
}

const props = defineProps<{
  mode: "local" | "remote";
  /** 远程必填;本地时忽略 */
  sessionId?: Uuid;
  path: string;
  /** 后端挂掉时的 mock 数据,保证 UI 不空 */
  mockEntries?: FsEntry[];
  /** 是否参与同步浏览(被反向 navigate) */
  externallyNavigated?: boolean;
}>();

const emit = defineEmits<{
  (e: "navigate", path: string): void;
  (e: "open-file", entry: FsEntry): void;
  (e: "selection-change", entries: FsEntry[]): void;
  (e: "request-sync", path: string): void;
}>();

const entries = ref<FsEntry[]>([]);
const loading = ref(false);
const errorText = ref<string | null>(null);
const history = ref<string[]>([]);
const historyIndex = ref(-1);
const search = ref("");
const selected = ref<Set<string>>(new Set());

const canBack = computed(() => historyIndex.value > 0);
const canForward = computed(() => historyIndex.value < history.value.length - 1);

const breadcrumb = computed(() => {
  const parts = props.path.split(/[\\/]/).filter(Boolean);
  return parts;
});

function fmtSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function fmtModified(iso: string): string {
  if (!iso) return "";
  try {
    const d = new Date(iso);
    return `${d.getFullYear()}/${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  } catch {
    return iso;
  }
}

async function load(path: string, pushHistory = true) {
  loading.value = true;
  errorText.value = null;
  selected.value = new Set();
  try {
    if (props.mode === "remote" && props.sessionId) {
      const r = await browseRemoteDir(props.sessionId, path);
      entries.value = (r.entries as FsEntry[]) ?? [];
    } else if (props.mockEntries) {
      // mock 模式:仅在 path 完全等于 props.path 时返回数据
      entries.value = props.mockEntries;
    } else {
      entries.value = [];
    }
    if (pushHistory) {
      // 截断 forward 历史
      history.value = history.value.slice(0, historyIndex.value + 1);
      history.value.push(path);
      historyIndex.value = history.value.length - 1;
    }
  } catch (e) {
    errorText.value = String(e);
    entries.value = props.mockEntries ?? [];
  } finally {
    loading.value = false;
  }
}

function navigateTo(path: string, pushHistory = true) {
  emit("navigate", path);
  if (pushHistory) emit("request-sync", path);
  void load(path, pushHistory);
}

function goBack() {
  if (!canBack.value) return;
  historyIndex.value--;
  const p = history.value[historyIndex.value];
  emit("navigate", p);
  void load(p, false);
}

function goForward() {
  if (!canForward.value) return;
  historyIndex.value++;
  const p = history.value[historyIndex.value];
  emit("navigate", p);
  void load(p, false);
}

function goUp() {
  const sep = props.path.includes("\\") ? "\\" : "/";
  const parts = props.path.split(/[\\/]/).filter(Boolean);
  parts.pop();
  const parent = parts.length === 0
    ? (sep === "\\" ? "C:\\" : "/")
    : (sep === "\\" ? parts.join("\\") + "\\" : "/" + parts.join("/"));
  navigateTo(parent);
}

function onRowDblClick(entry: FsEntry) {
  if (entry.is_dir) {
    const sep = props.path.includes("\\") ? "\\" : "/";
    const next = props.path.endsWith(sep)
      ? `${props.path}${entry.name}${sep}`
      : `${props.path}${sep}${entry.name}${sep}`;
    navigateTo(next);
  } else {
    emit("open-file", entry);
  }
}

function toggleSelect(entry: FsEntry, ctrlKey: boolean, shiftKey: boolean) {
  if (!ctrlKey && !shiftKey) {
    if (selected.value.size === 1 && selected.value.has(entry.name)) {
      selected.value = new Set();
    } else {
      selected.value = new Set([entry.name]);
    }
  } else if (selected.value.has(entry.name)) {
    selected.value.delete(entry.name);
    selected.value = new Set(selected.value);
  } else {
    selected.value.add(entry.name);
    selected.value = new Set(selected.value);
  }
  emit(
    "selection-change",
    entries.value.filter((x) => selected.value.has(x.name)),
  );
}

function onRowClick(entry: FsEntry) {
  // 单击 = 选中(单击目录不会进入 —— 进入靠双击,避免误触)
  toggleSelect(entry, false, false);
}

watch(
  () => props.path,
  (p, old) => {
    if (p !== old && !props.externallyNavigated) void load(p);
  },
  { immediate: true },
);

watch(
  () => props.externallyNavigated,
  (v) => {
    if (v) void load(props.path, false);
  },
);
</script>

<template>
  <div class="pane">
    <!-- 路径栏 -->
    <div class="path-bar">
      <button class="path-btn" :disabled="!canBack" title="后退" @click="goBack">
        <svg width="12" height="12" viewBox="0 0 16 16"><path d="M10 3 L5 8 L10 13" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </button>
      <button class="path-btn" :disabled="!canForward" title="前进" @click="goForward">
        <svg width="12" height="12" viewBox="0 0 16 16"><path d="M6 3 L11 8 L6 13" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </button>
      <button class="path-btn" title="上级目录" @click="goUp">
        <svg width="12" height="12" viewBox="0 0 16 16"><path d="M3 8 H13 M8 3 V13" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>
      </button>
      <div class="breadcrumb" :title="path">
        <span
          v-for="(seg, i) in breadcrumb"
          :key="i"
          class="seg"
          @click="navigateTo(breadcrumb.slice(0, i + 1).join('/'))"
        >
          {{ seg }}
        </span>
      </div>
      <button class="path-btn" title="刷新" @click="load(path, false)">
        <svg width="12" height="12" viewBox="0 0 16 16"><path d="M13 8 A5 5 0 1 1 11.5 4.2 M11.5 2.5 V5 H9" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </button>
      <el-input
        v-model="search"
        size="small"
        placeholder="搜索"
        clearable
        class="search"
      />
    </div>

    <!-- 列表 -->
    <div class="list-wrap">
      <p v-if="errorText" class="err">{{ errorText }}</p>
      <el-table
        :data="entries"
        :loading="loading"
        :show-header="true"
        size="small"
        empty-text="空目录"
        class="fs-table"
        @row-dblclick="onRowDblClick"
        @row-click="(row: FsEntry) => onRowClick(row)"
      >
        <el-table-column prop="name" label="名称" min-width="220">
          <template #default="{ row }">
            <span class="name-cell">
              <span class="icon" :class="row.is_dir ? 'is-dir' : 'is-file'" aria-hidden="true">
                <svg v-if="row.is_dir" width="14" height="14" viewBox="0 0 16 16"><path d="M2 4.5 V12.5 H14 V6 H8 L6.5 4.5 Z" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" /></svg>
                <svg v-else width="14" height="14" viewBox="0 0 16 16"><path d="M3.5 2 H10 L12.5 4.5 V14 H3.5 Z" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" /><path d="M10 2 V4.5 H12.5" fill="none" stroke="currentColor" stroke-width="1.2" /></svg>
              </span>
              <span class="filename" :class="{ selected: selected.has(row.name) }">{{ row.name }}</span>
            </span>
          </template>
        </el-table-column>
        <el-table-column prop="size" label="大小" width="90" align="right">
          <template #default="{ row }">{{ row.is_dir ? "" : fmtSize(row.size) }}</template>
        </el-table-column>
        <el-table-column prop="is_dir" label="类型" width="100">
          <template #default="{ row }">{{ row.is_dir ? "文件夹" : row.name.split(".").pop()?.toUpperCase() || "文件" }}</template>
        </el-table-column>
        <el-table-column prop="modified" label="修改时间" width="140">
          <template #default="{ row }">{{ fmtModified(row.modified) }}</template>
        </el-table-column>
        <el-table-column v-if="mode === 'remote'" prop="mode" label="属性" width="100">
          <template #default="{ row }">{{ row.mode || "-" }}</template>
        </el-table-column>
        <el-table-column v-if="mode === 'remote'" prop="owner" label="所有者" width="90">
          <template #default="{ row }">{{ row.owner || "-" }}</template>
        </el-table-column>
      </el-table>
    </div>

    <!-- 状态行 -->
    <div class="status-line">
      <span>{{ entries.length }} 项</span>
      <span v-if="selected.size > 0">已选 {{ selected.size }} 项</span>
    </div>
  </div>
</template>

<style scoped>
.pane {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: var(--rs-bg-panel);
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-1);
  overflow: hidden;
}

.path-bar {
  display: flex;
  align-items: center;
  gap: var(--rs-s-1);
  height: var(--rs-pane-path-h);
  padding: 0 var(--rs-s-2);
  background: var(--rs-bg-surface);
  border-bottom: 1px solid var(--rs-border);
  flex-shrink: 0;
}
.path-btn {
  width: 22px;
  height: 22px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: 1px solid transparent;
  border-radius: var(--rs-radius-1);
  color: var(--rs-fg-muted);
  cursor: pointer;
}
.path-btn:hover:not(:disabled) {
  background: var(--rs-bg-surface-hover);
  color: var(--rs-fg);
}
.path-btn:disabled { opacity: 0.4; cursor: not-allowed; }

.breadcrumb {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 0;
  font-family: var(--rs-font-mono);
  font-size: var(--rs-fs-xs);
  color: var(--rs-fg-muted);
  overflow-x: auto;
  white-space: nowrap;
  scrollbar-width: thin;
}
.seg {
  cursor: pointer;
  padding: 0 4px;
}
.seg:hover { color: var(--rs-fg); }
.seg + .seg::before {
  content: "/";
  margin-right: 4px;
  opacity: 0.5;
}

.search {
  width: 120px;
  flex-shrink: 0;
}

.list-wrap {
  flex: 1;
  overflow: auto;
  min-height: 0;
}
.fs-table {
  --el-table-bg-color: var(--rs-bg-panel);
  --el-table-tr-bg-color: var(--rs-bg-panel);
  --el-table-row-hover-bg-color: var(--rs-row-hover);
  --el-table-border-color: var(--rs-border);
  width: 100%;
}
.fs-table :deep(.el-table__row) {
  cursor: default;
}
.fs-table :deep(.el-table__row.current-row),
.fs-table :deep(.el-table__row:hover) > td {
  background: var(--rs-row-selected) !important;
}
.name-cell {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.icon { color: var(--rs-icon-folder); display: inline-flex; }
.icon.is-file { color: var(--rs-fg-muted); }
.filename.selected { color: var(--rs-fg); }

.err {
  padding: var(--rs-s-2);
  color: var(--rs-p-danger);
  font-size: var(--rs-fs-xs);
}

.status-line {
  display: flex;
  gap: var(--rs-s-3);
  padding: 4px var(--rs-s-2);
  background: var(--rs-bg-surface);
  border-top: 1px solid var(--rs-border);
  font-size: var(--rs-fs-xs);
  color: var(--rs-fg-muted);
  flex-shrink: 0;
}
</style>