<script setup lang="ts">
/**
 * SessionList —— v2 重设计
 *
 * Xshell 风格左侧树状会话管理:
 *   - 分组(folder,纯组织)
 *   - 主机节点 + 状态点(复用签名元素)
 *   - 右键菜单:连接 / 断开 / 在新标签打开 / 打开 SFTP / 在此打开终端 / 重命名 / 删除
 *   - 拖拽节点(占位实现,后端未启拖拽 API 时不影响 UI)
 *
 * 数据源:目前 sessions store 的 items 是扁平 list;我们用一个前端派生算法
 * 按 'group' 字段聚合成树,后端将来接入分组 API 后只需替换 buildTree。
 */
import { computed, onBeforeUnmount, ref } from "vue";
import { useSessionsStore } from "../stores/sessions";
import type { Uuid } from "../ipc/types";
import type { SessionConfig } from "../ipc/types";
import SessionCredentialDialog from "./SessionCredentialDialog.vue";
import ConnectionRecoveryNotice from "./ConnectionRecoveryNotice.vue";
import { confirmDialog } from "../utils/dialog";

const emit = defineEmits<{
  (e: "select", id: Uuid): void;
  (e: "open-sftp", id: Uuid): void;
  (e: "open-terminal", id: Uuid, path: string): void;
  /** 为同一会话新开一个终端窗口（不抢占已有面板） */
  (e: "open-terminal-window", id: Uuid): void;
  /** 复制会话：用同一份连接信息建新条目 */
  (e: "duplicate-session", id: Uuid): void;
  (e: "new-session"): void;
}>();

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const store = useSessionsStore();
const credentialSession = ref<SessionConfig | null>(null);

interface TreeNode {
  id: string;
  label: string;
  type: "group" | "session";
  group?: string;
  children?: TreeNode[];
  session?: SessionConfig;
}

const groups = computed<TreeNode[]>(() => {
  const filtered = filteredSessions.value;
  const map = new Map<string, TreeNode>();
  for (const s of filtered) {
    const g = s.folder_id || "默认";
    if (!map.has(g)) {
      map.set(g, { id: `g:${g}`, label: g, type: "group", group: g, children: [] });
    }
    map.get(g)!.children!.push({
      id: s.id,
      label: s.name,
      type: "session",
      group: g,
      session: s,
    });
  }
  return Array.from(map.values());
});

const filteredSessions = computed(() => {
  const kw = store.searchKeyword.trim().toLowerCase();
  if (!kw) return store.items;
  return store.items.filter(
    (s) => s.name.toLowerCase().includes(kw) || s.host.toLowerCase().includes(kw),
  );
});

const stateClass = (id: Uuid) =>
  `rs-status-dot--${(store.connectionState.get(id) ?? "disconnected").toLowerCase()}`;

function onRowClick(s: SessionConfig) {
  emit("select", s.id);
}

const contextMenu = ref<{
  visible: boolean;
  x: number;
  y: number;
  session: SessionConfig | null;
}>({ visible: false, x: 0, y: 0, session: null });

/** 摘除 openContextMenu 挂上的兜底 window 监听（菜单项点击也要摘，不能等下一次外部点击） */
let detachOutsideClose: (() => void) | null = null;

function closeContextMenu() {
  contextMenu.value.visible = false;
  detachOutsideClose?.();
  detachOutsideClose = null;
}

function openContextMenu(s: SessionConfig, e: MouseEvent) {
  e.preventDefault();
  // 重复右键时先摘掉上一次的兜底监听，避免叠加
  closeContextMenu();
  contextMenu.value = {
    visible: true,
    x: e.clientX,
    y: e.clientY,
    session: s,
  };
  const close = () => closeContextMenu();
  // 延到本次事件派发结束后再挂：否则同一次右键自己就会把菜单关掉
  const timer = setTimeout(() => {
    window.addEventListener("click", close);
    window.addEventListener("contextmenu", close);
  }, 0);
  detachOutsideClose = () => {
    clearTimeout(timer);
    window.removeEventListener("click", close);
    window.removeEventListener("contextmenu", close);
  };
}

/**
 * 菜单项的统一入口：执行动作 + 关菜单 + 摘除兜底 window 监听。
 *
 * 菜单容器带 `@click.stop`，菜单项的点击不会再冒泡到 window，因此
 * 「靠 window 监听关闭」这条老路对菜单项**无效**。每个菜单项都必须走这里，
 * 否则新增项很容易再漏一次关闭（R3-14）。
 */
function runMenuItem(action: (session: SessionConfig) => void) {
  const target = contextMenu.value.session;
  if (!target) return;
  closeContextMenu();
  action(target);
}

function ctxConnect(s: SessionConfig) {
  emit("select", s.id);
}
function ctxOpenSftp(s: SessionConfig) {
  emit("open-sftp", s.id);
}
function ctxOpenTerminal(s: SessionConfig) {
  emit("open-terminal", s.id, "~");
}
/** 新开窗口：同连接信息可以同时开多个 shell，不复用已存在的终端面板 */
function ctxOpenTerminalWindow(s: SessionConfig) {
  emit("open-terminal-window", s.id);
}
function ctxDisconnect(s: SessionConfig) {
  store.disconnect(s.id).catch(console.warn);
}
function ctxUpdateCredential(s: SessionConfig) {
  credentialSession.value = s;
}

