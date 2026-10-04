<script setup lang="ts">
/**
 * RShell 主布局 —— v2 重设计 (Xshell + Xftp 融合)
 *
 * 结构:
 *   ┌─────────────────────────────────────────────┐
 *   │ CustomTitleBar  (32px, 拖动 + 窗口控制)      │
 *   ├─────────────────────────────────────────────┤
 *   │ WorkspaceToolbar  (40px, switcher + 条件按钮)│
 *   ├──────────┬──────────────────────────────────┤
 *   │Activity  │ Workspace Content Area            │
 *   │Bar       │  - Terminal: Dockview 多终端      │
 *   │(48px)    │  - Transfer: 双窗格 + 传输面板    │
 *   ├──────────┴──────────────────────────────────┤
 *   │ StatusBar  (24px, 12 字段)                   │
 *   └─────────────────────────────────────────────┘
 *
 * 浮层:SessionCreateDialog / HostKeyMismatchDialog
 */
import { defineComponent, h, onBeforeUnmount, onMounted, ref, markRaw, computed, watch } from "vue";
import { DockviewVue } from "dockview-vue";
import type { ContextMenuItem, DockviewApi, DockviewReadyEvent } from "dockview-vue";
import { ElMessage } from "element-plus/es/components/message/index.mjs";
import { ElNotification } from "element-plus/es/components/notification/index.mjs";
import "dockview-vue/dist/styles/dockview.css";
import TerminalPane from "./components/TerminalPane.vue";
import TransferWorkspace from "./components/transfer/TransferWorkspace.vue";
import SessionCreateDialog from "./components/SessionCreateDialog.vue";
import SessionCredentialDialog from "./components/SessionCredentialDialog.vue";
import HostKeyMismatchDialog from "./components/HostKeyMismatchDialog.vue";
import CustomTitleBar from "./components/CustomTitleBar.vue";
import SidePanel, { type ToolSubview, type SettingsSubview } from "./components/SidePanel.vue";
import StatusBar from "./components/StatusBar.vue";
import DialogHost from "./components/dialogs/DialogHost.vue";
import PathPickerHost from "./components/dialogs/PathPickerHost.vue";
import WorkspaceToolbar, {
  type WorkspaceKind,
  type PanelKind,
} from "./components/WorkspaceToolbar.vue";
import TransferPanel, { type TransferItem } from "./components/TransferPanel.vue";
import { listTransfers, pauseTransfer, resumeTransfer, cancelTransfer, removeTransfer } from "./ipc/client";
import { subscribeAppEvents } from "./ipc/events";
import {
  DEFAULT_SIDEBAR_WIDTH,
  clampSidebarWidth,
  maxSidebarWidthForViewport,
} from "./utils/workspaceLayout";
import { toTransferItem } from "./utils/transferItem";
import { useTransferLog } from "./utils/transferLog";
import { shortTerminalTitle } from "./utils/terminalTitle";
import { newUuid } from "./utils/uuid";
import { useSessionsStore } from "./stores/sessions";
import { useHostKeyStore } from "./stores/hostKey";
import { useThemeStore } from "./stores/theme";
import type { Uuid, SessionConfig } from "./ipc/types";

const store = useSessionsStore();
const hostKeyStore = useHostKeyStore();
const themeStore = useThemeStore();
const dialogVisible = ref(false);
const activeTerminal = ref<Uuid | null>(null);
const activePanel = ref<PanelKind>("sessions");
const requestedToolSubview = ref<ToolSubview>("quick-commands");
const requestedSettingsSubview = ref<SettingsSubview>("theme");
const toolSubviewRequest = ref(0);
const settingsSubviewRequest = ref(0);
const panelExpanded = ref(true);

const workspace = ref<WorkspaceKind>("terminal");
const syncEnabled = ref(false);
const transferPanelExpanded = ref(true);
const activeTransferSession = ref<Uuid | null>(null);
const transferWorkspace = ref<InstanceType<typeof TransferWorkspace> | null>(null);
const transferCapabilities = ref({ upload: false, download: false, createFolder: false, delete: false, refresh: false, sync: false });
const transferItems = ref<TransferItem[]>([]);
/** 队列读取/订阅失败的提示；非空时 TransferPanel 就地展示（区别于"确实没有任务"） */
const transferLoadError = ref<string | null>(null);
/** 最近一次 pause/resume/cancel/remove 调用的错误；面板顶部红条展示，不静默 */
const transferActionError = ref<string | null>(null);
/** 当前正在调用传输控制命令的任务 ID，用于禁用按钮 */
const transferPendingIds = ref(new Set<string>());
/**
 * 传输面板「日志」tab 的数据源：真实事件流（入队 / 控制命令 / 冲突 / 失败 /
 * 完成），写入方是本文件与 TransferWorkspace，渲染方是 TransferPanel。
 * undefined 表示还没拖过，TransferPanel 用默认高度。
 */
