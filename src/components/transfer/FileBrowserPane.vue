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
import { DRAG_THRESHOLD, clickPending, dragState, hoverPane } from "../../utils/transferDrag";
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
  /** 本地模式：用户选定的本地根目录，用于在路径栏上显示「更换」入口 */
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
  /**
   * 拖放完成。`from` 是来源面板，`to` 是**目标**面板（投到了哪个面板上），
   * `entries` 是被拖动的条目。
   *
   * 抬起发生在来源面板（指针在来源面板按下），所以由来源面板发出这条请求。
   */
  (e: "drop-entries", payload: { from: "local" | "remote"; to: "local" | "remote"; entries: FsEntry[] }): void;
}>();

const entries = ref<FsEntry[]>([]);
const loading = ref(false);
const errorText = ref<string | null>(null);
/** 导航历史保留条数上限 */
const NAV_HISTORY_LIMIT = 100;
const history = ref<string[]>([]);
const historyIndex = ref(-1);
const search = ref("");
const selected = ref<Set<string>>(new Set());
/** Shift 区间选择的锚点行（最后一次非 Shift 点击的行） */
const anchorName = ref<string | null>(null);

const canBack = computed(() => historyIndex.value > 0);
const canForward = computed(() => historyIndex.value < history.value.length - 1);
const visibleEntries = computed(() => entries.value.filter((entry) => entry.name.toLowerCase().includes(search.value.toLowerCase())));

/**
 * 搜索词变化时清掉选中集。
 *
 * 多选可达之后这个问题才显出来：搜索过滤后，状态行仍显示「已选 N 项」，
 * 但其中多数已**看不见** —— 上传/下载/删除会作用于用户已经看不到的文件。
 * 看不见的选中项留着没意义，清掉最干净。
 */
watch(search, () => {
  if (selected.value.size === 0 && anchorName.value === null) return;
  selected.value = new Set();
  anchorName.value = null;
  emit("selection-change", []);
});

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
  // 拖拽后的 click 拦截器挂在**组件根元素**的 capture 阶段：行点击由
  // el-table 在目标阶段处理，冒泡阶段再拦就晚了；挂在根元素上既能覆盖
  // 面板内的行点击，又不会误伤应用其它区域。
  rootRef.value?.addEventListener("click", onWindowClickCapture, true);
  rootRef.value?.addEventListener("mousedown", onRootMouseDownCapture, true);
  // ResizeObserver 在 jsdom 里不存在，缺省时保持自适应高度即可
  if (typeof ResizeObserver === "undefined" || !listWrapRef.value) return;
  listObserver = new ResizeObserver(() => measureList());
  listObserver.observe(listWrapRef.value);
});