function openCredentialRecovery(id: Uuid) {
  const session = store.items.find((item) => item.id === id);
  if (session) credentialSession.value = session;
}

function retryConnection(id: Uuid) {
  void store.connect(id).catch(() => undefined);
}

// 复制流程由 App.vue 持有（入口有左侧列表与终端 tab 两处，而本组件按侧栏
// 当前面板条件挂载），这里只负责发出意图。
function ctxDuplicate(s: SessionConfig) {
  emit("duplicate-session", s.id);
}
async function ctxDelete(s: SessionConfig) {
  // 应用内确认窗替代 tauri-plugin-dialog 的 confirm()：不弹操作系统窗口
  const ok = await confirmDialog({
    title: "删除连接信息",
    message: `确定删除连接「${s.name}」吗？此操作不可撤销。`,
    detail: `${s.host}:${s.port}`,
    confirmText: "删除",
    danger: true,
  });
  if (!ok) return;
  // 失败原因由 sessions store 写入 store.error，列表顶部已有展示位
  await store.delete?.(s.id).catch(console.warn);
}

// 面板按侧栏当前面板条件卸载：兜底 window 监听不能比组件活得久
onBeforeUnmount(() => {
  detachOutsideClose?.();
  detachOutsideClose = null;
});
</script>

<template>
  <div class="session-tree">
    <div v-if="!props.embedded" class="tree-header">
      <h3>连接 ({{ store.items.length }})</h3>
      <div class="header-actions">
        <el-tooltip content="新建连接" placement="top">
          <button class="mini-btn" aria-label="新建连接" @click="emit('new-session')">
            <svg width="12" height="12" viewBox="0 0 16 16"><path d="M8 3 V13 M3 8 H13" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /></svg>
          </button>
        </el-tooltip>
        <el-tooltip content="刷新" placement="top">
          <button class="mini-btn" :class="{ 'is-loading': store.loading }" aria-label="刷新" @click="store.refresh()">
            <svg width="12" height="12" viewBox="0 0 16 16"><path d="M13 8 A5 5 0 1 1 11.5 4.2 M11.5 2.5 V5 H9" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" /></svg>
          </button>
        </el-tooltip>
      </div>
    </div>
    <el-input
      v-model="store.searchKeyword"
      size="small"
      placeholder="按名称 / 主机过滤"
      clearable
      class="search"
    />
    <p v-if="store.error" class="error">{{ store.error }}</p>
    <p class="model-hint">一条连接信息可同时开多个标签会话，每个标签是独立的会话实例。</p>
    <div v-if="store.loadIssues.length" class="error" role="alert">
      <p>部分连接加载失败。请检查配置和钥匙串访问权限后重试。</p>
      <p v-for="issue in store.loadIssues" :key="issue.session_id ?? 'storage'">
        {{ issue.session_id ?? '连接存储' }}: {{ issue.message }}
      </p>
      <button class="ctx-item" :disabled="store.retryingLoad" @click="store.retryLoad()">重试加载</button>
    </div>
    <SessionCredentialDialog v-if="credentialSession" :session="credentialSession" @close="credentialSession = null" />

    <div v-if="groups.length === 0" class="empty">
      <p>暂无连接</p>
    </div>

    <div class="tree-body">
      <div v-for="g in groups" :key="g.id" class="group">
        <div class="group-label">
          <svg class="caret" width="10" height="10" viewBox="0 0 16 16">
            <path d="M5 4 L11 8 L5 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
          <svg class="folder-icon" width="12" height="12" viewBox="0 0 16 16">
            <path d="M2 4.5 V12.5 H14 V6 H8 L6.5 4.5 Z" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" />
          </svg>
          <span>{{ g.label }}</span>
        </div>
        <ul class="children">
          <li
            v-for="node in g.children"
            :key="node.id"
            class="leaf"
            :class="{ 'is-active': store.currentId === node.session?.id }"
            @click="onRowClick(node.session!)"
            @contextmenu="openContextMenu(node.session!, $event)"
          >
            <span class="rs-status-dot" :class="stateClass(node.session!.id)" />
            <span class="leaf-name" :title="`${node.session!.host}:${node.session!.port}`">
              {{ node.label }}
            </span>
            <span class="leaf-host">{{ node.session!.host }}</span>
             <ConnectionRecoveryNotice
               v-if="store.connectionErrors.get(node.session!.id)"
               :session-id="node.session!.id"
               :kind="store.connectionErrors.get(node.session!.id)!.kind"
               :message="store.connectionErrors.get(node.session!.id)!.message"
               @update-credential="openCredentialRecovery"
               @retry-connect="retryConnection"
               @retry-load="store.retryLoad"
             />
          </li>
        </ul>
      </div>
    </div>

    <!-- 右键菜单 -->
    <div
      v-if="contextMenu.visible"
      class="ctx-menu"
      :style="{ left: `${contextMenu.x}px`, top: `${contextMenu.y}px` }"
      @click.stop
    >
      <button class="ctx-item" @click="runMenuItem(ctxConnect)">连接</button>
      <button class="ctx-item" @click="runMenuItem(ctxDisconnect)">断开</button>
      <button
        v-if="contextMenu.session?.protocol === 'SSH'"
        class="ctx-item"
        @click="runMenuItem(ctxUpdateCredential)"
      >
        更新凭据
      </button>
      <div class="ctx-sep" />
      <button
        v-if="contextMenu.session?.protocol === 'SSH'"
        class="ctx-item"
        @click="runMenuItem(ctxOpenSftp)"
      >
        打开 SFTP
      </button>
      <button class="ctx-item" @click="runMenuItem(ctxOpenTerminal)">打开标签</button>
      <button
        class="ctx-item"
        data-test="ctx-open-terminal-window"
        @click="runMenuItem(ctxOpenTerminalWindow)"
      >
        新开标签
      </button>
      <div class="ctx-sep" />
      <button
        class="ctx-item"
        data-test="ctx-duplicate"
        @click="runMenuItem(ctxDuplicate)"
      >
        复制连接信息
      </button>
      <button class="ctx-item ctx-danger" @click="runMenuItem(ctxDelete)">删除连接</button>
    </div>
  </div>
