<script setup lang="ts">
/**
 * TerminalPane —— 终端面板（xterm 全权渲染）
 *
 * 设计 §4.3 流程 A:挂载 xterm → invoke('attach_terminal', { session_id, on_data: ch })
 * （IPC 参数键契约:后端命令统一 rename_all = "snake_case",键名与 Rust 形参一致）
 * → flush 积压 → 转 Attached。term.onData → invoke('send_input')。
 *
 * 面板自带三组能力:
 *   1. 连接状态:跟随后端 ConnectionStateChanged 渲染「已连接/连接中/已断开」。
 *      断开时禁用输入并给出可重连入口（PROB-13 的错误条仍保留给 attach 失败）。
 *   2. 剪贴板:选中即复制（可配置、持久化）+ 右键直接粘贴 + 有选区时右键菜单
 *      （复制/粘贴/清屏）+ Ctrl 系快捷键。
 *   3. 终端配色:工具栏下拉切换后端已安装的配色方案（SetTerminalColorScheme）。
 *
 * 状态来源全部是后端真实状态：连接状态取 sessions store，方案列表取
 * ListThemes 的 available_schemes，切换走真实命令。没有任何本地猜测的
 * 「已连接」展示。
 *
 * onContextLoss:WebGL addon 在某些环境下会触发 context loss,自动 fallback 到 canvas
 * (设计 §9 #3 + 切片 1.3 完成判据)。
 */
