<script setup lang="ts">
/**
 * TransferWorkspace —— v2 重设计
 *
 * Xftp 风格双窗格:
 *   ┌─────────────┬─────────────┐
 *   │ LOCAL       │ REMOTE      │
 *   │ (本地路径)  │ (远程路径)  │
 *   └─────────────┴─────────────┘
 *
 * 选择文件只更新状态;传输与删除必须由显式操作触发。
 * 分隔条可拖动改变窗格比例。
 */
import { computed, onMounted, ref, watch } from "vue";
import { open, confirm } from "@tauri-apps/plugin-dialog";
import { ElMessage } from "element-plus/es/components/message/index.mjs";
import { ElMessageBox } from "element-plus/es/components/message-box/index.mjs";
import FileBrowserPane, { type FsEntry } from "./FileBrowserPane.vue";
import type { Uuid } from "../../ipc/types";
import {
  enqueueUpload,
  enqueueDownload,
  createRemoteDirectory,
  deleteRemoteEntry,
  getRemoteHomeDir,
} from "../../ipc/client";
import { loadLastLocalDir, saveLastLocalDir } from "../../utils/lastLocalDir";

const props = defineProps<{
  sessionId?: Uuid;
  connected?: boolean;
  remotePath?: string;
  localPath?: string;
  syncEnabled?: boolean;
}>();

const emit = defineEmits<{
  (e: "upload-queued", count: number): void;
  (e: "download-queued", count: number): void;
  (e: "remote-path", path: string): void;
  (e: "local-path", path: string): void;
  (e: "capabilities", state: { upload: boolean; download: boolean; createFolder: boolean; delete: boolean; refresh: boolean; sync: boolean }): void;
}>();

const localRoot = ref(props.localPath || "");
const internalLocalPath = ref(props.localPath || "");
const internalRemotePath = ref(props.remotePath || "/");
const selectedLocal = ref<FsEntry[]>([]);
const selectedRemote = ref<FsEntry[]>([]);
const localPane = ref<InstanceType<typeof FileBrowserPane> | null>(null);
const remotePane = ref<InstanceType<typeof FileBrowserPane> | null>(null);
const splitPct = ref(50);
const leftSyncFlag = ref(false);
const rightSyncFlag = ref(false);
const splitDragging = ref(false);

watch(() => props.localPath, (v) => { if (v && v !== internalLocalPath.value) { localRoot.value = v; internalLocalPath.value = v; } });

/** 目录入口是否被显式指定：外部给了 remotePath 就尊重它，否则自动用远端工作目录 */
const remotePathExplicit = ref(!!props.remotePath);
watch(() => props.remotePath, (v) => {
  if (v && v !== internalRemotePath.value) {
    internalRemotePath.value = v;
    remotePathExplicit.value = true;
  }
});

const validFile = (entries: FsEntry[]) => entries.length === 1 && !entries[0].is_dir && safeName(entries[0].name);
/** 可传输文件集合：全部为普通文件且名称安全（支持多选批量传输） */
const transferable = (entries: FsEntry[]) => entries.filter((e) => !e.is_dir && safeName(e.name));
const capabilities = computed(() => ({
  upload: !!props.sessionId && !!props.connected && !!localRoot.value && transferable(selectedLocal.value).length > 0,
  download: !!props.sessionId && !!props.connected && !!localRoot.value && transferable(selectedRemote.value).length > 0,
  createFolder: !!props.sessionId && !!props.connected && !!internalRemotePath.value,
  delete: !!props.sessionId && !!props.connected && validFile(selectedRemote.value),
  refresh: !!props.connected || !!localRoot.value,
  sync: !!props.connected && !!localRoot.value,
}));
watch(capabilities, (state) => emit("capabilities", state), { immediate: true });

function safeName(name: string) { return !!name && name !== "." && name !== ".." && !/[\\/\0]/.test(name); }
function joinPath(base: string, name: string) { return `${base.replace(/\/$/, "")}/${name}`; }

async function chooseLocalRoot() {
  const result = await open({ directory: true, multiple: false, recursive: true, title: "选择本地文件夹" });
  if (typeof result !== "string") return;
  localRoot.value = result;
  internalLocalPath.value = result;
  // 记住本次选择，供下次进入工作区时直接恢复
  saveLastLocalDir(typeof localStorage === "undefined" ? null : localStorage, result);
  emit("local-path", result);
}