const { entries: transferLogEntries, droppedCount: transferDroppedLogs, append: appendTransferLog, clear: clearTransferLogs } = useTransferLog();
const transferPanelHeight = ref<number | undefined>(undefined);
/**
 * 传输队列行可触发的控制动作。
 * - pause/resume/cancel 操作传输循环
 * - remove 只把终态条目移出队列，不删除已传输文件
 */
type TransferAction = "pause" | "resume" | "cancel" | "remove";
let unlistenTransfers: (() => void) | null = null;
let mounted = false;

async function refreshTransfers() {
  try {
    transferItems.value = (await listTransfers()).map(toTransferItem);
    transferLoadError.value = null;
  } catch (error) {
    // 不能静默：面板必须能区分「读取失败」与「确实没有任务」
    transferLoadError.value = String(error);
    appendTransferLog("error", "传输队列读取失败", String(error));
    console.error("无法读取传输队列", error);
  }
}

function actionLabel(action: TransferAction) {
  return { pause: "暂停", resume: "继续", cancel: "取消", remove: "删除" }[action];
}

/** 任务名用于日志文案：优先用队列里的名字，查不到（已移出队列）就退回 ID */
function transferName(taskId: string): string {
  return transferItems.value.find((row) => row.id === taskId)?.name ?? taskId;
}

async function runTransferAction(taskId: string, action: TransferAction) {
  if (transferPendingIds.value.has(taskId)) return;
  transferActionError.value = null;
  const next = new Set(transferPendingIds.value);
  next.add(taskId);
  transferPendingIds.value = next;
  try {
    if (action === "pause") {
      await pauseTransfer(taskId as Uuid);
    } else if (action === "resume") {
      await resumeTransfer(taskId as Uuid);
    } else if (action === "cancel") {
      await cancelTransfer(taskId as Uuid);
    } else {
      await removeTransfer(taskId as Uuid);
    }
    // 后端事件或队列刷新决定最终 phase；这里不写乐观状态。
    await refreshTransfers();
    appendTransferLog(
      action === "remove" ? "info" : "success",
      `${actionLabel(action)}：${transferName(taskId)}`,
    );
  } catch (error) {
    transferActionError.value = `${actionLabel(action)}失败：${String(error)}`;
    appendTransferLog("error", `${actionLabel(action)}失败：${transferName(taskId)}`, String(error));
    ElNotification.error({ title: `${actionLabel(action)}传输失败`, message: String(error) });
    console.error(`${actionLabel(action)}传输失败`, error);
  } finally {
    const remaining = new Set(transferPendingIds.value);
    remaining.delete(taskId);
    transferPendingIds.value = remaining;
  }
}

/**
 * 队列生命周期批量清理：逐条调用 RemoveTransfer。
 * 逐条而非新增批量命令，是为了让部分失败可见——批量接口吞掉单条错误会让
 * 用户以为队列已清空，实际仍有残留条目。终态守卫在服务端，
 * 活跃任务会被拒绝并计入失败。
 */
async function removeTransfers(taskIds: string[]) {
  if (taskIds.length === 0) return;
  transferActionError.value = null;
  const pending = new Set(transferPendingIds.value);
  for (const id of taskIds) pending.add(id);
  transferPendingIds.value = pending;

  const failures: string[] = [];
  try {
    for (const id of taskIds) {
      try {
        await removeTransfer(id as Uuid);
      } catch (error) {
        failures.push(String(error));
      }
    }
    await refreshTransfers();
    if (failures.length > 0) {
      transferActionError.value = `删除失败 ${failures.length} 个：${failures[0]}`;
      appendTransferLog("error", `批量删除未完成：${failures.length} 个条目未删除`, failures.join("；"));
      ElNotification.error({
        title: "队列清理未完成",
        message: `${failures.length} 个条目未能删除（进行中的任务需先取消）。`,
      });
    } else {
      appendTransferLog("success", `已从队列移除 ${taskIds.length} 个已结束条目`);
    }
  } finally {
    const remaining = new Set(transferPendingIds.value);
    for (const id of taskIds) remaining.delete(id);
    transferPendingIds.value = remaining;
  }
}

const sidebarWidth = ref(DEFAULT_SIDEBAR_WIDTH);
const viewportWidth = ref(
  typeof window === "undefined" ? 1280 : window.innerWidth,
);