import { computed, onMounted, onBeforeUnmount, ref, watch } from "vue";
import { Terminal } from "@xterm/xterm";
import type { ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { SearchAddon } from "@xterm/addon-search";
import { Channel } from "@tauri-apps/api/core";
import { invoke } from "@tauri-apps/api/core";
import { ElMessage } from "element-plus/es/components/message/index.mjs";
import { ipcErrorMessage, sendInput, resizeTerminal } from "../ipc/client";
import type { Uuid } from "../ipc/types";
import { useThemeStore } from "../stores/theme";
import { useSessionsStore } from "../stores/sessions";
import {
  defaultLocalStorage,
  loadCopyOnSelect,
  saveCopyOnSelect,
} from "../utils/terminalPrefs";

const themeStore = useThemeStore();
const sessionsStore = useSessionsStore();

const props = withDefaults(
  defineProps<{
    sessionId?: Uuid;
    /**
     * 会话连接状态,由 App.vue 从 sessions store 现取后传入。
     * 缺省按 disconnected 处理:没有真实连接就不该让用户以为能输入。
     */
    connectionState?: "disconnected" | "connecting" | "connected" | "failed";
  }>(),
  { connectionState: "disconnected" },
);

const containerRef = ref<HTMLDivElement | null>(null);
const searchBarVisible = ref(false);
const searchTerm = ref("");
/** attach_terminal 失败信息；非空时面板内渲染错误状态条（PROB-13，不再空白终端） */
const attachError = ref<string | null>(null);
let term: Terminal | null = null;
let fit: FitAddon | null = null;
let search: SearchAddon | null = null;
let channel: Channel<number[]> | null = null;
let detachSizeObserver: (() => void) | null = null;

// ── 连接状态 ──
/** 本面板是否曾经连上过：只有「连过又掉」才提示断开，避免新建面板时闪红条 */
const everConnected = ref(false);
watch(
  () => props.connectionState,
  (state) => {
    if (state === "connected") everConnected.value = true;
  },
  { immediate: true },
);

const isConnected = computed(() => props.connectionState === "connected");
/** 连接掉线（曾经连过、现在不是 connected）才叫「断开」 */
const isDropped = computed(() => everConnected.value && props.connectionState !== "connected");

const CONNECTION_LABEL: Record<string, string> = {
  connected: "已连接",
  connecting: "连接中",
  disconnected: "未连接",
  failed: "连接失败",
};
const connectionLabel = computed(() => CONNECTION_LABEL[props.connectionState] ?? props.connectionState);

const reconnecting = ref(false);

/** 真实重连:走 sessions store 的 connect（后端握手 + 事件广播），失败如实提示 */
async function reconnect() {
  if (!props.sessionId || reconnecting.value) return;
  reconnecting.value = true;
  try {
    await sessionsStore.connect(props.sessionId);
  } catch (error) {
    ElMessage.error(`重连失败：${String(error)}`);
  } finally {
    reconnecting.value = false;
  }
}

/** 断开态禁用输入:没有连接时把按键丢在本地,不如明确告知用户 */
watch(
  isConnected,
  (connected) => {
    if (term) term.options.disableStdin = !connected;
  },
  { immediate: false },
);

// ── 剪贴板:选中即复制 ──
const copyOnSelect = ref(loadCopyOnSelect(defaultLocalStorage()));
const copyOnSelectTimer = ref<ReturnType<typeof setTimeout> | null>(null);
let copyFailureNotified = false;

function onCopyOnSelectToggle(next: boolean | string | number) {
  const value = next === true || next === "true";
  copyOnSelect.value = value;
  // 存储失败（隐私模式/配额满）不阻断本次会话内的行为，只是不记住
  if (!saveCopyOnSelect(defaultLocalStorage(), value)) {
    console.warn("[TerminalPane] 选中即复制偏好未能持久化");
  }
}

/** 复制/粘贴需要剪贴板权限；无选中内容或剪贴板不可用时必须可见提示，不静默吞掉 */
function noteClipboardFailure(action: "复制" | "粘贴", error: unknown) {
  ElMessage.warning(`${action}失败：${String(error)}`);
  console.error(`${action}失败`, error);
}

async function writeClipboard(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch (e) {
    noteClipboardFailure("复制", e);
    return false;
  }
}

async function copySelection() {
  if (!term) return;
  const text = term.getSelection();
  if (!text) {
    ElMessage.info("终端没有选中内容");
    return;
  }
  await writeClipboard(text);
}

async function pasteClipboard() {
  if (!term) return;
  let text: string;
  try {
    text = await navigator.clipboard.readText();
  } catch (e) {
    noteClipboardFailure("粘贴", e);
    return;
  }
  if (!text) {
    ElMessage.info("剪贴板为空");
    return;
  }
  // 未连接时粘贴没有去处：明确告知，不假装送达
  if (!isConnected.value) {
    ElMessage.warning("会话未连接，无法粘贴。请先重连会话。");
    return;
  }
  // paste() 走 term.onData 通路，与键盘输入同等对待（含后端 IO 失败提示）
  term.paste(text);
}

/**
 * 选区状态用 ref 而非 computed：xterm 的选区不在 Vue 响应式系统里，
 * computed 不会因拖选而重算，右键菜单的「复制」可用态会一直是初始值。
 * 由 onSelectionChange 显式同步。
 */
const hasSelection = ref(false);

function syncSelectionState() {
  hasSelection.value = !!term?.getSelection();
}

/**
 * 选中即复制:每次选区变化都写剪贴板会把手抖、连点也放大成噪音，
 * 故做 150ms 去抖；选区清空不复制（复制空串没有意义）。
 */
function onSelectionChange() {
  syncSelectionState();
  if (!copyOnSelect.value || !term) return;
  const text = term.getSelection();
  if (!text) return;
  if (copyOnSelectTimer.value) clearTimeout(copyOnSelectTimer.value);
  copyOnSelectTimer.value = setTimeout(async () => {
    copyOnSelectTimer.value = null;
    const ok = await writeClipboard(text);
    if (ok) {
      copyFailureNotified = false;
      return;
    }
    // 失败只在「关闭功能前」提示一次，避免拖选过程中反复弹窗
    if (!copyFailureNotified) {
      copyFailureNotified = true;
      ElMessage.warning("选中即复制不可用：请检查系统剪贴板权限，或关闭该功能后手动复制。");
    }
  }, 150);
}

// ── 右键 ──
// 无选区：右键即粘贴（Xshell 习惯，粘贴是右键的主要意图）。
// 有选区：语义歧义（是想复制选中内容，还是想粘贴），给菜单让用户选。
const contextMenu = ref<{ x: number; y: number } | null>(null);

function openContextMenuAt(event: MouseEvent) {
  const host = containerRef.value;
  const width = 150;
  const height = 96;
  // 贴边时向内翻转，避免菜单被窗口裁掉
  const x = Math.min(event.clientX, (host?.clientWidth ?? window.innerWidth) - width - 4);
  const y = Math.min(event.clientY, (host?.clientHeight ?? window.innerHeight) - height - 4);
  contextMenu.value = { x: Math.max(4, x), y: Math.max(4, y) };
}

function onContextMenu(event: MouseEvent) {
  // 抑制 WebView 原生菜单
  event.preventDefault();
  syncSelectionState();
  if (hasSelection.value) {
    openContextMenuAt(event);
    return;
  }
  closeContextMenu();
  // 粘贴失败（剪贴板空/不可用/未连接）由 pasteClipboard 各自给出可见提示
  void pasteClipboard();
}

function closeContextMenu() {
  contextMenu.value = null;
}

async function menuCopy() {
  closeContextMenu();
  await copySelection();
}

function menuPaste() {
  closeContextMenu();
  void pasteClipboard();
}

function menuClear() {
  closeContextMenu();
  term?.clear();
}

// ── 终端配色方案 ──
/** 菜单里的「重新加载」哨兵：与真实方案名（来自后端）不会冲突 */
const RELOAD_SCHEMES = "__reload__";

const schemes = computed(() => themeStore.availableSchemes);
const currentScheme = computed(() => themeStore.currentScheme);

function reportSchemeError(prefix: string) {
  if (themeStore.error) ElMessage.error(`${prefix}：${themeStore.error}`);
}

function onSchemeChange(name: string) {
  if (name === RELOAD_SCHEMES) {
    // 方案列表可能刚被导入/卸载过，用户显式点「重新加载」才发 IPC
    void themeStore.refresh().then(() => reportSchemeError("重新加载配色方案失败"));
    return;
  }
  if (!name || name === themeStore.currentScheme) return;
  // applyScheme 失败时 store 回滚并写 error，这里原样暴露，不假装切换成功
  void themeStore.applyScheme(name).then(() => reportSchemeError("切换配色失败"));
}

function onTerminalTheme(event: Event) {
  if (term) term.options.theme = (event as CustomEvent<ITheme>).detail;
}

function findNext() {
  if (search && searchTerm.value) {
    search.findNext(searchTerm.value);
  }
}

function findPrev() {
  if (search && searchTerm.value) {
    search.findPrevious(searchTerm.value);
  }
}

function closeSearch() {
  searchBarVisible.value = false;
  if (search) search.clearDecorations();
}

/**
 * 从 :root 读取 v2 token,合成 xterm ITheme。
 * 任何 token 未设值(SSR / 测试环境)时回退到石墨底默认值。
 */
function readXtermThemeFromCssVars(): ITheme {
  const css = (name: string, fallback: string) => {
    if (typeof document === "undefined") return fallback;
    const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    return v || fallback;
  };
  return {
    background: css("--rs-bg", "#0e1116"),
    foreground: css("--rs-fg", "#e6edf3"),
    cursor: css("--rs-accent", "#58a6ff"),
    cursorAccent: css("--rs-bg", "#0e1116"),
    selectionBackground: css("--rs-row-selected", "#1f3658"),
    selectionForeground: css("--rs-fg", "#e6edf3"),
    black: css("--rs-p-graphite-3", "#2a313c"),
    red: "#f85149",
    green: "#3fb950",
    yellow: "#d29922",
    blue: css("--rs-accent", "#58a6ff"),
    magenta: "#bc8cff",
    cyan: "#39c5cf",
    white: "#adbac7",
    brightBlack: "#6e7681",
    brightRed: "#ff7b72",
    brightGreen: "#56d364",
    brightYellow: "#e3b341",
    brightBlue: css("--rs-accent", "#58a6ff"),
    brightMagenta: "#d2a8ff",
    brightCyan: "#56d4dd",
    brightWhite: "#e6edf3",
  };
}

function onTerminalAction(event: Event) {
  const detail = (event as CustomEvent<{ sessionId: Uuid; action: "find" | "clear" | "closeFind" | "copy" | "paste" }>)
    .detail;
  // R2-13：window 级快捷键（Ctrl+F/Escape）由 App.vue 统一拦截后经本事件
  // 路由到当前激活终端；这里按 sessionId 过滤，非激活面板不响应。
  if (detail.sessionId !== props.sessionId) return;
  if (detail.action === "find") searchBarVisible.value = true;
  if (detail.action === "clear") term?.clear();
  if (detail.action === "closeFind") closeSearch();
  if (detail.action === "copy") void copySelection();
  if (detail.action === "paste") void pasteClipboard();
}

// ── PROB-13：sendInput/resizeTerminal 持续失败的一次性可见提示 ──
// 单次瞬时失败不打扰；连续失败达阈值说明后端链路故障（按键正被静默丢弃），
// 弹一次 ElMessage；任一次成功即复位，允许下一轮持续失败再次提示。
const IO_FAILURE_NOTICE_THRESHOLD = 3;
let ioFailureStreak = 0;
let ioFailureNotified = false;

function noteIoSuccess() {
  ioFailureStreak = 0;
  ioFailureNotified = false;
}

function noteIoFailure(what: string, e: unknown) {
  console.error(`[TerminalPane] ${what} failed`, e);
  ioFailureStreak += 1;
  if (ioFailureStreak >= IO_FAILURE_NOTICE_THRESHOLD && !ioFailureNotified) {
    ioFailureNotified = true;
    ElMessage.error(
      "终端与后端通信持续失败：输入/尺寸调整未送达。请检查会话连接状态，必要时重连会话。",
    );
  }
}

/**
 * attach_terminal：把字节流 Channel 注册到后端（幂等，可重试）。
 * 失败时置 attachError 供面板内错误状态条展示，成功则清除。返回是否成功。
 */
async function attachTerminal(): Promise<boolean> {
  if (!term || !channel) return false;
  try {
    await invoke("attach_terminal", {
      session_id: props.sessionId,
      on_data: channel,
    });
    attachError.value = null;
    return true;
  } catch (e) {
    attachError.value = ipcErrorMessage(e);
    console.error("[TerminalPane] attach_terminal failed", e);
    return false;
  }
}

/** 错误状态条上的「重试附加」：真实重新执行 attach_terminal */
async function retryAttach() {
  await attachTerminal();
}

onMounted(async () => {
  if (!containerRef.value) return;

  term = new Terminal({
    fontFamily: 'JetBrains Mono, Menlo, Consolas, "DejaVu Sans Mono", monospace',
    fontSize: 13,
    cursorBlink: true,
    scrollback: 10000,
    theme: themeStore.terminalTheme ?? readXtermThemeFromCssVars(),
  });
  // 未连接时先禁输入，等真实 connected 事件再放开
  term.options.disableStdin = !isConnected.value;

  // Backspace 映射：xterm.js 默认回车送 \x7f (DEL)。多数远端 shell 的行规程以
  // \x08 (BS) 作为 erase 字符，收到 \x7f 时屏幕字符不消失——表现为「按退格没反应」。
  // 这里拦截该键并经与键盘输入相同的 sendInput 通路直送 \x08。
  // Ctrl+Backspace 保留 xterm 默认的 \x7f（部分 shell 用它删除前一个词）。
  term.attachCustomKeyEventHandler((event) => {
    if (event.type !== "keydown") return true;
    if (event.key !== "Backspace" || event.ctrlKey || event.metaKey || event.altKey) {
      return true;
    }
    event.preventDefault();
    event.stopPropagation();
    const sid = props.sessionId;
    if (!sid) return false;
    sendInput(sid, new TextEncoder().encode("\x08"))
      .then(() => noteIoSuccess())
      .catch((e) => noteIoFailure("send_input", e));
    return false;
  });

  fit = new FitAddon();
  term.loadAddon(fit);

  // 搜索 addon(切片 2.4)：findNext/findPrevious 在搜索栏 UI 触发
  search = new SearchAddon();
  term.loadAddon(search);

  // WebGL addon + onContextLoss 兜底（设计 §9 #3）
  try {
    const webgl = new WebglAddon();
    webgl.onContextLoss(() => {
      console.warn("[TerminalPane] WebGL context lost; falling back to canvas (default renderer)");
      webgl.dispose();
    });
    term.loadAddon(webgl);
  } catch (e) {
    console.warn("[TerminalPane] WebGL addon failed to load, using default canvas renderer", e);
  }

  term.open(containerRef.value);
  fit.fit();
  window.addEventListener("rshell:terminal-theme", onTerminalTheme);

  // 选中即复制：订阅 xterm 选区变化
  term.onSelectionChange(onSelectionChange);

  // 后端 → 前端字节流通道（设计 §4.1）
  channel = new Channel<number[]>();
  channel.onmessage = (data: number[]) => {
    if (term) {
      // data 是后端 Vec<u8>;xterm.write 接受 string 或 Uint8Array
      term.write(new Uint8Array(data));
    }
  };

  // 失败时错误状态条已在面板内可见（PROB-13），不中断后续本地渲染与监听注册
  await attachTerminal();

  // 前端 → 后端:键入数据直接转发（设计 §4.3 流程 A 末步）
  term.onData((data) => {
    const sid = props.sessionId;
    if (!sid) return;
    if (!isConnected.value) {
      // 断线时用户仍可能粘贴/敲键，明确告知而不是静默丢弃
      ElMessage.warning("会话未连接，输入已忽略。请先重连会话。");
      return;
    }
    sendInput(sid, new TextEncoder().encode(data))
      .then(() => noteIoSuccess())
      .catch((e) => noteIoFailure("send_input", e));
  });

  // 切片 2.4:搜索栏经 rshell:terminal-action 路由（R2-13：window 级
  // Ctrl+F/Escape 由 App.vue 统一拦截，避免多面板并存时同时作用于所有面板）。
  window.addEventListener("rshell:terminal-action", onTerminalAction);

  // 右键菜单在终端容器内，document 监听只负责「点到面板外关闭」
  document.addEventListener("mousedown", onDocumentMouseDown, true);
  document.addEventListener("keydown", onDocumentKeydown, true);

  // 尺寸变化:前端权威 resize_terminal（设计 §4.2 表格"终端尺寸"行）
  // ResizeObserver 缺失的环境（老 WebView / 非 DOM 测试）退化为只 fit 一次，
  // 不让构造器抛错把整个面板挂掉。
  const onResize = () => {
    if (fit && term) {
      try {
        fit.fit();
        const { cols, rows } = term;
        const sid = props.sessionId;
        if (!sid) return;
        resizeTerminal(sid, cols, rows)
          .then(() => noteIoSuccess())
          .catch((e) => noteIoFailure("resize_terminal", e));
      } catch {
        /* ignore fit errors during teardown */
      }
    }
  };
  if (typeof ResizeObserver === "undefined") {
    onResize();
  } else {
    const ro = new ResizeObserver(onResize);
    ro.observe(containerRef.value);
    detachSizeObserver = () => ro.disconnect();
  }
  // 配色方案列表不在这里拉：App.vue 挂载时已 refresh 过 theme store，
  // 方案变化由 ColorSchemeListChanged / 后端刷新驱动；用户要手动重拉时
  // 用工具栏的「重新加载方案…」。
});

function onDocumentMouseDown(e: MouseEvent) {
  if (!contextMenu.value) return;
  const target = e.target as HTMLElement | null;
  if (target?.closest(".term-context-menu")) return;
  closeContextMenu();
}

function onDocumentKeydown(e: KeyboardEvent) {
  if (e.key === "Escape" && contextMenu.value) closeContextMenu();
}

onBeforeUnmount(() => {
  if (copyOnSelectTimer.value) {
    clearTimeout(copyOnSelectTimer.value);
    copyOnSelectTimer.value = null;
  }
  document.removeEventListener("mousedown", onDocumentMouseDown, true);
  document.removeEventListener("keydown", onDocumentKeydown, true);
  detachSizeObserver?.();
  detachSizeObserver = null;
  window.removeEventListener("rshell:terminal-theme", onTerminalTheme);
  window.removeEventListener("rshell:terminal-action", onTerminalAction);
  term?.dispose();
  term = null;
  channel = null;
});
</script>

<template>
  <div class="terminal-pane-wrapper" :class="{ 'is-dropped': isDropped }">
    <!-- PROB-13：attach_terminal 失败时的错误状态条（替代空白终端） -->
    <div v-if="attachError" class="terminal-error-bar" role="alert">
      <span class="terminal-error-text">
        终端连接失败：{{ attachError }}。请重试附加；若会话已断开，请先重连会话。
      </span>
      <el-button size="small" type="danger" plain @click="retryAttach">重试附加</el-button>
    </div>

    <!-- 断开/连接中的状态条：状态取自后端 ConnectionStateChanged，不是本地猜测 -->
    <div v-if="isDropped" class="terminal-dropped-bar" role="status" data-test="term-dropped">
      <span class="rs-status-dot rs-status-dot--failed" aria-hidden="true" />
      <span class="dropped-text">
        会话已{{ connectionState === "connecting" ? "重新连接中" : "断开" }}，输入已暂停。重新连接后可继续键入。
      </span>
      <el-button
        size="small"
        type="primary"
        plain
        data-test="term-reconnect"
        :loading="reconnecting"
        :disabled="!sessionId || connectionState === 'connecting'"
        @click="reconnect"
      >
        重新连接
      </el-button>
    </div>
    <div
      v-else-if="connectionState === 'connecting'"
      class="terminal-dropped-bar is-connecting"
      role="status"
      data-test="term-connecting"
    >
      <span class="rs-status-dot rs-status-dot--connecting" aria-hidden="true" />
      <span class="dropped-text">正在建立连接…</span>
    </div>

    <!-- 工具栏：连接状态 + 终端配色 + 选中即复制 -->
    <div class="terminal-toolbar" data-test="term-toolbar">
      <span class="conn-chip" :class="`is-${connectionState}`" data-test="term-conn">
        <span class="rs-status-dot" :class="`rs-status-dot--${connectionState}`" aria-hidden="true" />
        {{ connectionLabel }}
      </span>
      <el-dropdown trigger="click" @command="onSchemeChange">
        <button class="tb-btn" data-test="term-scheme" :title="`终端配色：${currentScheme || '未选择'}`" aria-label="终端配色方案">
          <svg width="12" height="12" viewBox="0 0 16 16" aria-hidden="true">
            <path d="M8 2 A6 6 0 1 0 14 8 H8 Z" fill="none" stroke="currentColor" stroke-width="1.2" />
            <circle cx="5.5" cy="6" r="1" fill="currentColor" />
            <circle cx="8" cy="4.5" r="1" fill="currentColor" />
            <circle cx="10.5" cy="6" r="1" fill="currentColor" />
          </svg>
          <span class="tb-label">{{ currentScheme || "配色" }}</span>
        </button>
        <template #dropdown>
          <el-dropdown-menu>
            <el-dropdown-item
              v-for="name in schemes"
              :key="name"
              :command="name"
              :disabled="name === currentScheme"
            >
              {{ name }}
            </el-dropdown-item>
            <el-dropdown-item v-if="schemes.length === 0" disabled>
              暂无可用配色方案
            </el-dropdown-item>
            <el-dropdown-item divided :command="RELOAD_SCHEMES">重新加载方案…</el-dropdown-item>
          </el-dropdown-menu>
        </template>
      </el-dropdown>
      <el-switch
        :model-value="copyOnSelect"
        size="small"
        data-test="term-copy-on-select"
        title="选中即复制"
        aria-label="选中即复制"
        @update:model-value="onCopyOnSelectToggle"
      />
      <span class="tb-label">选中即复制</span>
    </div>

    <!-- 切片 2.4 搜索栏（默认隐藏） -->
    <div v-if="searchBarVisible" class="search-bar">
      <el-input
        v-model="searchTerm"
        size="small"
        placeholder="搜索终端内容"
        @keyup.enter="findNext"
        clearable
      />
      <el-button-group size="small">
        <el-button @click="findPrev">上</el-button>
        <el-button @click="findNext">下</el-button>
      </el-button-group>
      <el-button size="small" @click="closeSearch">关闭</el-button>
    </div>

    <div
      ref="containerRef"
      class="terminal-pane"
      data-test="term-canvas"
      @contextmenu="onContextMenu"
    />

    <!-- 终端右键菜单 -->
    <ul
      v-if="contextMenu"
      class="term-context-menu"
      data-test="term-context-menu"
      role="menu"
      :style="{ left: `${contextMenu.x}px`, top: `${contextMenu.y}px` }"
    >
      <li role="none">
        <button
          class="term-menu-item"
          role="menuitem"
          data-test="term-menu-copy"
          :disabled="!hasSelection"
          @click="menuCopy"
        >
          复制
        </button>
      </li>
      <li role="none">
        <button
          class="term-menu-item"
          role="menuitem"
          data-test="term-menu-paste"
          @click="menuPaste"
        >
          粘贴
        </button>
      </li>
      <li role="none">
        <button class="term-menu-item" role="menuitem" data-test="term-menu-clear" @click="menuClear">
          清屏
        </button>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.terminal-pane-wrapper {
  width: 100%;
  height: 100%;
  position: relative;
  background: var(--rs-bg);
  overflow: hidden;
}
/* 掉线时给一层极淡的压暗，视线能立刻分辨「这块是死的」 */
.terminal-pane-wrapper.is-dropped::after {
  content: "";
  position: absolute;
  inset: 0;
  background: rgb(0 0 0 / 18%);
  pointer-events: none;
  z-index: 1;
}
.terminal-pane {
  width: 100%;
  height: 100%;
  padding: 4px;
  box-sizing: border-box;
  overflow: hidden;
}

.terminal-toolbar {
  position: absolute;
  top: 6px;
  right: 12px;
  z-index: 12;
  display: flex;
  align-items: center;
  gap: var(--rs-s-2);
  padding: 3px 6px;
  font-size: var(--rs-fs-xs);
  color: var(--rs-fg-muted);
  background: var(--rs-bg-surface);
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-1);
  opacity: 0.75;
  transition: opacity var(--rs-dur-fast) var(--rs-easing);
}
.terminal-toolbar:hover { opacity: 1; }
.tb-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 20px;
  padding: 0 4px;
  background: transparent;
  border: none;
  border-radius: var(--rs-radius-1);
  color: var(--rs-fg-muted);
  font-family: var(--rs-font-ui);
  font-size: var(--rs-fs-xs);
  cursor: pointer;
}
.tb-btn:hover { background: var(--rs-bg-surface-hover); color: var(--rs-fg); }
.tb-label { white-space: nowrap; }
.conn-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  white-space: nowrap;
}
.conn-chip.is-connected { color: var(--rs-progress-done); }
.conn-chip.is-failed,
.conn-chip.is-disconnected { color: var(--rs-p-danger); }
.conn-chip.is-connecting { color: var(--rs-fg-muted); }

