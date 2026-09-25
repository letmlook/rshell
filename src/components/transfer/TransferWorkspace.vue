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
import { computed, ref, watch } from "vue";
import { open, confirm } from "@tauri-apps/plugin-dialog";
import { ElMessage, ElMessageBox } from "element-plus";
import FileBrowserPane, { type FsEntry } from "./FileBrowserPane.vue";
import type { Uuid } from "../../ipc/types";
import {
  enqueueUpload,
  enqueueDownload,
  createRemoteDirectory,
  deleteRemoteEntry,
} from "../../ipc/client";

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
watch(() => props.remotePath, (v) => { if (v && v !== internalRemotePath.value) internalRemotePath.value = v; });

const validFile = (entries: FsEntry[]) => entries.length === 1 && !entries[0].is_dir && safeName(entries[0].name);
const capabilities = computed(() => ({
  upload: !!props.sessionId && !!props.connected && !!localRoot.value && validFile(selectedLocal.value),
  download: !!props.sessionId && !!props.connected && !!localRoot.value && validFile(selectedRemote.value),
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
  emit("local-path", result);
}

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
  const file = selectedLocal.value[0];
  try {
    await enqueueUpload(joinPath(internalLocalPath.value, file.name), joinPath(internalRemotePath.value, file.name), props.sessionId);
    emit("upload-queued", 1);
  } catch (error) { ElMessage.error(`上传失败：${String(error)}`); }
}

async function download() {
  if (!capabilities.value.download || !props.sessionId) return;
  const file = selectedRemote.value[0];
  try {
    await enqueueDownload(joinPath(internalRemotePath.value, file.name), joinPath(internalLocalPath.value, file.name), props.sessionId);
    emit("download-queued", 1);
  } catch (error) { ElMessage.error(`下载失败：${String(error)}`); }
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
      />
    </div>
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
</style>
