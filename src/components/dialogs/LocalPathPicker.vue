<script setup lang="ts">
/**
 * LocalPathPicker —— 应用内本地文件/文件夹选择窗体
 *
 * 替代 `tauri-plugin-dialog` 的 `open()`：后者弹的是操作系统原生选择窗口，
 * 与本应用外观完全割裂，而且在部分 WebView 上还会出现「点了没反应」的授权
 * 困惑。本组件直接用已授权的 `fs:read-all` 读目录，窗体、导航、样式都是自己的。
 *
 * 能力：
 *   - 面包屑 + 手工输入路径（复用 PathBar），支持 Windows 盘符与 POSIX 根
 *   - 目录树下拉（复用 PathBar 的懒加载树）
 *   - 目录/文件列表：双击目录进入，单击文件选中，隐藏目录过滤开关
 *   - 目录读取失败就地显示真实原因（权限/不存在），不显示成空目录
 *
 * 两种模式：
 *   directory —— 选当前所在目录（「选择此文件夹」），只列目录
 *   file      —— 选具体文件（「打开」按钮，仅选中文件时可用）
 */
import { computed, onMounted, ref } from "vue";
import { readDir, stat } from "@tauri-apps/plugin-fs";
import { homeDir } from "@tauri-apps/api/path";
import PathBar from "../transfer/PathBar.vue";

interface PickerEntry {
  name: string;
  isDirectory: boolean;
  size: number;
  modified: string;
}

const props = withDefaults(
  defineProps<{
    open: boolean;
    mode: "directory" | "file";
    title: string;
    /** 初始目录；不给则用用户主目录 */
    initialPath?: string;
    /** file 模式下的文件过滤；返回 false 的条目不列出 */
    accept?: (name: string) => boolean;
    /** 目录模式：是否显示"新建文件夹" */
    allowCreateDirectory?: boolean;
  }>(),
  { allowCreateDirectory: false },
);

const emit = defineEmits<{
  (e: "close", selected: string | null): void;
}>();

const currentPath = ref("");
const entries = ref<PickerEntry[]>([]);
const loading = ref(false);
const errorText = ref<string | null>(null);
const selectedFile = ref<string | null>(null);
const showHidden = ref(false);
const newFolderVisible = ref(false);
const newFolderName = ref("");

const parentPath = computed<string | null>(() => {
  const path = currentPath.value;
  if (!path) return null;
  const idx = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  if (idx < 0) return null;
  if (idx === 0) return path.startsWith("\\") ? "\\" : "/";
  return path.slice(0, idx);
});

/** Windows 根目录（`C:\`）没有上级；UNC 路径按普通路径处理 */
const canGoUp = computed(() => !!parentPath.value && !/^[A-Za-z]:[\\/]?$/.test(currentPath.value));

const visibleEntries = computed(() => {
  const filter = props.accept;
  return entries.value.filter((entry) => {
    if (props.mode === "directory" && !entry.isDirectory) return false;
    if (!showHidden.value && entry.name.startsWith(".")) return false;
    if (filter && !entry.isDirectory && !filter(entry.name)) return false;
    return true;
  });
});

const canSubmit = computed(() =>
  props.mode === "directory" ? !!currentPath.value : !!selectedFile.value,
);

function fmtSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function fmtModified(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return `${d.getFullYear()}/${String(d.getMonth() + 1).padStart(2, "0")}/${String(d.getDate()).padStart(2, "0")} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

function joinPath(base: string, name: string): string {
  const sep = base.includes("\\") && !base.includes("/") ? "\\" : "/";
  return `${base.replace(/[\\/]+$/, "")}${sep}${name}`;
}

/** 权限/不存在等失败必须显示真实原因，不能退化成空目录 */
function describeError(e: unknown, path: string): string {
  const raw = String(e);
  if (/not found|no such file|enoent|os error 2/i.test(raw)) return `目录不存在：${path}`;
  if (/forbidden|not allowed on the scope|permission|os error 5/i.test(raw)) {
    return `无权读取 ${path}。`;
  }
  return raw;
}

async function load(path: string) {
  if (!path) return;
  loading.value = true;
  errorText.value = null;
  try {
    const items = await readDir(path);
    const detailed = await Promise.all(
      items.map(async (item) => {
        const full = joinPath(path, item.name);
        try {
          const info = await stat(full);
          return {
            name: item.name,
            isDirectory: item.isDirectory,
            size: info.size ?? 0,
            modified: info.mtime ? new Date(info.mtime).toISOString() : "",
          };
        } catch (e) {
          // 条目在 readDir 与 stat 之间被删掉是常态，占位即可；
          // 其余错误（ACL 拒绝）不该让整页变成空目录
          if (!/not found|no such file|enoent|os error 2/i.test(String(e))) throw e;
          return { name: item.name, isDirectory: item.isDirectory, size: 0, modified: "" };
        }
      }),
    );
    detailed.sort((a, b) => {
      if (a.isDirectory !== b.isDirectory) return a.isDirectory ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
    entries.value = detailed;
    currentPath.value = path;
    selectedFile.value = null;
  } catch (e) {
    errorText.value = describeError(e, path);
    entries.value = [];
  } finally {
    loading.value = false;
  }
}

function goTo(path: string) {
  if (path) void load(path);
}

function goUp() {
  const parent = parentPath.value;
  if (parent && canGoUp.value) void load(parent);
}

function onRowClick(entry: PickerEntry) {
  if (entry.isDirectory) {
    selectedFile.value = null;
    return;
  }
  selectedFile.value = joinPath(currentPath.value, entry.name);
}

function onRowDblClick(entry: PickerEntry) {
  if (entry.isDirectory) void load(joinPath(currentPath.value, entry.name));
}

function submit() {
  if (!canSubmit.value) return;
  emit("close", props.mode === "directory" ? currentPath.value : selectedFile.value);
}

function cancel() {
  emit("close", null);
}

async function createDirectory() {
  const name = newFolderName.value.trim();
  if (!name || /[\\/]/.test(name) || name === "." || name === "..") {
    errorText.value = "请输入不含 \\ / 的文件夹名称";
    return;
  }
  const { mkdir } = await import("@tauri-apps/plugin-fs");
  try {
    await mkdir(joinPath(currentPath.value, name), { recursive: false });
    newFolderVisible.value = false;
    newFolderName.value = "";
    await load(currentPath.value);
  } catch (e) {
    errorText.value = `新建文件夹失败：${String(e)}`;
  }
}

onMounted(async () => {
  if (!props.open) return;
  let start = props.initialPath ?? "";
  if (!start) {
    try {
      start = await homeDir();
    } catch {
      start = "";
    }
  }
  await load(start);
});
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      class="picker-backdrop"
      data-test="picker-backdrop"
      @mousedown.self="cancel"
      @keydown.esc="cancel"
    >
      <div class="picker" role="dialog" aria-modal="true" :aria-label="title" data-test="picker">
        <header class="picker-head">
          <h3>{{ title }}</h3>
          <button class="icon-btn" aria-label="关闭" data-test="picker-close" @click="cancel">
            <svg width="12" height="12" viewBox="0 0 16 16">
              <path d="M4 4 L12 12 M12 4 L4 12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
            </svg>
          </button>
        </header>

        <div class="picker-bar">
          <button class="icon-btn" :disabled="!canGoUp" title="上级目录" data-test="picker-up" @click="goUp">
            <svg width="12" height="12" viewBox="0 0 16 16">
              <path d="M3 8 H13 M8 3 V13" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
            </svg>
          </button>
          <PathBar mode="local" :path="currentPath" @navigate="goTo" />
          <button class="icon-btn" title="刷新" data-test="picker-refresh" @click="load(currentPath)">
            <svg width="12" height="12" viewBox="0 0 16 16">
              <path d="M13 8 A5 5 0 1 1 11.5 4.2 M11.5 2.5 V5 H9" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
            </svg>
          </button>
          <label class="hidden-toggle" title="显示以点开头的隐藏项">
            <input v-model="showHidden" type="checkbox" />
            隐藏项
          </label>
        </div>

        <p v-if="errorText" class="picker-error" role="alert" data-test="picker-error">{{ errorText }}</p>

        <div v-if="newFolderVisible" class="new-folder">
          <input
            v-model="newFolderName"
            class="new-folder-input"
            data-test="picker-new-folder-input"
            placeholder="新文件夹名称"
            @keyup.enter="createDirectory"
          />
          <button class="text-btn" data-test="picker-new-folder-ok" @click="createDirectory">创建</button>
          <button class="text-btn" @click="newFolderVisible = false">取消</button>
        </div>

        <div class="picker-list" data-test="picker-list">
          <p v-if="loading" class="picker-hint">读取中…</p>
          <p v-else-if="visibleEntries.length === 0" class="picker-hint" data-test="picker-empty">
            {{ mode === "directory" ? "此目录没有子目录" : "此目录没有可选文件" }}
          </p>
          <ul v-else class="picker-rows">
            <li
              v-for="entry in visibleEntries"
              :key="entry.name"
              class="picker-row"
              :class="{ 'is-selected': mode === 'file' && selectedFile === joinPath(currentPath, entry.name) }"
              :data-test="`picker-row-${entry.name}`"
              @click="onRowClick(entry)"
              @dblclick="onRowDblClick(entry)"
            >
              <span class="p-icon" :class="entry.isDirectory ? 'is-dir' : 'is-file'" aria-hidden="true">
                <svg v-if="entry.isDirectory" width="14" height="14" viewBox="0 0 16 16">
                  <path d="M2 4.5 V12.5 H14 V6 H8 L6.5 4.5 Z" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" />
                </svg>
                <svg v-else width="14" height="14" viewBox="0 0 16 16">
                  <path d="M3.5 2 H10 L12.5 4.5 V14 H3.5 Z" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" />
                  <path d="M10 2 V4.5 H12.5" fill="none" stroke="currentColor" stroke-width="1.2" />
                </svg>
              </span>
              <span class="p-name" :title="entry.name">{{ entry.name }}</span>
              <span class="p-size">{{ entry.isDirectory ? "" : fmtSize(entry.size) }}</span>
              <span class="p-time">{{ fmtModified(entry.modified) }}</span>
            </li>
          </ul>
        </div>

        <footer class="picker-foot">
          <button
            v-if="allowCreateDirectory"
            class="text-btn"
            data-test="picker-new-folder"
            @click="newFolderVisible = !newFolderVisible"
          >
            新建文件夹
          </button>
          <div class="spacer" />
          <button class="text-btn" data-test="picker-cancel" @click="cancel">取消</button>
          <button
            class="primary-btn"
            data-test="picker-submit"
            :disabled="!canSubmit"
            @click="submit"
          >
            {{ mode === "directory" ? "选择此文件夹" : "打开" }}
          </button>
        </footer>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.picker-backdrop {
  position: fixed;
  inset: 0;
  z-index: 4100;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--rs-s-4);
  background: rgb(0 0 0 / 45%);
}
.picker {
  width: 680px;
  max-width: 100%;
  height: 480px;
  max-height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--rs-bg-surface);
  color: var(--rs-fg);
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-2, 8px);
  box-shadow: 0 12px 32px rgb(0 0 0 / 45%);
  font-family: var(--rs-font-ui);
  overflow: hidden;
}
.picker-head {
  display: flex;
  align-items: center;
  height: 36px;
  padding: 0 var(--rs-s-3);
  border-bottom: 1px solid var(--rs-border);
  flex-shrink: 0;
}
.picker-head h3 {
  flex: 1;
  margin: 0;
  font-size: var(--rs-fs-lg, 15px);
  font-weight: 600;
}
.picker-bar {
  display: flex;
  align-items: center;
  gap: var(--rs-s-1);
  padding: 4px var(--rs-s-2);
  border-bottom: 1px solid var(--rs-border);
  flex-shrink: 0;
}
.icon-btn {
  width: 22px;
  height: 22px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: 1px solid transparent;
  border-radius: var(--rs-radius-1);
  color: var(--rs-fg-muted);
  cursor: pointer;
}
.icon-btn:hover:not(:disabled) { background: var(--rs-bg-surface-hover); color: var(--rs-fg); }
.icon-btn:disabled { opacity: 0.4; cursor: not-allowed; }
.hidden-toggle {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: var(--rs-fs-xs);
  color: var(--rs-fg-muted);
  white-space: nowrap;
  cursor: pointer;
}
.picker-error {
  margin: 0;
  padding: 4px var(--rs-s-3);
  font-size: var(--rs-fs-xs);
  color: var(--rs-p-danger);
  background: var(--rs-bg-panel);
  border-bottom: 1px solid var(--rs-border);
}
.new-folder {
  display: flex;
  gap: var(--rs-s-1);
  padding: 4px var(--rs-s-2);
  border-bottom: 1px solid var(--rs-border);
}
.new-folder-input {
  flex: 1;
  min-width: 0;
  height: 24px;
  padding: 0 6px;
  font-size: var(--rs-fs-xs);
  font-family: var(--rs-font-mono);
  color: var(--rs-fg);
  background: var(--rs-bg-panel);
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-1);
}
.picker-list {
  flex: 1;
  min-height: 0;
  overflow: auto;
  user-select: none;
}
.picker-hint {
  margin: 0;
  padding: var(--rs-s-3);
  font-size: var(--rs-fs-xs);
  color: var(--rs-fg-muted);
}
.picker-rows {
  list-style: none;
  margin: 0;
  padding: 0;
}
.picker-row {
  display: grid;
  grid-template-columns: 18px minmax(0, 1fr) 80px 108px;
  align-items: center;
  gap: var(--rs-s-2);
  padding: 3px var(--rs-s-3);
  font-size: var(--rs-fs-xs);
  cursor: default;
}
.picker-row:hover { background: var(--rs-row-hover); }
.picker-row.is-selected { background: var(--rs-row-selected); }
.p-icon { color: var(--rs-icon-folder); display: inline-flex; }
.p-icon.is-file { color: var(--rs-fg-muted); }
.p-name {
  font-family: var(--rs-font-mono);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.p-size, .p-time {
  text-align: right;
  color: var(--rs-fg-muted);
  font-family: var(--rs-font-mono);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.picker-foot {
  display: flex;
  align-items: center;
  gap: var(--rs-s-2);
  padding: var(--rs-s-2) var(--rs-s-3);
  border-top: 1px solid var(--rs-border);
  flex-shrink: 0;
}
.spacer { flex: 1; }
.text-btn {
  height: 26px;
  padding: 0 var(--rs-s-3);
  font-size: var(--rs-fs-xs);
  font-family: var(--rs-font-ui);
  color: var(--rs-fg);
  background: var(--rs-bg-panel);
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-1);
  cursor: pointer;
}
.text-btn:hover { border-color: var(--rs-accent); }
.primary-btn {
  height: 26px;
  padding: 0 var(--rs-s-3);
  font-size: var(--rs-fs-xs);
  font-family: var(--rs-font-ui);
  color: #fff;
  background: var(--rs-accent);
  border: 1px solid var(--rs-accent);
  border-radius: var(--rs-radius-1);
  cursor: pointer;
}
.primary-btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