const sidebarMaxWidth = computed(() =>
  maxSidebarWidthForViewport(viewportWidth.value),
);

/**
 * dockview 面板壳：dockview-vue 不渲染默认/具名插槽，面板组件由
 * api.addPanel 按 components 注册表创建，且面板数据装在单个 `params`
 * prop 里（结构 { params, api, containerApi }）。这里解出 sessionId
 * 转发给 TerminalPane，保持 TerminalPane 的 sessionId prop 契约不变。
 */
const TerminalPanelView = defineComponent({
  name: "TerminalPanelView",
  props: { params: { type: Object, default: undefined } },
  setup(panelProps) {
    return () => {
      const panelParams = panelProps.params as { params?: { sessionId?: Uuid; terminalId?: Uuid } } | undefined;
      const sessionId = panelParams?.params?.sessionId;
      if (!sessionId) return null;
      // terminalId 决定这个标签用哪个 pty：主 pty 由连接创建，附加标签各开各的。
      // 缺省 undefined = 主 pty。
      const terminalId = panelParams?.params?.terminalId;
      // 连接状态从 store 现取：终端面板要跟着真实连接变化（断开/重连），
      // 不能只在挂载时读一次。render 函数里读 store 天然是响应式的。
      const connectionState = store.connectionState.get(sessionId) ?? "disconnected";
      return h(TerminalPane, { sessionId, terminalId, connectionState });
    };
  },
});

// as never 与原 TerminalPane 注册同款：DefineComponent 全泛型签名和
// dockview-vue 的 VueComponent 别名不完全兼容
const components = markRaw({ terminal: TerminalPanelView as never });

/** dockview 容器 ready 后的 api；终端面板只能经该 api 创建 */
let dockviewApi: DockviewApi | null = null;

const TERMINAL_PANEL_PREFIX = "terminal-";
/** 同会话多开窗口时面板 id 的序号分隔符；UUID 只有十六进制与连字符，不会撞上 */
const PANEL_INDEX_SEPARATOR = "~";

/** 面板 id ⇄ 会话：第 1 个窗口不带后缀，第 n 个带 `~n` */
function terminalPanelId(sessionId: Uuid, index: number): string {
  return index <= 1
    ? `${TERMINAL_PANEL_PREFIX}${sessionId}`
    : `${TERMINAL_PANEL_PREFIX}${sessionId}${PANEL_INDEX_SEPARATOR}${index}`;
}

function parseTerminalPanelId(panelId: string): { sessionId: Uuid; index: number } | null {
  if (!panelId.startsWith(TERMINAL_PANEL_PREFIX)) return null;
  const rest = panelId.slice(TERMINAL_PANEL_PREFIX.length);
  const sep = rest.indexOf(PANEL_INDEX_SEPARATOR);
  if (sep < 0) return { sessionId: rest as Uuid, index: 1 };
  const sessionId = rest.slice(0, sep) as Uuid;
  const index = Number.parseInt(rest.slice(sep + 1), 10);
  if (!sessionId || !Number.isInteger(index) || index < 1) return null;
  return { sessionId, index };
}

function terminalPanelTitle(sessionId: Uuid, position: number): string {
  const name = store.items.find((s) => s.id === sessionId)?.name;
  const base = shortTerminalTitle(name, sessionId);
  return position <= 1 ? base : `${base} #${position}`;
}

/** 新窗口序号 = 现存最大序号 + 1；用最大值而非个数，避免中间窗口被关掉后撞 id */
function nextTerminalPanelIndex(sessionId: Uuid): number {
  let max = 0;
  for (const panel of dockviewApi?.panels ?? []) {
    const info = parseTerminalPanelId(panel.id);
    if (info && info.sessionId === sessionId) max = Math.max(max, info.index);
  }
  return max + 1;
}

/**
 * 按创建顺序给某会话的窗口重新编号：主窗口不带后缀，附加窗口显示 `#2`、
 * `#3`… 关掉中间一个窗口后编号自动补齐，不会出现 `#2 #4` 这种跳号。
 */
function retitleTerminalPanels(sessionId: Uuid) {
  if (!dockviewApi) return;
  const owned = dockviewApi.panels
    .map((panel) => ({ panel, info: parseTerminalPanelId(panel.id) }))
    .filter((entry) => !!entry.info && entry.info!.sessionId === sessionId)
    .sort((a, b) => a.info!.index - b.info!.index);
  owned.forEach((entry, i) => entry.panel.api.setTitle(terminalPanelTitle(sessionId, i + 1)));
}