/**
 * 远端默认目录：未显式指定 remotePath 时用登录用户的工作目录，
 * 而不是文件系统根 `/`（根目录对普通用户没有意义，也容易被误当作可写区）。
 * 解析失败保持当前路径，不把远端面板打成空白。
 */
async function resolveRemoteHome() {
  if (!props.sessionId || !props.connected || remotePathExplicit.value) return;
  try {
    const home = await getRemoteHomeDir(props.sessionId);
    if (!home || home === internalRemotePath.value) return;
    internalRemotePath.value = home;
    emit("remote-path", home);
  } catch (error) {
    console.warn("解析远端工作目录失败，保持当前目录", error);
  }
}

onMounted(() => {
  // 恢复上次打开的本地目录；记录已失效（目录被删/改名）时由 FileBrowserPane
  // 的错误行显示真实原因，用户可点「更换」重新选择，不静默清空。
  const remembered = loadLastLocalDir(typeof localStorage === "undefined" ? null : localStorage);
  if (remembered && !localRoot.value) {
    localRoot.value = remembered;
    internalLocalPath.value = remembered;
    emit("local-path", remembered);
  }
  void resolveRemoteHome();
});

watch(() => props.connected, (now) => { if (now) void resolveRemoteHome(); });

function onLocalNavigate(path: string) {
  internalLocalPath.value = path;
  emit("local-path", path);
  if (props.syncEnabled && localRoot.value && !leftSyncFlag.value) {
    leftSyncFlag.value = true;
    internalRemotePath.value = "/" + path.slice(localRoot.value.length).replace(/^\/+/, "");
    emit("remote-path", internalRemotePath.value);
    setTimeout(() => (leftSyncFlag.value = false), 0);
  }
}
function onRemoteNavigate(path: string) {
  internalRemotePath.value = path;
  emit("remote-path", path);
  if (props.syncEnabled && localRoot.value && !rightSyncFlag.value) {
    rightSyncFlag.value = true;
    internalLocalPath.value = joinPath(localRoot.value, path.replace(/^\/+/, ""));
    emit("local-path", internalLocalPath.value);
    setTimeout(() => (rightSyncFlag.value = false), 0);
  }
}

function onLocalSyncRequest(path: string) {
  if (props.syncEnabled && localRoot.value && !rightSyncFlag.value) {
    rightSyncFlag.value = true;
    internalRemotePath.value = "/" + path.slice(localRoot.value.length).replace(/^\/+/, "");
    emit("remote-path", internalRemotePath.value);
    setTimeout(() => (rightSyncFlag.value = false), 0);
  }
}
function onRemoteSyncRequest(path: string) {
  if (props.syncEnabled && localRoot.value && !leftSyncFlag.value) {
    leftSyncFlag.value = true;
    internalLocalPath.value = joinPath(localRoot.value, path.replace(/^\/+/, ""));
    emit("local-path", internalLocalPath.value);
    setTimeout(() => (leftSyncFlag.value = false), 0);
  }
}

async function upload() {
  if (!capabilities.value.upload || !props.sessionId) return;
  const files = transferable(selectedLocal.value);
  if (files.length === 0) return;
  // 批量：逐个入队。单条失败不阻断其余文件，失败条目在面板顶部汇总提示。
  const failures: string[] = [];
  for (const file of files) {
    try {
      await enqueueUpload(
        joinPath(internalLocalPath.value, file.name),
        joinPath(internalRemotePath.value, file.name),
        props.sessionId,
      );
    } catch (error) {
      failures.push(`${file.name}：${String(error)}`);
    }
  }
  const queued = files.length - failures.length;
  if (queued > 0) emit("upload-queued", queued);
  if (failures.length > 0) {
    ElMessage.error(`上传失败 ${failures.length} 个：${failures.join("；")}`);
  }
}