onBeforeUnmount(() => {
  listObserver?.disconnect();
  listObserver = null;
  // window 上的监听必须摘干净：组件在拖拽中途卸载时，指针监听残留会一直
  // 跟着已销毁的组件跑。
  rootRef.value?.removeEventListener("click", onWindowClickCapture, true);
  rootRef.value?.removeEventListener("mousedown", onRootMouseDownCapture, true);
  detachPointerListeners();
  dragPointer.value = null;
  dragging.value = false;
  dragState.value = null;
  hoverPane.value = null;
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
 * 把后端 / 系统的原始错误翻译成用户看得懂的中文说明。
 *
 * 只做**已知**错误的翻译（目录没了 / 没权限 / scope 拒绝）；其余错误原样透出，
 * 不猜、不吞掉真实原因。原始错误仍作为细节附在后面，便于排查。
 */
function describeLoadError(e: unknown, path: string): string {
  const raw = String(e);
  if (/forbidden path|not allowed on the scope|scope/i.test(raw)) {
    // capability 已是全局读写（`fs:read-all` + `**`），scope 不是这里的失败原因：
    // 真正的拒绝来自操作系统（权限不足 / 路径不存在 / 挂载点不可用）。
    // 旧的「尚未授权，请重新选择」建议会把人引向错误的操作。
    return `无权读取 ${path}：系统拒绝了该目录的访问（路径可能已失效，或当前账户没有读权限）。可点上方「更换本地目录」另选一个目录。`;
  }
  if (isEntryGone(e)) {
    return `目录 ${path} 不存在或已被移除。可点上方「更换本地目录」另选一个目录。`;
  }
  if (/permission denied|eacces|os error 13/i.test(raw)) {
    return `无权读取 ${path}：当前账户没有该目录的读权限。可点上方「更换本地目录」另选一个目录。`;
  }
  return raw;
}

async function load(path: string, pushHistory = true) {
  loading.value = true;
  errorText.value = null;
  selected.value = new Set();
  anchorName.value = null;
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
            // stat 跟随符号链接：readDir 的 isDirectory 把 macOS 的 /tmp、/var
            // 这类符号链接目录报成文件，图标与双击进入都会失准。详见 utils/localFs.ts。
            is_dir: details.isDirectory,
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
      // 导航历史留个上限：逐级浏览深层目录时它会一直涨，
      // 「后退」还能一路退回几百层之前的目录，实用性也一起下降
      if (history.value.length > NAV_HISTORY_LIMIT) {
        history.value = history.value.slice(-NAV_HISTORY_LIMIT);
      }
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

/**
 * 选中集更新。
 *
 * 修饰键决定语义（与系统文件管理器一致）：
 *   - 无修饰键：单选；若点的是唯一已选行则取消选择
 *   - Ctrl/⌘：在当前选中集上增删该行（多选）
 *   - Shift：从「锚点行」到该行做区间选择（区间按列表顺序，不是按已选集合）
 *
 * 之前 ctrlKey/shiftKey 两个形参恒为 false —— 调用点永远传 (false, false)，
 * 于是多选在界面上根本做不到，「批量上传/批量下载」这条路径不可达
 * （后端 enqueue_upload 是支持多文件的，前端却只能选中一个）。
 */
function toggleSelect(entry: FsEntry, ctrlKey: boolean, shiftKey: boolean) {
  if (shiftKey && anchorName.value !== null) {
    const list = visibleEntries.value;
    const from = list.findIndex((x) => x.name === anchorName.value);
    const to = list.findIndex((x) => x.name === entry.name);
    if (from >= 0 && to >= 0) {
      const [lo, hi] = from <= to ? [from, to] : [to, from];
      selected.value = new Set(list.slice(lo, hi + 1).map((x) => x.name));
      emitSelection();
      return;
    }
  }
  if (!ctrlKey && !shiftKey) {
    if (selected.value.size === 1 && selected.value.has(entry.name)) {
      selected.value = new Set();
    } else {
      selected.value = new Set([entry.name]);
    }
    anchorName.value = entry.name;
  } else if (selected.value.has(entry.name)) {
    selected.value.delete(entry.name);
    selected.value = new Set(selected.value);
    anchorName.value = entry.name;
  } else {
    selected.value.add(entry.name);
    selected.value = new Set(selected.value);
    anchorName.value = entry.name;
  }
  emitSelection();
}

/** 把当前选中集按可见顺序发给上层（上层据此决定上传/下载/删除的目标） */
function emitSelection() {
  emit(
    "selection-change",
    entries.value.filter((x) => selected.value.has(x.name)),
  );
}

function onRowClick(entry: FsEntry, event?: MouseEvent) {
  // 单击 = 选中(单击目录不会进入 —— 进入靠双击,避免误触)
  toggleSelect(entry, !!event?.ctrlKey || !!event?.metaKey, !!event?.shiftKey);
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
    // 锚点跟着选中走：否则下一次 Shift 点击的区间起点可能是一个已经不在
    // 选中集里的行，区间结果会出乎意料
    anchorName.value = entry.name;
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

// ── 拖拽 ──
// 本地 ⇄ 远程 面板之间互相拖：本地拖到远程 = 上传，远程拖到本地 = 下载。
//
// 用指针事件而不是 HTML5 drag & drop：macOS 的 WKWebView 里 HTML5 拖放
// 只会派发 dragstart / dragend，dragover / drop 根本不触发（真机合成手势
// 实测：事件序列只有 dragstart → dragend，缺 drag / dragover / drop），
// 于是投放永远不发生。同一套合成手势在 Chrome（Blink）里能完整派发，
// 说明是 WebKit 的原生拖拽会话不被合成事件驱动，不是页面代码的问题。
// 指针事件在 WKWebView 里行为正常，且能被自动化验证。
const dragPointer = ref<{ x: number; y: number; entries: FsEntry[] } | null>(null);
/** 拖拽进行中（指针已越过阈值） */
const dragging = ref(false);
/** 面板根元素：click 拦截器挂在这里 */
const rootRef = ref<HTMLElement | null>(null);
/**
 * 本面板是否正被当作拖拽目标高亮。
 *
 * 判定只用**共享状态**：拖拽是否在进行中、指针停在哪个面板、来源是哪个面板
 * —— 这三样都在 `utils/transferDrag` 里（两个面板是兄弟组件，实例级的
 * `dragging` 只有来源面板为 true，混进来会让目标面板永远不高亮）。
 */
const isDropTarget = computed(
  () => dragState.value !== null && hoverPane.value === props.mode && dragState.value.from !== props.mode,
);

/**
 * 按下：记住起点与要拖的条目（多选时带走整个选中集）。
 *
 * 监听挂在 window 上而不是面板上：拖拽一旦开始，指针会离开源面板，
 * 面板级监听就收不到后续的 pointermove / pointerup 了（拖到对面面板时
 * 必然发生），投放也就永远判不出来。
 */
/**
 * 从文件名单元格起手时取消浏览器默认的文本选择。
 *
 * 必须在 **capture 阶段**拦 mousedown：WebKit 在 mousedown 的默认动作里就把
 * 选区锚点定下来了，等到 pointermove 发现是拖拽再改 user-select 已经太晚 ——
 * 拖过去会拖出一条跨面板的文本高亮。
 *
 * 只针对文件名单元格（.name-cell）：那里的文本本来也**不能**选（user-select:none），
 * 取消默认动作不影响任何用户可见的选中行为。
 */
function onRootMouseDownCapture(event: MouseEvent) {
  if (event.button !== 0) return;
  const target = event.target as HTMLElement | null;
  if (!target?.closest(".name-cell")) return;
  event.preventDefault();
}

function onRowPointerDown(entry: FsEntry, event: PointerEvent) {
  if (event.button !== 0) return;
  // 拖拽期间禁掉本面板的文本选中：
  // pointerdown 落在 .name-cell（user-select:none）上，但 WebKit 在 mousedown
  // 时就把选区锚点定下来了，后续移动会把选区扩到面板的其它区域（表头、搜索框），
  // 拖过去会拖出一条跨区域的高亮。
  //
  // 整块面板临时设为不可选：pointerdown 落在 .name-cell（user-select:none）上，
  // 但 WebKit 在 mousedown 时就把选区锚点定下来了，后续移动会把选区扩到
  // 面板的其它区域（表头、搜索框），拖过去会拖出一条跨区域的高亮。
  // 这里不能 preventDefault —— 那会连带挡掉行点击（单击选中就失效了）。
  rootRef.value?.classList.add("is-dragging-select");
  const dragged = selected.value.has(entry.name)
    ? entries.value.filter((x) => selected.value.has(x.name))
    : [entry];
  dragPointer.value = { x: event.clientX, y: event.clientY, entries: dragged };
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp);
  window.addEventListener("pointercancel", onPointerCancel);
}

/** 移动：越过阈值才算拖拽，并记下指针停在哪个面板上 */
function onPointerMove(event: PointerEvent) {
  const start = dragPointer.value;
  if (!start) return;
  // 欧氏距离：曼哈顿距离下横向 3px + 纵向 3px 的正常手抖就会超阈值，
  // 一次普通点击被误判成拖拽 → 点击被吞、行不被选中。
  const moved = Math.hypot(event.clientX - start.x, event.clientY - start.y);
  if (!dragging.value) {
    if (moved < DRAG_THRESHOLD) return;
    dragging.value = true;
    // 越过阈值 = 确认是拖拽：清掉 mousedown 时可能已经建立的选区
    window.getSelection?.()?.removeAllRanges();
    dragState.value = { from: props.mode, names: start.entries.map((x) => x.name) };
  }
  hoverPane.value = paneUnderPointer(event.clientX, event.clientY);
}

/** 拆掉 window 上的拖拽监听（抬起 / 取消 / 卸载都要走这里） */
function detachPointerListeners() {
  window.removeEventListener("pointermove", onPointerMove);
  window.removeEventListener("pointerup", onPointerUp);
  window.removeEventListener("pointercancel", onPointerCancel);
  rootRef.value?.classList.remove("is-dragging-select");
}

/**
 * 指针抬起：落在另一个面板上就投放。
 *
 * 真的拖过之后，浏览器仍会在松手时派发 click —— 拖拽不该顺手改掉选中集，
 * 所以这里记一笔，由 capture 阶段的 click 拦截器吞掉那一次 click。
 */
function onPointerUp(event: PointerEvent) {
  const start = dragPointer.value;
  const wasDragging = dragging.value;
  dragPointer.value = null;
  dragging.value = false;
  detachPointerListeners();
  dragState.value = null;
  hoverPane.value = null;
  if (!start || !wasDragging) return;
  // 抬起后浏览器紧接着派发 click：这一次要吞掉（否则「拖一下」会顺手改掉选中集）。
  // 标志位下一 tick 就复位 —— 跨面板拖拽时 click 派发到两者的公共祖先，
  // 面板级拦截器看不到它，标志位若留着就会把面板内**下一个**点击也吞掉。
  clickPending.value = true;
  setTimeout(() => { clickPending.value = false; }, 0);
  const target = paneUnderPointer(event.clientX, event.clientY);
  // 拖回本面板（或拖到面板外）不做任何事
  if (!target || target === props.mode) return;
  // 抬起发生在**源面板**（指针在源面板按下），所以由源面板发出投放请求，
  // 并带上目标面板：上层据此决定是上传还是下载。
  emit("drop-entries", { from: props.mode, to: target, entries: start.entries });
}

/**
 * 手势被系统取消（pointercancel）：只清理，**不投放**。
 *
 * pointercancel 之后没有可靠的落点，拿它当 pointerup 会把文件投到
 * 一个用户根本没松开的位置。
 */
function onPointerCancel() {
  dragPointer.value = null;
  dragging.value = false;
  detachPointerListeners();
  dragState.value = null;
  hoverPane.value = null;
}

/**
 * 吞掉拖拽后紧随的那一次 click。
 *
 * 拖过之后松手，浏览器仍会派发 click（指针起止都在同一行时），不吞掉的话
 * 「拖一下」会顺带改掉选中集 —— 与用户意图不符。
 *
 * 只在事件**落在本面板内**时才吞：跨面板拖拽时 click 按 DOM 规范派发到
 * 两个面板的公共祖先（不在本面板内），那一次 click 本来就不该由本面板处理；
 * 若标志位留着，还会把面板内**下一个**点击也吞掉（用户看到「点了没反应」）。
 */
function onWindowClickCapture(event: MouseEvent) {
  if (!clickPending.value) return;
  const target = event.target as Node | null;
  if (!target || !rootRef.value?.contains(target)) return;
  clickPending.value = false;
  event.preventDefault();
  event.stopPropagation();
}

/**
 * 指针坐标下是哪个面板。
 *
 * 面板根节点带 data-pane="local|remote"：用 elementFromPoint 找到指针
 * 下面的元素，再向上找带该标记的祖先。
 */
function paneUnderPointer(x: number, y: number): "local" | "remote" | null {
  const element = document.elementFromPoint(x, y);
  const host = element?.closest("[data-pane]");
  const mode = host?.getAttribute("data-pane");
  return mode === "local" || mode === "remote" ? mode : null;
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
  <div ref="rootRef" class="pane">
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

    <!-- 列表：也是另一面板的拖放目标（本地 ⇄ 远程 互拖） -->
    <div
      ref="listWrapRef"
      class="list-wrap"
      :class="{ 'is-drop-target': isDropTarget }"
      data-test="fs-list"
      :data-pane="mode"
      @contextmenu="onPaneContextMenu"
    >
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
        @row-click="(row: FsEntry, _column: unknown, event: MouseEvent) => onRowClick(row, event)"
        @row-contextmenu="(row: FsEntry, _column: unknown, event: MouseEvent) => onRowContextMenu(row, event)"
      >
        <el-table-column prop="name" label="名称" min-width="220">
          <template #default="{ row }">
            <span
              class="name-cell"
              data-test="fs-row-name"
              @pointerdown="onRowPointerDown(row, $event)"
            >
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
  /* 文件名单元格可拖：拖到对面面板 = 上传 / 下载 */
  cursor: grab;
  touch-action: none;
  /* 拖拽时不要拉出文本选区（否则会跨面板拖出一条高亮） */
  user-select: none;
}
.icon { color: var(--rs-icon-folder); display: inline-flex; }
.icon.is-file { color: var(--rs-fg-muted); }
.filename.selected { color: var(--rs-fg); }

/* 拖拽进行中：禁掉本面板的文本选中 */
.pane.is-dragging-select {
  user-select: none;
}
.pane.is-dragging-select :deep(*) {
  user-select: none;
}

/* 拖放悬停时的投放提示 */
.list-wrap.is-drop-target {
  outline: 2px dashed var(--rs-accent);
  outline-offset: -2px;
}

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
