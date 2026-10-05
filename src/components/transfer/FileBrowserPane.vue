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
 * 本地目录由已授权的 Tauri fs 读取;远程目录由 SFTP IPC 读取。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { readDir, stat } from "@tauri-apps/plugin-fs";
import { browseRemoteDir } from "../../ipc/client";
import { isWithinRoot } from "../../utils/rootBoundary";
import PathBar from "./PathBar.vue";
import type { Uuid, FilePermissions } from "../../ipc/types";

export interface FsEntry {
  name: string;
  size: number;
  is_dir: boolean;
  modified: string;
  /** 仅远程:rwx 权限字符串(如 rwxr-xr-x),由 FilePermissions 拼出 */
  mode?: string;
  owner?: string;
}

const props = defineProps<{
  mode: "local" | "remote";
  /** 远程必填;本地时忽略 */
  sessionId?: Uuid;
  path: string;
  rootPath?: string;
  /** 本地模式：已授权的根目录，用于在路径栏上显示「更换」入口 */
  rootLabel?: string;
  /** 是否参与同步浏览(被反向 navigate) */
  externallyNavigated?: boolean;
}>();

const emit = defineEmits<{
  (e: "navigate", path: string): void;
  (e: "open-file", entry: FsEntry): void;
  (e: "selection-change", entries: FsEntry[]): void;
  (e: "request-sync", path: string): void;
  /** 请求打开本地目录选择器（仅本地模式） */
  (e: "choose-root"): void;
  /**
   * 右键菜单请求。`entries` 为右键时的操作目标：点在已选行上时是整个当前选中集，
   * 点在未选行上时是那单独一行；点在列表空白处时为空数组（只给面板级操作）。
   */
  (e: "row-context-menu", payload: { entries: FsEntry[]; x: number; y: number }): void;
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
const visibleEntries = computed(() => entries.value.filter((entry) => entry.name.toLowerCase().includes(search.value.toLowerCase())));

/**
 * 列表容器的实测高度，喂给 el-table 的 height。
 *
 * 之前只有外层 `.list-wrap { overflow: auto }`，表头随内容一起滚走，且
 * 纵向滚动条挂在面板外层——双窗格里几乎分不清是列表还是面板在滚。
 * 交给 el-table 自己管高度后：表头固定、纵向滚动条贴着表格，
 * 横向滚动条也出现在表格底部（列宽总和大于面板宽度时才出现）。
 *
 * 未测到高度（jsdom / 隐藏容器）时传 undefined，el-table 退化为自适应高度，
 * 不会因为 0 高度把整个面板压没。
 */
const listWrapRef = ref<HTMLElement | null>(null);
const tableHeight = ref<number | undefined>(undefined);
let listObserver: ResizeObserver | null = null;

function measureList() {
  const el = listWrapRef.value;
  if (!el) return;
  const h = el.clientHeight;
  tableHeight.value = h > 0 ? h : undefined;
}

onMounted(() => {
  measureList();
  // ResizeObserver 在 jsdom 里不存在，缺省时保持自适应高度即可
  if (typeof ResizeObserver === "undefined" || !listWrapRef.value) return;
  listObserver = new ResizeObserver(() => measureList());
  listObserver.observe(listWrapRef.value);
});

onBeforeUnmount(() => {
  listObserver?.disconnect();
  listObserver = null;
});

/** 根目录是否已授权（本地模式）。未授权时路径栏显示「选择文件夹」入口 */
const hasRoot = computed(() => props.mode !== "local" || !!props.rootPath);

const parentPath = computed<string | null>(() => {
  if (props.mode === "local" && props.rootPath && props.path === props.rootPath) return null;
  // 本地可能是 Windows 路径（D:\data），远端是 POSIX（/home/x）——两种分隔符都要认
  const idx = Math.max(props.path.lastIndexOf("/"), props.path.lastIndexOf("\\"));
  if (idx < 0) return null;
  if (idx === 0) return props.path.startsWith("\\") ? "\\" : "/";
  return props.path.slice(0, idx);
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

/** 由 FilePermissions 的九个布尔位拼出 rwx 权限字符串(如 rwxr-xr-x),无权限位以 - 表示 */
function fmtPermissions(p: FilePermissions): string {
  const triple = (r: boolean, w: boolean, x: boolean): string =>
    `${r ? "r" : "-"}${w ? "w" : "-"}${x ? "x" : "-"}`;
  return (
    triple(p.owner_read, p.owner_write, p.owner_execute) +
    triple(p.group_read, p.group_write, p.group_execute) +
    triple(p.other_read, p.other_write, p.other_execute)
  );
}

/** stat 失败是否因条目已不存在(ENOENT / os error 2);其余错误必须向用户显示真实原因 */
function isEntryGone(e: unknown): boolean {
  const msg = String(e).toLowerCase();
  return msg.includes("enoent") || msg.includes("no such file") || msg.includes("os error 2");
}

/**
 * Tauri 的 fs scope 拒绝时，后端原文会指向 capability 文件（"maybe it is not
 * allowed on the scope for `allow-read-dir` permission in your capability file"），
 * 这在「目录是用户自己选的」场景下是误导——真实原因是进程重启后运行时 scope
 * 为空。改写成用户可行动的说法，但不吞掉失败：仍然展示是哪个目录出的问题。
 */
function describeLoadError(e: unknown, path: string): string {
  const raw = String(e);
  if (/forbidden path|not allowed on the scope|scope/i.test(raw)) {
    return `无权读取 ${path}：该目录尚未授权。请点击上方「本地：… · 更换」重新选择。`;
  }
  return raw;
}

async function load(path: string, pushHistory = true) {
  loading.value = true;
  errorText.value = null;
  selected.value = new Set();
  emit("selection-change", []);
  try {
    if (props.mode === "remote" && props.sessionId) {
      const r = await browseRemoteDir(props.sessionId, path);
      entries.value = r.entries.map((entry) => ({
        name: entry.name,
        size: entry.size,
        is_dir: entry.file_type === "Directory",
        modified: entry.modified && /^\d+$/.test(entry.modified)
          ? new Date(Number(entry.modified) * 1000).toISOString() : entry.modified,
        owner: entry.owner,
        mode: fmtPermissions(entry.permissions),
      }));
    } else if (props.mode === "remote") {
      throw new Error("请先连接 SSH 会话");
    } else if (props.mode === "local" && path) {
      const items = await readDir(path);
      entries.value = await Promise.all(items.map(async (item) => {
        const fullPath = joinPath(path, item.name);
        try {
          const details = await stat(fullPath);
          return {
            name: item.name,
            size: details.size,
            is_dir: item.isDirectory,
            modified: details.mtime?.toISOString() ?? "",
          };
        } catch (e) {
          // ENOENT = 条目在 readDir 与 stat 之间被删除,允许占位;
          // 其余错误(如 ACL 拒绝)必须抛出,由错误行展示真实原因,禁止静默回退为 0/空
          if (!isEntryGone(e)) throw e;
          return { name: item.name, size: 0, is_dir: item.isDirectory, modified: "" };
        }
      }));
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
    errorText.value = describeLoadError(e, path);
    entries.value = [];
  } finally {
    loading.value = false;
  }
}

function joinPath(base: string, name: string): string {
  return `${base.replace(/\/$/, "")}/${name}`;
}

function refresh() { return load(props.path, false); }
defineExpose({ refresh });

function navigateTo(path: string, pushHistory = true) {
  // 越出已授权根目录的导航直接丢弃（面包屑 / 目录树 / 双击 / 上级都汇到这里）。
  // 判定与 PathBar.commitEdit 共用 isWithinRoot：前缀比较会放过
  // `/home/user/../etc`，且只判 `/` 会误挡 Windows 根 `C:\data`（R3-10）。
  if (props.mode === "local" && props.rootPath && !isWithinRoot(path, props.rootPath)) return;
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
  const parent = parentPath.value;
  if (!parent) return;
  navigateTo(parent);
}

function onRowDblClick(entry: FsEntry) {
  if (entry.is_dir) {
    const next = joinPath(props.path, entry.name);
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

/**
 * 右键：若点在当前选中集之外，先把选中切到该行（标准文件管理器行为），
 * 否则菜单会作用到一批用户并未指向的文件。已选行上则保持整个多选集。
 */
function onRowContextMenu(entry: FsEntry, event: MouseEvent) {
  event.preventDefault();
  let targets: FsEntry[];
  if (selected.value.has(entry.name)) {
    targets = entries.value.filter((x) => selected.value.has(x.name));
  } else {
    selected.value = new Set([entry.name]);
    targets = [entry];
    emit("selection-change", targets);
  }
  emit("row-context-menu", { entries: targets, x: event.clientX, y: event.clientY });
}

/** 列表空白处右键：只提供面板级操作（新建目录/刷新），不带文件目标 */
function onPaneContextMenu(event: MouseEvent) {
  // 表格行自带处理，行内点击不冒泡到这里；这里只接空白
  if ((event.target as HTMLElement).closest(".el-table__row")) return;
  event.preventDefault();
  emit("row-context-menu", { entries: [], x: event.clientX, y: event.clientY });
}

watch(
  () => [props.path, props.sessionId] as const,
  ([p, session], old) => {
    if ((!old || p !== old[0] || session !== old[1]) && !props.externallyNavigated) void load(p);
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
      <button
        class="path-btn"
        :disabled="!parentPath"
        title="上级目录"
        data-test="fs-up"
        @click="goUp"
      >
        <svg width="12" height="12" viewBox="0 0 16 16"><path d="M3 8 H13 M8 3 V13" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>
      </button>
      <PathBar
        :mode="mode"
        :session-id="sessionId"
        :path="path"
        :root-path="rootPath"
        @navigate="navigateTo"
      />
      <button class="path-btn" title="刷新" @click="load(path, false)">
        <svg width="12" height="12" viewBox="0 0 16 16"><path d="M13 8 A5 5 0 1 1 11.5 4.2 M11.5 2.5 V5 H9" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </button>
      <button
        v-if="mode === 'local'"
        class="path-btn choose-btn"
        data-test="fs-choose-root"
        :title="hasRoot ? '更换本地目录' : '选择本地目录'"
        :aria-label="hasRoot ? '更换本地目录' : '选择本地目录'"
        @click="emit('choose-root')"
      >
        <svg width="12" height="12" viewBox="0 0 16 16">
          <path d="M1.5 12.5 V4 H6 L7.5 5.5 H14.5 V12.5 Z" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" />
        </svg>
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
    <div ref="listWrapRef" class="list-wrap" @contextmenu="onPaneContextMenu">
      <p v-if="errorText" class="err">{{ errorText }}</p>
      <el-table
        :data="visibleEntries"
        :loading="loading"
        :show-header="true"
        :height="tableHeight"
        size="small"
        :empty-text="mode === 'local' && !path ? '先选择本地文件夹' : '空目录'"
        class="fs-table"
        @row-dblclick="onRowDblClick"
        @row-click="(row: FsEntry) => onRowClick(row)"
        @row-contextmenu="(row: FsEntry, _column: unknown, event: MouseEvent) => onRowContextMenu(row, event)"
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

/* 「选择/更换本地目录」入口：与刷新同级，置于搜索框左侧 */
.choose-btn { color: var(--rs-accent); }
.choose-btn:hover:not(:disabled) { color: var(--rs-accent); }

/* 面包屑 / 输入框 / 目录树的样式都在 PathBar 里，这里只留搜索框 */
.search {
  width: 120px;
  flex-shrink: 0;
}

/* 滚动交给 el-table 自己（表头固定 + 表格底部横向滚动条），
   外层只负责占满剩余高度，不参与滚动 */
.list-wrap {
  flex: 1;
  overflow: hidden;
  min-height: 0;
  position: relative;
}
.fs-table {
  --el-table-bg-color: var(--rs-bg-panel);
  --el-table-tr-bg-color: var(--rs-bg-panel);
  --el-table-row-hover-bg-color: var(--rs-row-hover);
  --el-table-border-color: var(--rs-border);
  width: 100%;
}
/* 横向滚动条贴在表格底部：滚动条槽与主题一致，避免默认浅色条割裂面板 */
.fs-table :deep(.el-scrollbar__bar.is-horizontal) {
  height: 8px;
}
.fs-table :deep(.el-scrollbar__bar.is-horizontal .el-scrollbar__thumb) {
  background-color: var(--rs-fg-disabled);
  opacity: 0.8;
}
.fs-table :deep(.el-scrollbar__bar.is-vertical) {
  width: 8px;
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
