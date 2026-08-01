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
 * 同步浏览:开启后,左窗格 navigate 也会驱动右窗格 navigate(单向);
 *         反向亦然(用 externallyNavigated 标志避免循环)。
 * 拖拽:左→右 = 上传;右→左 = 下载(emit,TransferPanel 处理实际入队)。
 * 分隔条可拖动改变窗格比例。
 */
import { computed, ref, watch } from "vue";
import FileBrowserPane, { type FsEntry } from "./FileBrowserPane.vue";
import type { Uuid } from "../../ipc/types";
import {
  enqueueUpload,
  enqueueDownload,
} from "../../ipc/client";

const props = defineProps<{
  sessionId?: Uuid;
  remotePath?: string;
  localPath?: string;
  syncEnabled?: boolean;
}>();

const emit = defineEmits<{
  (e: "upload-queued", count: number): void;
  (e: "download-queued", count: number): void;
  (e: "remote-path", path: string): void;
  (e: "local-path", path: string): void;
}>();

const internalLocalPath = ref(props.localPath || "C:\\code\\pmi\\datasave\\machines");
const internalRemotePath = ref(props.remotePath || "/root/pmi/datasave/machines");
const splitPct = ref(50);
const leftSyncFlag = ref(false);
const rightSyncFlag = ref(false);
const splitDragging = ref(false);

watch(() => props.localPath, (v) => { if (v && v !== internalLocalPath.value) internalLocalPath.value = v; });
watch(() => props.remotePath, (v) => { if (v && v !== internalRemotePath.value) internalRemotePath.value = v; });

const localMock = ref<FsEntry[]>([
  { name: "..", size: 0, is_dir: true, modified: "2026-08-01T15:17:00" },
  { name: "machine_m01986.json", size: 118 * 1024, is_dir: false, modified: "2026-08-01T15:17:00" },
]);
const remoteMock = ref<FsEntry[]>([
  { name: "..", size: 0, is_dir: true, modified: "2026-08-01T15:15:00" },
  { name: "machine_m01986.json.bak", size: 134 * 1024, is_dir: false, modified: "2026-07-21T11:03:00", mode: "-rw-r--r--", owner: "root" },
  { name: "machine_m01986.json", size: 117 * 1024, is_dir: false, modified: "2026-08-01T15:15:00", mode: "-rw-r--r--", owner: "root" },
  { name: "machine_m01986.json.bak.1", size: 186 * 1024, is_dir: false, modified: "2026-07-21T15:08:00", mode: "-rw-r--r--", owner: "root" },
]);

function onLocalNavigate(path: string) {
  internalLocalPath.value = path;
  emit("local-path", path);
  if (props.syncEnabled && !leftSyncFlag.value) {
    leftSyncFlag.value = true;
    internalRemotePath.value = path;
    emit("remote-path", path);
    setTimeout(() => (leftSyncFlag.value = false), 0);
  }
}
function onRemoteNavigate(path: string) {
  internalRemotePath.value = path;
  emit("remote-path", path);
  if (props.syncEnabled && !rightSyncFlag.value) {
    rightSyncFlag.value = true;
    internalLocalPath.value = path;
    emit("local-path", path);
    setTimeout(() => (rightSyncFlag.value = false), 0);
  }
}

function onLocalSyncRequest(path: string) {
  if (props.syncEnabled && !rightSyncFlag.value) {
    rightSyncFlag.value = true;
    internalRemotePath.value = path;
    emit("remote-path", path);
    setTimeout(() => (rightSyncFlag.value = false), 0);
  }
}
function onRemoteSyncRequest(path: string) {
  if (props.syncEnabled && !leftSyncFlag.value) {
    leftSyncFlag.value = true;
    internalLocalPath.value = path;
    emit("local-path", path);
    setTimeout(() => (leftSyncFlag.value = false), 0);
  }
}

async function uploadSelected(entries: FsEntry[]) {
  if (!props.sessionId) return;
  let count = 0;
  for (const e of entries) {
    if (e.name === ".." || e.is_dir) continue;
    try {
      const local = internalLocalPath.value.endsWith("\\")
        ? `${internalLocalPath.value}${e.name}`
        : `${internalLocalPath.value}/${e.name}`;
      const remote = internalRemotePath.value.endsWith("/")
        ? `${internalRemotePath.value}${e.name}`
        : `${internalRemotePath.value}/${e.name}`;
      await enqueueUpload(local, remote, props.sessionId);
      count++;
    } catch (err) {
      console.warn("upload failed", err);
    }
  }
  emit("upload-queued", count);
}

async function downloadSelected(entries: FsEntry[]) {
  if (!props.sessionId) return;
  let count = 0;
  for (const e of entries) {
    if (e.name === ".." || e.is_dir) continue;
    try {
      const remote = internalRemotePath.value.endsWith("/")
        ? `${internalRemotePath.value}${e.name}`
        : `${internalRemotePath.value}/${e.name}`;
      const local = internalLocalPath.value.endsWith("\\")
        ? `${internalLocalPath.value}${e.name}`
        : `${internalLocalPath.value}/${e.name}`;
      await enqueueDownload(remote, local, props.sessionId);
      count++;
    } catch (err) {
      console.warn("download failed", err);
    }
  }
  emit("download-queued", count);
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
      <FileBrowserPane
        mode="local"
        :path="internalLocalPath"
        :mock-entries="localMock"
        @navigate="onLocalNavigate"
        @request-sync="onLocalSyncRequest"
        @selection-change="(s) => uploadSelected(s)"
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
        mode="remote"
        :session-id="sessionId"
        :path="internalRemotePath"
        :mock-entries="remoteMock"
        :externally-navigated="rightSyncFlag"
        @navigate="onRemoteNavigate"
        @request-sync="onRemoteSyncRequest"
        @selection-change="(s) => downloadSelected(s)"
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