</template>

<style scoped>
.session-tree {
  display: flex;
  flex-direction: column;
  height: 100%;
  gap: var(--rs-s-2);
}
.tree-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
h3 {
  margin: 0;
  font-size: var(--rs-fs-lg);
  font-weight: 500;
  color: var(--rs-fg);
}
.header-actions { display: flex; gap: 2px; }
.mini-btn {
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
.mini-btn:hover { background: var(--rs-bg-surface-hover); color: var(--rs-fg); }
.mini-btn.is-loading svg { animation: spin 0.9s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }

.search { width: 100%; }

.error {
  color: var(--rs-p-danger);
  font-size: var(--rs-fs-xs);
  margin: 0;
}
.notice {
  color: var(--rs-fg-muted);
  font-size: var(--rs-fs-xs);
  margin: 0;
  padding: 4px var(--rs-s-2);
  border-left: 2px solid var(--rs-accent);
  background: var(--rs-bg-surface);
}
.empty { color: var(--rs-fg-disabled); font-size: var(--rs-fs-xs); padding: var(--rs-s-2); }
.model-hint {
  margin: 0;
  font-size: 11px;
  line-height: 1.5;
  color: var(--rs-fg-disabled);
}

.tree-body {
  flex: 1;
  overflow-y: auto;
  min-height: 0;
}

.group {
  margin-bottom: var(--rs-s-1);
}
.group-label {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px var(--rs-s-2);
  font-size: var(--rs-fs-xs);
  color: var(--rs-fg-muted);
  font-family: var(--rs-font-display);
  letter-spacing: 0.4px;
  text-transform: uppercase;
  cursor: pointer;
  user-select: none;
}
.group-label:hover { color: var(--rs-fg); }
.caret { transition: transform var(--rs-dur-fast) var(--rs-easing); }
.folder-icon { color: var(--rs-icon-folder); }

.children {
  list-style: none;
  margin: 0;
  padding: 0 0 0 var(--rs-s-3);
}
.leaf {
  display: grid;
  grid-template-columns: 12px 1fr auto;
  align-items: center;
  gap: 6px;
  padding: 4px var(--rs-s-2);
  font-size: var(--rs-fs-md);
  color: var(--rs-fg);
  border-radius: var(--rs-radius-1);
  cursor: pointer;
  user-select: none;
}
.leaf:hover { background: var(--rs-row-hover); }
.leaf.is-active { background: var(--rs-row-selected); }
.leaf > .connection-recovery {
  grid-column: 1 / -1;
}

.leaf-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.leaf-host {
  font-size: var(--rs-fs-xs);
  color: var(--rs-fg-disabled);
  font-family: var(--rs-font-mono);
  max-width: 70px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rs-status-dot {
  width: 8px;
  height: 8px;
}

.ctx-menu {
  position: fixed;
  background: var(--rs-bg-surface);
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-1);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
  padding: 4px;
  min-width: 140px;
  z-index: 1000;
}
.ctx-item {
  display: block;
  width: 100%;
  text-align: left;
  background: transparent;
  border: none;
  color: var(--rs-fg);
  padding: 6px var(--rs-s-3);
  font-family: var(--rs-font-ui);
  font-size: var(--rs-fs-sm);
  cursor: pointer;
  border-radius: var(--rs-radius-1);
}
.ctx-item:hover { background: var(--rs-row-selected); }
.ctx-item.ctx-danger { color: var(--rs-p-danger); }
.ctx-sep {
  height: 1px;
  background: var(--rs-border);
  margin: 4px 0;
}
</style>