/**
 * 确保该会话有终端面板。`forceNew` 为真时**总是**新开一个窗口——
 * 同一份连接信息可以同时开多个 shell，各窗口是各自独立的 attach 通道。
 */
function ensureTerminalPanel(sessionId: Uuid, opts: { forceNew?: boolean } = {}) {
  if (!dockviewApi) return;
  if (!opts.forceNew) {
    const main = dockviewApi.getPanel(terminalPanelId(sessionId, 1));
    if (main) {
      main.api.setTitle(terminalPanelTitle(sessionId, 1));
      main.api.setActive();
      retitleTerminalPanels(sessionId);
      return;
    }
  }
  const index = nextTerminalPanelIndex(sessionId);
  const panelId = terminalPanelId(sessionId, index);
  // 序号取自现存最大值，理论上不会撞；真撞上就复用而不是 addPanel 报错
  const clash = dockviewApi.getPanel(panelId);
  if (clash) {
    clash.api.setActive();
    return;
  }
  dockviewApi.addPanel({
    id: panelId,
    component: "terminal",
    title: terminalPanelTitle(sessionId, index),
    // terminalId 决定这个标签用哪个 pty。主 pty 以 session_id 寻址（首标签），
    // 附加标签各带一个自己的 id，挂载时后端为它单独开 pty = 独立的 shell 进程。
    params: {
      sessionId,
      terminalId: index <= 1 ? sessionId : newUuid(),
    },
  });
  retitleTerminalPanels(sessionId);
}

function onDockviewReady(event: DockviewReadyEvent) {
  dockviewApi = event.api;
  if (activeTerminal.value) ensureTerminalPanel(activeTerminal.value);
  // 聚焦窗口决定工具栏的连接/断开作用在哪条会话上：同一会话的多个窗口
  // 共享连接状态，但跨会话必须跟着焦点走。
  event.api.onDidActivePanelChange(({ panel }) => {
    if (!panel) return;
    const info = parseTerminalPanelId(panel.id);
    if (!info) return;
    activeTerminal.value = info.sessionId;
    store.currentId = info.sessionId;
  });
  event.api.onDidRemovePanel(({ id }) => {
    const info = parseTerminalPanelId(id);
    if (info) retitleTerminalPanels(info.sessionId);
  });
}

// 容器 v-if 卸载时 DockviewVue 已 dispose 其 api，清引用防误用
function onDockviewUnmounted() {
  dockviewApi = null;
}

// 面板的创建/激活由两条路径保证：selectSession 直接调 ensureTerminalPanel
//（覆盖「面板被关闭后重击同一会话」——watch 因 Object.is 相等不会触发），
// 容器首次挂载则由 onDockviewReady 兜底。activeTerminal 仅在 selectSession
// 写入，不再需要针对创建/激活的 watch。

// R2-12：会话被删除后，其 terminal-{id} 面板必须同步关闭——否则残留的
// 面板指向已删除会话，键入只会触发 IO 失败提示。会话列表在删除路径
// （store.deleteSessionById → SessionListChanged → store.refresh）后更新，
// 这里对 items 做差集，把已消失会话的面板关掉。
function closeOrphanTerminalPanels() {
  if (!dockviewApi) return;
  const alive = new Set(store.items.map((session) => session.id));
  for (const panel of dockviewApi.panels) {
    const info = parseTerminalPanelId(panel.id);
    if (!info) continue;
    if (!alive.has(info.sessionId)) dockviewApi.getPanel(panel.id)?.api.close();
    else retitleTerminalPanels(info.sessionId);
  }
}

watch(
  () => store.items.map((session) => session.id),
  () => closeOrphanTerminalPanels(),
);

function setSidebarWidth(width: number) {
  sidebarWidth.value = clampSidebarWidth(width, sidebarMaxWidth.value);
}

function onViewportResize() {
  viewportWidth.value = window.innerWidth;
  sidebarWidth.value = clampSidebarWidth(
    sidebarWidth.value,
    sidebarMaxWidth.value,
  );
}

function selectPanel(panel: PanelKind) {
  activePanel.value = panel;
  panelExpanded.value = true;
}

function toggleSidebar(expanded?: boolean) {
  panelExpanded.value = expanded ?? !panelExpanded.value;
}

async function selectSession(id: Uuid) {
  activeTerminal.value = id;
  store.currentId = id;
  // 重复点击同一会话不会触发 watch(activeTerminal)（Object.is 相等），
  // 面板被用户关闭后必须在这里直接重建。容器尚未挂载时 dockviewApi 为
  // null，此调用是 no-op，由 onDockviewReady 兜底建面板。
  ensureTerminalPanel(id);
  if (store.connectionState.get(id) === "connected" || store.connectionState.get(id) === "connecting") return;
  try {
    await store.connect(id);
  } catch (e) {
    ElMessage.error(`连接失败：${String(e)}`);
  }
}

