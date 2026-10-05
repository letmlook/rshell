<script setup lang="ts">
/**
 * PathBar —— 文件面板路径栏（面包屑 + 自定义输入 + 目录树下拉）
 *
 * 三种导航入口共用一条状态：
 *   1. 面包屑：点任一段跳到对应绝对路径（保留原有交互）
 *   2. 铅笔按钮：就地变成输入框，用户手输绝对路径，Enter 提交 / Esc 取消
 *   3. 树形按钮：下拉目录树，逐层懒加载，点节点即导航；打开时自动把当前
 *      路径所在的那条链展开好，用户不必从根一路点下来
 *
 * 目录树手写而不用 el-tree：这个仓库按组件粒度拆包（见 src/element-plus.ts，
 * 超过 500 KiB 会触发 scripts/check-build-output.mjs 阈值），为一个面板引入
 * 整棵 el-tree 不划算；这里只需要「可展开的目录列表」，扁平渲染即可。
 *
 * 加载失败的目录必须显示真实原因（节点内联错误行），不能静默变成空目录——
 * 空目录与「无权访问」在界面上必须可区分。
 */
import { computed, nextTick, onBeforeUnmount, ref, shallowRef, watch } from "vue";
import { readDir } from "@tauri-apps/plugin-fs";
import { browseRemoteDir } from "../../ipc/client";
import { isWithinRoot } from "../../utils/rootBoundary";
import type { Uuid } from "../../ipc/types";

const props = defineProps<{
  mode: "local" | "remote";
  /** 远程必填；本地忽略 */
  sessionId?: Uuid;
  path: string;
  /** 本地模式已授权的根目录：树以它为根，且输入不得越界 */
  rootPath?: string;
}>();

const emit = defineEmits<{ (e: "navigate", path: string): void }>();

/** 目录树的一个节点（只含子目录，不含文件） */
interface DirNode {
  label: string;
  path: string;
  depth: number;
  expanded: boolean;
  /** 子目录是否已成功加载过；false 表示展开时还要去读一次 */
  loaded: boolean;
  loading: boolean;
  error: string | null;
  children: DirNode[];
}

const editing = ref(false);
const draftPath = ref("");
const inputError = ref<string | null>(null);
const pathInputRef = ref<{ focus?: () => void } | null>(null);

const treeOpen = ref(false);
const rootNode = shallowRef<DirNode | null>(null);
/** 当前这棵树是为哪条路径展开的；换路径后需要重建 */
const treeForPath = ref<string | null>(null);
const treeError = ref<string | null>(null);
const treeWrapRef = ref<HTMLElement | null>(null);
const treeInputRef = ref<HTMLInputElement | null>(null);
const treeFilter = ref("");

/**
 * 树根：本地有显式 rootPath 时以它为根（文件面板不能越界）；
 * 没给 rootPath 时（路径选择器）退到当前路径所在卷/文件系统根，
 * 这样树与手工输入都覆盖整台机器，不被某个授权根卡住。
 */
const treeRoot = computed(() => {
  if (props.mode === "remote") return "/";
  if (props.rootPath) return props.rootPath;
  return /^[A-Za-z]:[\\/]/.test(props.path) ? props.path.slice(0, 3) : "/";
});
const canUseTree = computed(() => !!treeRoot.value);

const isWinPath = computed(() => props.path.includes("\\") || /^[A-Za-z]:/.test(props.path));

/** 面包屑分段：本地保留盘符（`D:\data` 不能按 `/` 切开），点一段导航到其绝对路径 */
const breadcrumb = computed(() => {
  if (!props.path) return [];
  const sep = isWinPath.value ? "\\" : "/";
  const parts = props.path.split(sep).filter(Boolean);
  if (isWinPath.value && /^[A-Za-z]:$/.test(parts[0] ?? "")) {
    const drive = parts.shift() as string;
    return [
      { label: `${drive}${sep}`, path: `${drive}${sep}` },
      ...parts.map((seg, i) => ({
        label: seg,
        path: `${drive}${sep}${parts.slice(0, i + 1).join(sep)}`,
      })),
    ];
  }
  return parts.map((seg, i) => ({ label: seg, path: `/${parts.slice(0, i + 1).join("/")}` }));
});