.search-bar {
  position: absolute;
  top: 36px;
  right: 16px;
  display: flex;
  gap: 6px;
  align-items: center;
  background: var(--el-bg-color);
  padding: 6px 8px;
  border: 1px solid var(--el-border-color);
  border-radius: 4px;
  z-index: 10;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
}
.terminal-error-bar {
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  display: flex;
  gap: 10px;
  align-items: center;
  max-width: 80%;
  padding: 10px 14px;
  background: var(--el-bg-color);
  border: 1px solid var(--el-color-danger, #f85149);
  border-radius: 6px;
  color: var(--el-color-danger, #f85149);
  z-index: 20;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
}
.terminal-error-text {
  font-size: 13px;
  line-height: 1.5;
  word-break: break-word;
}
.terminal-dropped-bar {
  position: absolute;
  top: 36px;
  left: 12px;
  z-index: 12;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 8px;
  font-size: var(--rs-fs-xs);
  color: var(--rs-p-danger);
  background: var(--rs-bg-surface);
  border: 1px solid var(--rs-p-danger);
  border-radius: var(--rs-radius-1);
}
.terminal-dropped-bar.is-connecting {
  color: var(--rs-fg-muted);
  border-color: var(--rs-border);
}
.dropped-text { color: var(--rs-fg); }

.term-context-menu {
  position: absolute;
  z-index: 30;
  margin: 0;
  padding: 4px 0;
  list-style: none;
  min-width: 132px;
  background: var(--rs-bg-surface);
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-1);
  box-shadow: 0 6px 18px rgb(0 0 0 / 32%);
}
.term-menu-item {
  display: block;
  width: 100%;
  padding: 5px 14px;
  text-align: left;
  background: transparent;
  border: none;
  color: var(--rs-fg);
  font-family: var(--rs-font-ui);
  font-size: var(--rs-fs-xs);
  cursor: pointer;
}
.term-menu-item:hover:not(:disabled) { background: var(--rs-bg-surface-hover); }
.term-menu-item:disabled { color: var(--rs-fg-disabled); cursor: not-allowed; }
</style>