function openNewSession() {
  dialogVisible.value = true;
}

function openPanel(name: PanelKind) {
  activePanel.value = name;
  panelExpanded.value = true;
}

function openToolSubview(name: ToolSubview) {
  requestedToolSubview.value = name;
  toolSubviewRequest.value++;
  openPanel("tools");
}

function openSettingsSubview(name: SettingsSubview) {
  requestedSettingsSubview.value = name;
  settingsSubviewRequest.value++;
  openPanel("settings");
}

function pickWorkspace(w: WorkspaceKind) {
  workspace.value = w;
  if (w === "transfer" && !activeTransferSession.value && store.currentId) {
    activeTransferSession.value = store.currentId;
  }
}

function onOpenSftp(id: Uuid) {
  workspace.value = "transfer";
  activeTransferSession.value = id;
  void selectSession(id);
}

function onOpenTerminal(_id: Uuid, _path: string) {
  workspace.value = "terminal";
  void selectSession(_id);
}

/**
 * 为连接信息新开一个标签会话。
 *
 * 概念（见 docs/08）：左侧「连接」是连接信息（主机/端口/用户/认证），
 * 终端标签是**独立的会话实例**——同一条连接可以同时开多个标签，
 * 每个标签各挂自己的 attach 通道与终端状态，关闭其中一个不影响其余。
 *
 * 连接未建立时先握手再建标签；已连接则直接复用，不重复握手。
 */
async function openTabSession(id: Uuid) {
  workspace.value = "terminal";
  activeTerminal.value = id;
  store.currentId = id;
  if (store.connectionState.get(id) !== "connected" && store.connectionState.get(id) !== "connecting") {
    try {
      await store.connect(id);
    } catch (e) {
      ElMessage.error(`连接失败：${String(e)}`);
      return;
    }
  }
  ensureTerminalPanel(id, { forceNew: true });
}

/**
 * 在**当前连接信息**下新开一个标签。
 *
 * 与「复制标签」的区别：复制标签用右键所在那个标签的连接，新建标签用
 * 当前聚焦标签的连接——右键非聚焦标签时两者结果不同。
 */
function newTabFromActive(): void {
  const id = activeTerminal.value;
  if (!id) {
    ElMessage.info("先选中一个标签会话，再新建标签。");
    return;
  }
  void openTabSession(id);
}

// ── 复制会话 ──
// 流程放在 App.vue 而不是 SessionList：入口有两个（左侧会话列表右键、终端
// tab 右键），而 SessionList 是按侧栏当前面板条件挂载的（切到「文件/工具」
// 就卸载），挂在那儿监听 tab 触发的复制会静默失灵。
/** 复制后需要补录凭据的会话；非空时渲染凭据对话框 */
const credentialForDuplicate = ref<SessionConfig | null>(null);
const duplicatingSession = ref(false);

/** 口令类认证且新条目还没有凭据时，才需要立刻补录 */
function duplicateNeedsCredential(session: SessionConfig): boolean {
  const auth = session.auth_method;
  if ("Password" in auth) return !auth.Password.has_password;
  if ("KeyboardInteractive" in auth) return !auth.KeyboardInteractive.has_password;
  return false;
}

async function duplicateSession(id: Uuid) {
  if (duplicatingSession.value) return;
  duplicatingSession.value = true;
  try {
    const newId = await store.duplicate(id);
    const created = store.items.find((item) => item.id === newId) ?? null;
    if (created && duplicateNeedsCredential(created)) {
      credentialForDuplicate.value = created;
      ElNotification.info({
        title: "已复制连接信息",
        message: `「${created.name}」的连接信息与原条目相同；凭据不随配置复制，请先填写凭据再连接。`,
      });
    } else {
      ElNotification.info({
        title: "已复制连接信息",
        message: `「${created?.name ?? "新连接"}」的连接信息与原条目相同；凭据未复制，如需要请在连接列表右键「更新凭据」。`,
      });
    }
    // 不自动选中/连接：新条目此刻没有凭据，自动连接只会换来一次注定失败的
    // 认证与一个错误弹窗。连不连由用户点击决定。
  } catch (error) {
    ElNotification.error({ title: "复制连接信息失败", message: String(error) });
  } finally {
    duplicatingSession.value = false;
  }
}