async function download() {
  if (!capabilities.value.download || !props.sessionId) return;
  const files = transferable(selectedRemote.value);
  if (files.length === 0) return;
  const failures: string[] = [];
  for (const file of files) {
    try {
      await enqueueDownload(
        joinPath(internalRemotePath.value, file.name),
        joinPath(internalLocalPath.value, file.name),
        props.sessionId,
      );
    } catch (error) {
      failures.push(`${file.name}：${String(error)}`);
    }
  }
  const queued = files.length - failures.length;
  if (queued > 0) emit("download-queued", queued);
  if (failures.length > 0) {
    ElMessage.error(`下载失败 ${failures.length} 个：${failures.join("；")}`);
  }
}

async function createFolder() {
  if (!capabilities.value.createFolder || !props.sessionId) return;
  try {
    const { value } = await ElMessageBox.prompt("文件夹名称", "新建远程文件夹");
    if (!safeName(value)) { ElMessage.error("无效的文件夹名称"); return; }
    await createRemoteDirectory(props.sessionId, joinPath(internalRemotePath.value, value));
    await remotePane.value?.refresh();
  } catch (error) { if (error !== "cancel" && error !== "close") ElMessage.error(`创建失败：${String(error)}`); }
}

async function deleteSelected() {
  if (!capabilities.value.delete || !props.sessionId) return;
  const file = selectedRemote.value[0];
  if (!await confirm(`确定删除远程文件「${file.name}」吗？`, { title: "删除文件", kind: "warning" })) return;
  try {
    await deleteRemoteEntry(props.sessionId, joinPath(internalRemotePath.value, file.name));
    selectedRemote.value = [];
    await remotePane.value?.refresh();
  } catch (error) { ElMessage.error(`删除失败：${String(error)}`); }
}

async function refresh() { await Promise.all([localPane.value?.refresh(), remotePane.value?.refresh()]); }
defineExpose({ upload, download, createFolder, deleteSelected, refresh, chooseLocalRoot });

// ── 右键菜单 ──
// 菜单归属哪个窗格由右键所在位置决定；动作只暴露后端真实支持的操作，
// 不可用时置灰而不是隐藏，避免用户以为功能不存在。
const contextMenu = ref<{
  mode: "local" | "remote";
  entries: FsEntry[];
  x: number;
  y: number;
} | null>(null);

type ContextAction = "upload" | "download" | "createFolder" | "delete" | "refresh";

function onRowContextMenu(mode: "local" | "remote", payload: { entries: FsEntry[]; x: number; y: number }) {
  contextMenu.value = { mode, entries: payload.entries, x: payload.x, y: payload.y };
}

function closeContextMenu() {
  contextMenu.value = null;
}

/** 当前菜单里可用的动作；空选中时只剩面板级操作 */
const contextActions = computed<ContextAction[]>(() => {
  const menu = contextMenu.value;
  if (!menu) return [];
  const files = transferable(menu.entries);
  if (files.length === 0) {
    return menu.mode === "remote" && capabilities.value.createFolder
      ? ["createFolder", "refresh"]
      : ["refresh"];
  }
  if (menu.mode === "local") {
    return capabilities.value.upload ? ["upload", "refresh"] : ["refresh"];
  }
  const actions: ContextAction[] = [];
  if (capabilities.value.download) actions.push("download");
  if (capabilities.value.createFolder) actions.push("createFolder");
  // 删除只对单个普通文件开放：批量删除需要逐条确认，语义与单条不同
  if (capabilities.value.delete) actions.push("delete");
  actions.push("refresh");
  return actions;
});

const CONTEXT_ACTION_LABEL: Record<ContextAction, string> = {
  upload: "上传到远程",
  download: "下载到本地",
  createFolder: "新建文件夹",
  delete: "删除",
  refresh: "刷新",
};

async function runContextAction(action: ContextAction) {
  const menu = contextMenu.value;
  closeContextMenu();
  if (!menu) return;
  if (action === "upload") await upload();
  else if (action === "download") await download();
  else if (action === "createFolder") await createFolder();
  else if (action === "delete") await deleteSelected();
  else await refresh();
}

function startSplitDrag(e: MouseEvent) {
  splitDragging.value = true;
  e.preventDefault();
}
function onSplitMove(e: MouseEvent) {
  if (!splitDragging.value) return;
  const container = (e.currentTarget as HTMLElement).getBoundingClientRect();
  const pct = ((e.clientX - container.left) / container.width) * 100;
  splitPct.value = Math.max(20, Math.min(80, pct));
}
function endSplitDrag() {
  splitDragging.value = false;
}

