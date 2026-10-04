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
import { readDir } from "@tauri-apps/plugin-fs";
import { ElMessage } from "element-plus/es/components/message/index.mjs";
import FileBrowserPane, { type FsEntry } from "./FileBrowserPane.vue";
import type { ConflictPolicy, Uuid } from "../../ipc/types";
import {
  browseRemoteDir,
  enqueueUpload,
  enqueueDownload,
  createRemoteDirectory,
  deleteRemoteEntry,
  getRemoteHomeDir,
} from "../../ipc/client";
import { loadLastLocalDir, saveLastLocalDir } from "../../utils/lastLocalDir";
import { appendTransferLog } from "../../utils/transferLog";
import { confirmDialog, openDialog, promptDialog } from "../../utils/dialog";
import { pickLocalPath } from "../../utils/pathPicker";

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

/**
 * 目标已存在时的处理选择。
 *
 * 走应用内自定义窗体（utils/dialog），不再用 ElMessageBox：
 * 「覆盖」是破坏性操作，用 danger 样式；「重命名…」再串一个输入窗，
 * 取消输入等同跳过该文件。
 */
type ConflictChoice =
  | { action: "overwrite" }
  | { action: "rename"; name: string }
  | { action: "skip" };

async function askConflict(targetLabel: string, currentName: string): Promise<ConflictChoice> {
  const choice = await openDialog({
    title: "目标已存在同名文件",
    message: `${targetLabel} 已存在同名文件，要如何处理？`,
    detail: targetLabel,
    buttons: [
      { label: "跳过", value: "skip", variant: "ghost" },
      { label: "重命名…", value: "rename", variant: "ghost" },
      { label: "覆盖", value: "overwrite", variant: "danger" },
    ],
  });
  if (choice === "overwrite") return { action: "overwrite" };
  if (choice !== "rename") return { action: "skip" };
  const name = await promptDialog({
    title: "重命名后传输",
    message: `为「${currentName}」输入新的文件名。`,
    defaultValue: currentName,
    confirmText: "传输",
    validate: (value) => {
      const trimmed = value.trim();
      if (!trimmed) return "文件名不能为空";
      if (/[\\/]/.test(trimmed)) return "文件名不能包含 \\ 或 /";
      if (trimmed === "." || trimmed === "..") return "文件名不能是 . 或 ..";
      return null;
    },
  });
  return name ? { action: "rename", name: name.trim() } : { action: "skip" };
}

/**
 * 目标目录里已存在的文件名集合。
 *
 * 入队前先读一次目录，右键上传/下载时同名文件能**当场**弹处理窗，
 * 不必等后端拒绝一次再问。读不到（权限/断线）返回 null：不假装"不存在"，
 * 交给入队时的 target_exists 兜底。
 */
async function existingNames(dir: string, remote: boolean): Promise<Set<string> | null> {
  if (!dir) return null;
  try {
    if (remote) {
      if (!props.sessionId) return null;
      const result = await browseRemoteDir(props.sessionId, dir);
      return new Set(result.entries.map((entry) => entry.name));
    }
    const items = await readDir(dir);
    return new Set(items.map((item) => item.name));
  } catch {
    return null;
  }
}

/** 按冲突策略调用入队；遇到 target_exists 弹一次对话框并按用户选择重试一次 */
async function enqueueWithConflict(
  targetLabel: string,
  run: (policy: ConflictPolicy) => Promise<unknown>,
): Promise<boolean> {
  try {
    await run("Fail");
    return true;
  } catch (error) {
    const kind = (error as { kind?: string })?.kind;
    if (kind !== "target_exists") throw error;
    const name = targetLabel.split("/").pop() ?? targetLabel;
    const choice = await askConflict(targetLabel, name);
    if (choice.action === "skip") return false;
    await run(choice.action === "overwrite" ? "Overwrite" : { Rename: choice.name });
    return true;
  }
}
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
  // 应用内选择器替代 tauri-plugin-dialog 的 open()：不再弹操作系统窗口
  const result = await pickLocalPath({
    mode: "directory",
    title: "选择本地文件夹",
    initialPath: internalLocalPath.value || localRoot.value || undefined,
    allowCreateDirectory: true,
  });
  if (!result) return;
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