/**
 * 终端标签的右键菜单。
 *
 * 用 dockview 的 `getTabContextMenuItems`：省略该选项时 dockview 根本不弹菜单，
 * 返回空数组则对该面板抑制。菜单项执行后 dockview 会自动收起菜单。
 * 「关闭标签」自己实现而不用内置 `'close'`，因为内置项文案是英文 Close。
 *
 * 三项都以**连接信息**为操作对象，但结果不同：
 *   新建标签 —— 当前聚焦标签的连接，再开一个独立会话实例
 *   复制标签 —— 右键所在标签的连接，再开一个独立会话实例
 *   关闭标签 —— 只关这个标签，同连接的其他标签不受影响
 */
function terminalTabContextMenuItems(params: {
  panel: { id: string; api: { close: () => void } };
}): ContextMenuItem[] {
  const info = parseTerminalPanelId(params.panel.id);
  if (!info) return [];
  return [
    { label: "新建标签", action: newTabFromActive },
    { label: "复制标签", action: () => void openTabSession(info.sessionId) },
    "separator",
    { label: "关闭标签", action: () => params.panel.api.close() },
  ];
}

async function connectCurrent() {
  if (!store.currentId) return;
  try { await store.connect(store.currentId); }
  catch (error) { ElMessage.error(`连接失败：${String(error)}`); }
}

async function disconnectCurrent() {
  if (!store.currentId) return;
  try { await store.disconnect(store.currentId); }
  catch (error) { ElMessage.error(`断开失败：${String(error)}`); }
}

type TerminalAction = "find" | "clear" | "closeFind" | "copy" | "paste";

function terminalAction(action: TerminalAction) {
  if (!activeTerminal.value) return;
  window.dispatchEvent(new CustomEvent("rshell:terminal-action", { detail: { sessionId: activeTerminal.value, action } }));
}

// R2-13：window 级快捷键只路由到当前激活终端。以前每个 TerminalPane 各自挂
// window keydown，多面板并存时一次 Ctrl+F 会同时切换所有面板的搜索栏；
// 现在统一在这里拦截，经 rshell:terminal-action（面板内按 sessionId 过滤）分发。
// 复制粘贴用 Ctrl+Shift+C / Ctrl+Shift+V（Xterm.js 惯例）：裸 Ctrl+C 在终端里
// 是 SIGINT，Ctrl+V 是 quoted-insert，都不能劫持成剪贴板操作。
function onGlobalKeydown(e: KeyboardEvent) {
  const accel = e.ctrlKey || e.metaKey;
  if (accel && e.shiftKey && e.key.toLowerCase() === "c") {
    e.preventDefault();
    terminalAction("copy");
  } else if (accel && e.shiftKey && e.key.toLowerCase() === "v") {
    e.preventDefault();
    terminalAction("paste");
  } else if (accel && e.key.toLowerCase() === "f") {
    e.preventDefault();
    terminalAction("find");
  } else if (e.key === "Escape") {
    terminalAction("closeFind");
  }
}

const currentConnectionState = computed(() => {
  const id = activeTerminal.value;
  if (!id) return "disconnected";
  return store.connectionState.get(id) ?? "disconnected";
});

onMounted(async () => {
  mounted = true;
  window.addEventListener("resize", onViewportResize);
  window.addEventListener("keydown", onGlobalKeydown);
  await store.refresh();
  if (!mounted) return;
  await store.subscribeEvents();
  if (!mounted) return;
  await hostKeyStore.subscribeEvents();
  if (!mounted) return;
  await themeStore.refresh();
  if (!mounted) return;
  await themeStore.subscribeEvents();
  if (!mounted) return;
  await refreshTransfers();
  if (!mounted) return;
  try {
    const stop = await subscribeAppEvents((event) => {
      if (event === "TransferQueueChanged" || (typeof event === "object" && event !== null && (
        "TransferCompleted" in event || "TransferFailed" in event || "TransferProgress" in event
      ))) void refreshTransfers();
      // 终态与冲突进日志。TransferProgress 不进：每次采样都写一条会把日志刷爆，
      // 进度本来就由队列列表实时呈现。
      if (typeof event !== "string" && "TransferCompleted" in event) {
        appendTransferLog("success", `传输完成：${transferName(event.TransferCompleted.task_id)}`);
      }
      if (typeof event !== "string" && "TransferFailed" in event) {
        appendTransferLog("error", `传输失败：${transferName(event.TransferFailed.task_id)}`, event.TransferFailed.error);
      }
      if (typeof event !== "string" && "TransferConflict" in event) {
        appendTransferLog("warn", `目标已存在：${event.TransferConflict.path}`);
      }
      if (typeof event !== "string" && "TriggerFired" in event && event.TriggerFired.action_summary.startsWith("notify: ")) {
        ElNotification({ title: "触发器通知", message: event.TriggerFired.action_summary.slice(8) });
      }
      if (typeof event !== "string" && "TriggerActionFailed" in event) {
        ElNotification.error({ title: "触发器执行失败", message: event.TriggerActionFailed.error });
      }
      if (typeof event !== "string" && "ComposeSendFailed" in event) {
        ElNotification.error({
          title: "撰写发送失败",
          message: `会话 ${event.ComposeSendFailed.session_id}：${event.ComposeSendFailed.error}`,
        });
      }
    });
    if (mounted) unlistenTransfers = stop;
    else stop();
  } catch (error) {
    console.error("无法订阅传输队列", error);
    transferLoadError.value = String(error);
    appendTransferLog("error", "传输事件订阅失败", String(error));
    ElNotification.error({
      title: "传输事件订阅失败",
      message: "传输进度与结果将不再自动刷新，请重启应用。",
    });
  }
});