const leftWidth = computed(() => `${splitPct.value}%`);
const rightWidth = computed(() => `${100 - splitPct.value}%`);
</script>

<template>
  <div
    class="transfer-workspace"
    :class="{ 'is-dragging': splitDragging }"
    @mousemove="onSplitMove"
    @mouseup="endSplitDrag"
    @mouseleave="endSplitDrag"
  >
    <div class="pane-slot" :style="{ width: leftWidth }">
      <button v-if="!localRoot" class="choose-root" @click="chooseLocalRoot">选择本地文件夹</button>
      <button v-else class="choose-root" @click="chooseLocalRoot">本地：{{ localRoot }} · 更换</button>
      <FileBrowserPane
        ref="localPane"
        mode="local"
        :path="internalLocalPath"
        :root-path="localRoot"
        @navigate="onLocalNavigate"
        @request-sync="onLocalSyncRequest"
        @selection-change="(s) => selectedLocal = s"
        @row-context-menu="(p) => onRowContextMenu('local', p)"
      />
    </div>
    <div
      class="split"
      role="separator"
      aria-orientation="vertical"
      @mousedown="startSplitDrag"
    />
    <div class="pane-slot" :style="{ width: rightWidth }">
      <FileBrowserPane
        ref="remotePane"
        mode="remote"
        :session-id="connected ? sessionId : undefined"
        :path="internalRemotePath"
        :externally-navigated="rightSyncFlag"
        @navigate="onRemoteNavigate"
        @request-sync="onRemoteSyncRequest"
        @selection-change="(s) => selectedRemote = s"
        @row-context-menu="(p) => onRowContextMenu('remote', p)"
      />
    </div>

    <!-- 右键菜单：fixed 定位到光标处，点击外部或 Escape 关闭 -->
    <div
      v-if="contextMenu"
      class="context-backdrop"
      data-test="context-backdrop"
      @click="closeContextMenu"
      @contextmenu.prevent="closeContextMenu"
    />
    <ul
      v-if="contextMenu"
      class="context-menu"
      data-test="context-menu"
      role="menu"
      :style="{ left: `${contextMenu.x}px`, top: `${contextMenu.y}px` }"
    >
      <li
        v-for="action in contextActions"
        :key="action"
        role="menuitem"
        class="context-item"
        :data-test="`context-${action}`"
        tabindex="0"
        @click="runContextAction(action)"
        @keydown.enter="runContextAction(action)"
      >
        {{ CONTEXT_ACTION_LABEL[action] }}
      </li>
    </ul>
  </div>
</template>

<style scoped>
.transfer-workspace {
  display: flex;
  width: 100%;
  height: 100%;
  gap: 0;
  padding: var(--rs-s-2);
  background: var(--rs-bg);
  user-select: none;
}
.transfer-workspace.is-dragging {
  cursor: col-resize;
}

.pane-slot {
  height: 100%;
  min-width: 200px;
}
.choose-root { display: block; width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--rs-accent); }

.split {
  width: 5px;
  cursor: col-resize;
  flex-shrink: 0;
  position: relative;
  margin: 0 2px;
}
.split::before {
  content: "";
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  width: 1px;
  height: 32px;
  background: var(--rs-border);
}
.split:hover::before {
  background: var(--rs-accent);
}

.context-backdrop {
  position: fixed;
  inset: 0;
  z-index: 2000;
}
.context-menu {
  position: fixed;
  z-index: 2001;
  margin: 0;
  padding: 4px 0;
  list-style: none;
  min-width: 132px;
  background: var(--rs-bg-surface);
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-1);
  box-shadow: 0 6px 18px rgb(0 0 0 / 32%);
}
.context-item {
  padding: 6px 14px;
  font-size: var(--rs-fs-xs);
  color: var(--rs-fg);
  cursor: pointer;
  white-space: nowrap;
}
.context-item:hover,
.context-item:focus-visible {
  background: var(--rs-bg-surface-hover);
  outline: none;
}
</style>