onMounted(async () => {
  // 恢复上次打开的本地目录。capability 声明了全局 fs scope（`fs:scope` allow
  // `**`），任意目录都可读，无需再逐个授权；目录若已失效，错误由
  // FileBrowserPane 的错误行显示真实原因，用户可点「更换」重新选择。
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
  // 入队前先读一次远端目录：同名文件当场问覆盖/重命名/跳过，
  // 而不是先让后端拒绝一次再问（少一次往返，也不会先失败再解释）。
  const existing = await existingNames(internalRemotePath.value, true);
  const failures: string[] = [];
  let queued = 0;
  for (const file of files) {
    try {
      const remote = joinPath(internalRemotePath.value, file.name);
      if (existing?.has(file.name)) {
        const choice = await askConflict(`远端 ${remote}`, file.name);
        if (choice.action === "skip") continue;
        const policy: ConflictPolicy = choice.action === "overwrite" ? "Overwrite" : { Rename: choice.name };
        const target = choice.action === "rename" ? joinPath(internalRemotePath.value, choice.name) : remote;
        await enqueueUpload(joinPath(internalLocalPath.value, file.name), target, props.sessionId!, policy);
        queued += 1;
        continue;
      }
      const ok = await enqueueWithConflict(`远端 ${remote}`, (policy) =>
        enqueueUpload(joinPath(internalLocalPath.value, file.name), remote, props.sessionId!, policy),
      );
      if (ok) queued += 1;
    } catch (error) {
      failures.push(`${file.name}：${String(error)}`);
    }
  }
  if (queued > 0) {
    emit("upload-queued", queued);
    appendTransferLog("success", `上传入队 ${queued} 个文件 → ${internalRemotePath.value}`);
  }
  if (failures.length > 0) {
    appendTransferLog("error", `上传入队失败 ${failures.length} 个`, failures.join("；"));
    ElMessage.error(`上传失败 ${failures.length} 个：${failures.join("；")}`);
  }
}

async function download() {
  if (!capabilities.value.download || !props.sessionId) return;
  const files = transferable(selectedRemote.value);
  if (files.length === 0) return;
  // 同上传：先读本地目标目录，同名文件当场问覆盖/重命名/跳过
  const existing = await existingNames(internalLocalPath.value, false);
  const failures: string[] = [];
  let queued = 0;
  for (const file of files) {
    try {
      const local = joinPath(internalLocalPath.value, file.name);
      if (existing?.has(file.name)) {
        const choice = await askConflict(`本地 ${local}`, file.name);
        if (choice.action === "skip") continue;
        const policy: ConflictPolicy = choice.action === "overwrite" ? "Overwrite" : { Rename: choice.name };
        const target = choice.action === "rename" ? joinPath(internalLocalPath.value, choice.name) : local;
        await enqueueDownload(joinPath(internalRemotePath.value, file.name), target, props.sessionId!, policy);
        queued += 1;
        continue;
      }
      const ok = await enqueueWithConflict(`本地 ${local}`, (policy) =>
        enqueueDownload(joinPath(internalRemotePath.value, file.name), local, props.sessionId!, policy),
      );
      if (ok) queued += 1;
    } catch (error) {
      failures.push(`${file.name}：${String(error)}`);
    }
  }
  if (queued > 0) {
    emit("download-queued", queued);
    appendTransferLog("success", `下载入队 ${queued} 个文件 → ${internalLocalPath.value}`);
  }
  if (failures.length > 0) {
    appendTransferLog("error", `下载入队失败 ${failures.length} 个`, failures.join("；"));
    ElMessage.error(`下载失败 ${failures.length} 个：${failures.join("；")}`);
  }
}

async function createFolder() {
  if (!capabilities.value.createFolder || !props.sessionId) return;
  const value = await promptDialog({
    title: "新建远程文件夹",
    message: `将在 ${internalRemotePath.value} 下创建。`,
    placeholder: "文件夹名称",
    confirmText: "创建",
    validate: (input) => (safeName(input.trim()) ? null : "请输入不含 \\ / 的名称，且不能是 . 或 .."),
  });
  if (!value) return;
  const target = joinPath(internalRemotePath.value, value.trim());
  try {
    await createRemoteDirectory(props.sessionId, target);
    appendTransferLog("success", `新建远程文件夹：${target}`);
    await remotePane.value?.refresh();
  } catch (error) {
    appendTransferLog("error", "新建远程文件夹失败", String(error));
    ElMessage.error(`创建失败：${String(error)}`);
  }
}

async function deleteSelected() {
  if (!capabilities.value.delete || !props.sessionId) return;
  const file = selectedRemote.value[0];
  const target = joinPath(internalRemotePath.value, file.name);
  const ok = await confirmDialog({
    title: "删除远程文件",
    message: `确定删除远程文件「${file.name}」吗？此操作不可撤销。`,
    detail: target,
    confirmText: "删除",
    danger: true,
  });
  if (!ok) return;
  try {
    await deleteRemoteEntry(props.sessionId, target);
    appendTransferLog("success", `已删除远程文件：${target}`);
    selectedRemote.value = [];
    await remotePane.value?.refresh();
  } catch (error) {
    appendTransferLog("error", `删除远程文件失败：${target}`, String(error));
    ElMessage.error(`删除失败：${String(error)}`);
  }
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
      <FileBrowserPane
        ref="localPane"
        mode="local"
        :path="internalLocalPath"
        :root-path="localRoot"
        :root-label="localRoot"
        @choose-root="chooseLocalRoot"
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