onBeforeUnmount(() => {
  mounted = false;
  window.removeEventListener("resize", onViewportResize);
  window.removeEventListener("keydown", onGlobalKeydown);
  dockviewApi = null;
  unlistenTransfers?.();
  store.disposeEvents();
  hostKeyStore.disposeEvents();
  themeStore.disposeEvents();
});
</script>

<template>
  <div class="rshell-shell">
    <CustomTitleBar
      @new-session="openNewSession"
      @toggle-sidebar="toggleSidebar"
      @open-key-manager="openPanel('keys')"
      @open-theme-panel="openSettingsSubview('theme')"
      @open-plugin-panel="openSettingsSubview('plugins')"
      @open-transfer-queue="pickWorkspace('transfer'); transferPanelExpanded = true"
      @open-quick-commands="openToolSubview('quick-commands')"
      @open-triggers="openToolSubview('triggers')"
      @open-tunnels="openToolSubview('tunnels')"
    />

    <WorkspaceToolbar
      :workspace="workspace"
      :connection-state="currentConnectionState"
      :terminal-available="!!activeTerminal"
      :active-panel="activePanel"
      :sidebar-expanded="panelExpanded"
      :sync-enabled="syncEnabled"
      :transfer-panel-expanded="transferPanelExpanded"
      :can-upload="transferCapabilities.upload"
      :can-download="transferCapabilities.download"
      :can-create-folder="transferCapabilities.createFolder"
      :can-delete="transferCapabilities.delete"
      :can-refresh-files="transferCapabilities.refresh"
      :can-sync-files="transferCapabilities.sync"
      :on-new-session="openNewSession"
      :on-connect="connectCurrent"
      :on-disconnect="disconnectCurrent"
      :on-find="() => terminalAction('find')"
      :on-clear-screen="() => terminalAction('clear')"
      :on-new-tab="newTabFromActive"
      @change-workspace="pickWorkspace"
      @select-panel="selectPanel"
      @toggle-sidebar="toggleSidebar"
      @sync-toggle="syncEnabled = !syncEnabled"
      @upload="transferWorkspace?.upload()"
      @download="transferWorkspace?.download()"
      @new-folder="transferWorkspace?.createFolder()"
      @delete="transferWorkspace?.deleteSelected()"
      @refresh="transferWorkspace?.refresh()"
      @toggle-transfer-panel="transferPanelExpanded = !transferPanelExpanded"
    />

    <div class="body">
      <SidePanel
        :active="activePanel"
        :tool-subview="requestedToolSubview"
        :settings-subview="requestedSettingsSubview"
        :tool-subview-request="toolSubviewRequest"
        :settings-subview-request="settingsSubviewRequest"
        :active-session-id="store.currentId"
        :active-session-connected="!!store.currentId && store.connectionState.get(store.currentId) === 'connected'"
        :width="sidebarWidth"
        :max-width="sidebarMaxWidth"
        :expanded="panelExpanded"
        @update:width="setSidebarWidth"
        @select-session="selectSession"
        @new-session="openNewSession"
        @open-sftp="onOpenSftp"
        @open-terminal="onOpenTerminal"
        @open-terminal-window="openTabSession"
        @duplicate-session="duplicateSession"
      />

      <main class="main-area">
        <!-- Terminal Workspace -->
        <div v-show="workspace === 'terminal'" class="workspace-layer terminal-layer">
          <DockviewVue
            v-if="activeTerminal"
            :components="components"
            :get-tab-context-menu-items="terminalTabContextMenuItems"
            style="width: 100%; height: 100%"
            @ready="onDockviewReady"
            @vue:unmounted="onDockviewUnmounted"
          />
          <div v-else class="empty">
            <div class="empty-content">
              <div class="empty-icon">⌬</div>
              <h2>RShell</h2>
              <p>从左侧「连接」面板新建或选择一个 SSH 连接</p>
              <el-button type="primary" @click="openNewSession">新建连接</el-button>
            </div>
          </div>
        </div>

        <!-- Transfer Workspace -->
        <div v-show="workspace === 'transfer'" class="workspace-layer transfer-layer">
          <div class="transfer-area">
            <TransferWorkspace
              v-if="activeTransferSession"
              ref="transferWorkspace"
              :session-id="activeTransferSession"
              :connected="store.connectionState.get(activeTransferSession) === 'connected'"
              :sync-enabled="syncEnabled"
              @capabilities="transferCapabilities = $event"
              @upload-queued="refreshTransfers"
              @download-queued="refreshTransfers"
              style="flex: 1; min-height: 0"
            />
            <div v-else class="empty">
              <div class="empty-content">
                <div class="empty-icon">⇄</div>
                <h2>传输工作区</h2>
                <p>从左侧「连接」右键 → 打开 SFTP，或先连接一个连接信息</p>
                <el-button type="primary" :disabled="!activeTerminal" @click="onOpenSftp(activeTerminal!)">
                  使用当前连接
                </el-button>
              </div>
            </div>
            <TransferPanel
              :expanded="transferPanelExpanded"
              :items="transferItems"
              :error="transferLoadError"
              :action-error="transferActionError"
              :pending-task-ids="transferPendingIds"
              :height="transferPanelHeight"
              :logs="transferLogEntries"
              :dropped-logs="transferDroppedLogs"
              @toggle="transferPanelExpanded = !transferPanelExpanded"
              @pause="(taskId) => runTransferAction(taskId, 'pause')"
              @resume="(taskId) => runTransferAction(taskId, 'resume')"
              @cancel="(taskId) => runTransferAction(taskId, 'cancel')"
              @remove="(taskId) => runTransferAction(taskId, 'remove')"
              @remove-many="removeTransfers"
              @update:height="transferPanelHeight = $event"
              @clear-log="clearTransferLogs"
            />
          </div>
        </div>
      </main>
    </div>

    <StatusBar :workspace="workspace" />

    <SessionCreateDialog
      :visible="dialogVisible"
      @close="dialogVisible = false"
      @created="(id) => selectSession(id)"
    />
    <!-- 复制会话后为新条目补录凭据（凭据不随配置复制） -->
    <SessionCredentialDialog
      v-if="credentialForDuplicate"
      :session="credentialForDuplicate"
      @close="credentialForDuplicate = null"
    />
    <HostKeyMismatchDialog />
    <!-- 全局自定义弹窗：确认/输入窗体与本地路径选择器都在应用内渲染，
         不再出现 Element Plus 的 MessageBox 或操作系统原生窗口 -->
    <DialogHost />
    <PathPickerHost />
  </div>