/** 相对树根的路径分段；不在树根之下（本地越界）返回空数组 */
function relativeSegments(root: string, target: string): string[] {
  if (!root || !target) return [];
  const sep = isWinPath.value ? "\\" : "/";
  const normalize = (p: string) => (sep === "/" ? p : p.replace(/\//g, "\\")).replace(/[\\/]+$/, "");
  const nRoot = normalize(root);
  const nTarget = normalize(target);
  if (nTarget === nRoot) return [];
  if (!nTarget.startsWith(nRoot + sep)) return [];
  return nTarget.slice(nRoot.length + 1).split(sep).filter(Boolean);
}

function makeNode(label: string, path: string, depth: number): DirNode {
  return { label, path, depth, expanded: false, loaded: false, loading: false, error: null, children: [] };
}

/**
 * 树的失效信号。
 *
 * 树是**普通对象**而不是 reactive()：扁平渲染的 computed 只在求值时读到
 * 哪一层，深层节点后续的 `expanded = true` 就没人订阅，展开后界面不动
 * （表现为「树打不开第二层」）。所有写操作统一 bump 版本号，让渲染重跑，
 * 行为与树的深度无关。
 */
const treeVersion = ref(0);
function touch() {
  treeVersion.value += 1;
}

/** 把底层错误改写成用户可行动的说法；不吞掉失败，也不假装目录是空的 */
function describeListError(e: unknown, path: string): string {
  const raw = String(e);
  if (/forbidden path|not allowed on the scope|scope/i.test(raw)) {
    return `无权读取 ${path}：该目录尚未授权。`;
  }
  if (/请先连接/.test(raw)) return raw;
  return `${path}：${raw}`;
}

async function loadSubDirs(dir: string): Promise<DirNode[]> {
  if (props.mode === "remote") {
    if (!props.sessionId) throw new Error("请先连接 SSH 会话");
    const r = await browseRemoteDir(props.sessionId, dir);
    return r.entries
      .filter((entry) => entry.file_type === "Directory")
      .map((entry) => makeNode(entry.name, joinPath(dir, entry.name), 1));
  }
  const items = await readDir(dir);
  return items
    .filter((item) => item.isDirectory)
    .map((item) => makeNode(item.name, joinPath(dir, item.name), 1));
}

function joinPath(base: string, name: string): string {
  const sep = base.includes("\\") && !base.includes("/") ? "\\" : "/";
  const head = base.replace(/[\\/]+$/, "");
  return `${head}${sep}${name}`;
}

async function expandNode(node: DirNode): Promise<void> {
  if (node.expanded || node.loading) return;
  node.error = null;
  touch();
  if (!node.loaded) {
    node.loading = true;
    touch();
    try {
      const kids = await loadSubDirs(node.path);
      for (const kid of kids) kid.depth = node.depth + 1;
      node.children = kids;
      node.loaded = true;
    } catch (e) {
      node.error = describeListError(e, node.path);
    } finally {
      node.loading = false;
    }
  }
  node.expanded = true;
  touch();
}

function collapseNode(node: DirNode) {
  node.expanded = false;
  touch();
}

/**
 * 打开下拉时把「树根 → 当前路径」这条链自动加载并展开。
 * 逐层读目录（深度 = 路径段数），比一次列全树便宜；某一层读不到（权限/
 * 路径不存在/符号链接未解析）就停在能到的最深处，并把原因挂在该节点上。
 */
async function buildChain(): Promise<void> {
  const root = treeRoot.value;
  treeError.value = null;
  if (!root) {
    treeError.value = "尚未选择根目录";
    rootNode.value = null;
    return;
  }
  const rootLabel = root === "/" ? "/" : root.replace(/[\\/]+$/, "");
  const node = makeNode(rootLabel, root, 0);
  node.expanded = true;
  rootNode.value = node;
  touch();
  /** 读一层子目录；失败原因挂在节点上，不抛出到链式展开之外 */
  const ensureLoaded = async (target: DirNode) => {
    if (target.loaded || target.loading) return;
    target.loading = true;
    touch();
    try {
      const kids = await loadSubDirs(target.path);
      for (const kid of kids) kid.depth = target.depth + 1;
      target.children = kids;
      target.loaded = true;
    } catch (e) {
      target.error = describeListError(e, target.path);
    } finally {
      target.loading = false;
      touch();
    }
  };
  try {
    if (props.mode === "remote" && !props.sessionId) {
      node.error = "请先连接 SSH 会话";
      touch();
      return;
    }
    let current = node;
    for (const seg of relativeSegments(root, props.path)) {
      await ensureLoaded(current);
      if (current.error) return;
      const next = current.children.find((kid) => kid.label === seg);
      if (!next) {
        // 当前路径不在可见子目录里：停在最深处，由用户自行选择，不静默跳走
        current.error = `${props.path} 不在 ${current.path} 的子目录列表中`;
        touch();
        return;
      }
      // 链上每一层都要展开，否则用户只看到根 + 第一层，还得自己一层层点
      current.expanded = true;
      current = next;
    }
    // 让当前目录本身也可展开（用户常在树里继续往下点）
    await ensureLoaded(current);
    if (!current.error) {
      current.expanded = true;
      touch();
    }
  } catch (e) {
    treeError.value = describeListError(e, props.path);
  } finally {
    treeForPath.value = props.path;
  }
}

async function toggleTree() {
  if (treeOpen.value) {
    closeTree();
    return;
  }
  treeOpen.value = true;
  if (treeForPath.value !== props.path || !rootNode.value) await buildChain();
  treeFilter.value = "";
  await nextTick();
  treeInputRef.value?.focus();
}

function closeTree() {
  treeOpen.value = false;
  treeFilter.value = "";
}

/**
 * 当前要渲染的节点（扁平列表，靠 depth 做缩进）。
 *
 * 无筛选：只渲染已展开分支。
 * 有筛选：命中的节点连同其祖先链全部渲染，且不再受展开态限制——否则
 * 深层目录命中时看不到它挂在哪条路径下。
 */
const visibleNodes = computed<DirNode[]>(() => {
  // 先建立版本依赖：树是普通对象，只有版本号变化才会让这里重算
  void treeVersion.value;
  const root = rootNode.value;
  if (!root) return [];
  const filter = treeFilter.value.trim().toLowerCase();
  if (!filter) {
    const out: DirNode[] = [];
    const walk = (node: DirNode) => {
      out.push(node);
      if (node.expanded) node.children.forEach(walk);
    };
    walk(root);
    return out;
  }
  const shown = new Set<DirNode>();
  const mark = (node: DirNode, ancestorMatched: boolean) => {
    const matched = ancestorMatched || node.label.toLowerCase().includes(filter);
    if (matched) shown.add(node);
    node.children.forEach((kid) => mark(kid, matched));
  };
  mark(root, false);
  const out: DirNode[] = [];
  const walk = (node: DirNode) => {
    if (!shown.has(node)) return;
    out.push(node);
    node.children.forEach(walk);
  };
  walk(root);
  return out;
});

/** 可见节点上的加载错误；集中展示在列表下方，避免每行都插错误文本挤掉目录名 */
const nodeErrors = computed(() => visibleNodes.value.map((n) => n.error).filter((t): t is string => !!t));

function isCurrent(node: DirNode): boolean {
  return node.path === props.path;
}

function onNodeClick(node: DirNode) {
  emit("navigate", node.path);
  closeTree();
}

// ── 自定义输入 ──
async function startEdit() {
  draftPath.value = props.path;
  inputError.value = null;
  editing.value = true;
  await nextTick();
  pathInputRef.value?.focus?.();
}

function cancelEdit() {
  editing.value = false;
  draftPath.value = "";
  inputError.value = null;
}

function isAbsolute(candidate: string): boolean {
  return candidate.startsWith("/") || candidate.startsWith("\\") || /^[A-Za-z]:[\\/]?/.test(candidate);
}

function commitEdit() {
  const target = draftPath.value.trim();
  if (!target) {
    inputError.value = "路径不能为空";
    return;
  }
  if (!isAbsolute(target)) {
    inputError.value = "请输入绝对路径（如 /var/log 或 D:\\data）";
    return;
  }
  // 本地只允许在已授权根目录内导航：越界直接说明原因，不静默丢弃输入。
  // 判定用共享的 isWithinRoot（词法消解 `.` / `..` 后逐段比较），
  // 与 FileBrowserPane.navigateTo 同一份实现，两个入口不会对 `..` 或
  // Windows 根给出相反结论。
  if (props.mode === "local" && props.rootPath) {
    if (!isWithinRoot(target, props.rootPath)) {
      inputError.value = `路径超出已授权的根目录 ${props.rootPath}`;
      return;
    }
  }
  cancelEdit();
  if (target !== props.path) emit("navigate", target);
}

// ── 点击外部关闭 ──
function onDocumentMouseDown(e: MouseEvent) {
  if (!treeOpen.value) return;
  const wrap = treeWrapRef.value;
  if (wrap && e.target instanceof Node && wrap.contains(e.target)) return;
  closeTree();
}

function onDocumentKeydown(e: KeyboardEvent) {
  if (e.key === "Escape" && treeOpen.value) {
    e.stopPropagation();
    closeTree();
  }
}

watch(treeOpen, (open) => {
  if (typeof document === "undefined") return;
  if (open) {
    document.addEventListener("mousedown", onDocumentMouseDown, true);
    document.addEventListener("keydown", onDocumentKeydown, true);
  } else {
    document.removeEventListener("mousedown", onDocumentMouseDown, true);
    document.removeEventListener("keydown", onDocumentKeydown, true);
  }
});

onBeforeUnmount(() => {
  if (typeof document === "undefined") return;
  document.removeEventListener("mousedown", onDocumentMouseDown, true);
  document.removeEventListener("keydown", onDocumentKeydown, true);
});
</script>

<template>
  <div class="path-bar" data-test="fs-path-bar">
    <!-- 非编辑态：面包屑 + 两个入口按钮 -->
    <template v-if="!editing">
      <div class="breadcrumb" :title="path || '（未选择目录）'">
        <span v-if="!path" class="seg is-empty">（未选择目录）</span>
        <span
          v-for="(seg, i) in breadcrumb"
          :key="i"
          class="seg"
          :class="{ 'is-current': i === breadcrumb.length - 1 }"
          :data-test="`fs-crumb-${i}`"
          @click="emit('navigate', seg.path)"
        >
          {{ seg.label }}
        </span>
      </div>
      <button
        class="path-btn"
        data-test="fs-path-edit"
        title="输入路径"
        aria-label="输入路径"
        @click="startEdit"
      >
        <svg width="12" height="12" viewBox="0 0 16 16">
          <path d="M11.5 2.5 L13.5 4.5 L5.5 12.5 L2.5 13.5 L3.5 10.5 Z" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" />
        </svg>
      </button>
      <button
        class="path-btn"
        data-test="fs-path-tree-toggle"
        title="目录树"
        aria-label="目录树"
        aria-haspopup="tree"
        :aria-expanded="treeOpen"
        :disabled="!canUseTree"
        @click="toggleTree"
      >
        <svg width="12" height="12" viewBox="0 0 16 16">
          <path d="M2 4 H6 L7.2 5.5 H14 V12.5 H2 Z" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" />
        </svg>
      </button>
    </template>

    <!-- 编辑态：整条路径栏变成输入框 -->
    <template v-else>
      <el-input
        ref="pathInputRef"
        v-model="draftPath"
        size="small"
        class="path-input"
        data-test="fs-path-input"
        placeholder="输入绝对路径后回车"
        @keyup.enter="commitEdit"
        @keyup.esc="cancelEdit"
        @blur="cancelEdit"
      />
      <button class="path-btn" title="确认" aria-label="确认路径" data-test="fs-path-commit" @click="commitEdit">
        <svg width="12" height="12" viewBox="0 0 16 16">
          <path d="M3 8.5 L6.5 12 L13 4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </button>
      <button class="path-btn" title="取消" aria-label="取消编辑" data-test="fs-path-cancel" @click="cancelEdit">
        <svg width="12" height="12" viewBox="0 0 16 16">
          <path d="M4 4 L12 12 M12 4 L4 12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
        </svg>
      </button>
    </template>

    <p v-if="inputError" class="path-error" role="alert" data-test="fs-path-error">{{ inputError }}</p>

    <!-- 目录树下拉 -->
    <div v-if="treeOpen" ref="treeWrapRef" class="tree-pop" data-test="fs-path-tree" role="tree">
      <div class="tree-head">
        <input
          ref="treeInputRef"
          v-model="treeFilter"
          class="tree-filter"
          data-test="fs-path-tree-filter"
          placeholder="筛选目录名"
        />
        <button class="path-btn" title="关闭" aria-label="关闭目录树" data-test="fs-path-tree-close" @click="closeTree">
          <svg width="12" height="12" viewBox="0 0 16 16">
            <path d="M4 4 L12 12 M12 4 L4 12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
          </svg>
        </button>
      </div>
      <p v-if="treeError" class="tree-error" role="alert" data-test="fs-path-tree-error">{{ treeError }}</p>
      <ul class="tree-list" role="group">
        <li v-if="visibleNodes.length === 0" class="tree-empty" data-test="fs-path-tree-empty">
          {{ treeFilter ? "没有匹配的目录" : "没有子目录" }}
        </li>
        <li
          v-for="(node, i) in visibleNodes"
          :key="node.path"
          class="tree-row"
          :class="{ 'is-current': isCurrent(node) }"
          role="treeitem"
          :aria-selected="isCurrent(node)"
          :data-test="`fs-path-tree-node-${i}`"
          :data-path="node.path"
          :style="{ paddingLeft: `${6 + node.depth * 12}px` }"
        >
          <button
            class="tree-toggle"
            :aria-label="node.expanded ? '折叠' : '展开'"
            :data-test="`fs-path-tree-toggle-${i}`"
            @click="node.expanded ? collapseNode(node) : expandNode(node)"
          >
            <span v-if="node.loading">…</span>
            <span v-else>{{ node.expanded ? "▾" : "▸" }}</span>
          </button>
          <button class="tree-label" :title="node.path" :data-test="'fs-path-tree-label-' + i" @click="onNodeClick(node)">
            {{ node.label }}
          </button>
        </li>
      </ul>
      <p v-if="nodeErrors.length" class="tree-error" role="alert">
        <span v-for="(text, i) in nodeErrors" :key="i" class="tree-error-line">{{ text }}</span>
      </p>
    </div>
  </div>
</template>

<style scoped>
.path-bar {
  position: relative;
  display: flex;
  align-items: center;
  gap: var(--rs-s-1);
  flex: 1;
  min-width: 0;
}

.breadcrumb {
  flex: 1;
  display: flex;
  align-items: center;
  min-width: 0;
  font-family: var(--rs-font-mono);
  font-size: var(--rs-fs-xs);
  color: var(--rs-fg-muted);
  overflow-x: auto;
  white-space: nowrap;
  scrollbar-width: thin;
  /* 路径过长时折叠到中间，保留根与末段可读性 */
  mask-image: linear-gradient(to right, transparent 0, #000 12px, #000 calc(100% - 12px), transparent 100%);
}
.seg {
  cursor: pointer;
  padding: 0 4px;
}
.seg:hover { color: var(--rs-fg); }
.seg.is-current { color: var(--rs-fg); }
.seg.is-empty { cursor: default; opacity: 0.7; }
.seg + .seg::before {
  content: "/";
  margin-right: 4px;
  opacity: 0.5;
}

.path-btn {
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
.path-btn:hover:not(:disabled) {
  background: var(--rs-bg-surface-hover);
  color: var(--rs-fg);
}
.path-btn:disabled { opacity: 0.4; cursor: not-allowed; }

.path-input { flex: 1; min-width: 0; }

.path-error {
  position: absolute;
  top: calc(100% + 2px);
  left: 0;
  right: 0;
  margin: 0;
  padding: 2px 6px;
  font-size: var(--rs-fs-xs);
  color: var(--rs-p-danger);
  background: var(--rs-bg-surface);
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-1);
  z-index: 20;
}

.tree-pop {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  z-index: 2200;
  width: min(420px, 90vw);
  max-height: 320px;
  display: flex;
  flex-direction: column;
  background: var(--rs-bg-surface);
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-1);
  box-shadow: 0 6px 18px rgb(0 0 0 / 32%);
  overflow: hidden;
}
.tree-head {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 6px;
  border-bottom: 1px solid var(--rs-border);
}
.tree-filter {
  flex: 1;
  min-width: 0;
  height: 22px;
  padding: 0 6px;
  font-size: var(--rs-fs-xs);
  font-family: var(--rs-font-ui);
  color: var(--rs-fg);
  background: var(--rs-bg-panel);
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-1);
}
.tree-filter:focus { outline: 1px solid var(--rs-accent); }

.tree-list {
  list-style: none;
  margin: 0;
  padding: 4px 0;
  overflow: auto;
  min-height: 0;
  flex: 1;
}
.tree-row {
  display: flex;
  align-items: center;
  gap: 2px;
}
.tree-row.is-current { background: var(--rs-row-selected); }
.tree-row:hover { background: var(--rs-row-hover); }
.tree-toggle {
  width: 18px;
  height: 20px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  color: var(--rs-fg-muted);
  font-size: 10px;
  cursor: pointer;
}
.tree-label {
  flex: 1;
  min-width: 0;
  text-align: left;
  padding: 2px 6px 2px 0;
  background: transparent;
  border: none;
  color: var(--rs-fg);
  font-family: var(--rs-font-mono);
  font-size: var(--rs-fs-xs);
  cursor: pointer;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tree-empty,
.tree-error {
  margin: 0;
  padding: 6px 8px;
  font-size: var(--rs-fs-xs);
  color: var(--rs-fg-muted);
}
.tree-error { color: var(--rs-p-danger); }
.tree-error-line { display: block; }
</style>