</template>

<style scoped>
.rshell-shell {
  display: flex;
  flex-direction: column;
  width: 100vw;
  height: 100vh;
  margin: 0;
  padding: 0;
  background: var(--rs-bg);
  color: var(--rs-fg);
  font-family: var(--rs-font-ui);
  overflow: hidden;
  box-sizing: border-box;
}

.body {
  flex: 1 1 0;
  display: flex;
  min-height: 0;
  min-width: 0;
  margin: 0;
  padding: 0;
  overflow: hidden;
  box-sizing: border-box;
}

.main-area {
  flex: 1 1 0;
  background: var(--rs-bg);
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  margin: 0;
  padding: 0;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
}

.transfer-area {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  min-height: 0;
}

.workspace-layer { width: 100%; height: 100%; min-width: 0; min-height: 0; }
.terminal-layer,
.transfer-layer { display: flex; flex-direction: column; }

.empty {
  display: flex;
  align-items: center;
  justify-content: center;
  flex: 1;
  margin: 0;
  padding: 0;
}
.empty-content {
  text-align: center;
  color: var(--rs-fg-muted);
}
.empty-icon {
  font-size: 64px;
  color: var(--rs-accent);
  opacity: 0.6;
  margin-bottom: var(--rs-s-4);
}
.empty-content h2 {
  margin: 0 0 var(--rs-s-2);
  font-size: var(--rs-fs-2xl);
  font-weight: 500;
  color: var(--rs-fg);
}
.empty-content p {
  margin: 0 0 var(--rs-s-4);
  font-size: var(--rs-fs-md);
}
</style>
