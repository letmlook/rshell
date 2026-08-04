<script setup lang="ts">
/**
 * TerminalPane —— 切片 1.3 + 切片 2.4 搜索
 *
 * 设计 §4.3 流程 A:挂载 xterm → invoke('attach_terminal', { session_id, onData: ch })
 * → flush 积压 → 转 Attached。term.onData → invoke('send_input')。
 *
 * 切片 2.4:Element Plus 搜索栏 + @xterm/addon-search 的 findNext/findPrevious。
 *
 * onContextLoss:WebGL addon 在某些环境下会触发 context loss,自动 fallback 到 canvas
 * (设计 §9 #3 + 切片 1.3 完成判据)。
 */
import { onMounted, onBeforeUnmount, ref } from "vue";
import { Terminal } from "@xterm/xterm";
import type { ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { SearchAddon } from "@xterm/addon-search";
import { Channel } from "@tauri-apps/api/core";
import { invoke } from "@tauri-apps/api/core";
import { sendInput, resizeTerminal } from "../ipc/client";
import type { Uuid } from "../ipc/types";
import { useThemeStore } from "../stores/theme";

const props = defineProps<{
  sessionId?: Uuid;
}>();

const themeStore = useThemeStore();

const containerRef = ref<HTMLDivElement | null>(null);
const searchBarVisible = ref(false);
const searchTerm = ref("");
let term: Terminal | null = null;
let fit: FitAddon | null = null;
let search: SearchAddon | null = null;
let channel: Channel<number[]> | null = null;
let detachSizeObserver: (() => void) | null = null;

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

function onWindowKeydown(e: KeyboardEvent) {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
    e.preventDefault();
    searchBarVisible.value = !searchBarVisible.value;
  } else if (e.key === "Escape" && searchBarVisible.value) {
    closeSearch();
  }
}

onMounted(async () => {
  if (!containerRef.value) return;

  term = new Terminal({
    fontFamily: 'JetBrains Mono, Menlo, Consolas, "DejaVu Sans Mono", monospace',
    fontSize: 13,
    cursorBlink: true,
    scrollback: 10000,
    theme: readXtermThemeFromCssVars(),
  });

  // 注册到主题 store —— 主题切换时由 store 统一刷新 term.options.theme
  themeStore.registerXterm(term);

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

  // 后端 → 前端字节流通道（设计 §4.1）
  channel = new Channel<number[]>();
  channel.onmessage = (data: number[]) => {
    if (term) {
      // data 是后端 Vec<u8>;xterm.write 接受 string 或 Uint8Array
      term.write(new Uint8Array(data));
    }
  };

  try {
    // 后端 #[tauri::command(rename_all = "snake_case")]:形参名直接用 snake_case
    // 与 rshell-api 契约一致。
    await invoke("attach_terminal", {
      session_id: props.sessionId,
      on_data: channel,
    });
  } catch (e) {
    console.error("[TerminalPane] attach_terminal failed", e);
  }

  // 前端 → 后端:键入数据直接转发（设计 §4.3 流程 A 末步）
  term.onData((data) => {
    const sid = props.sessionId;
    if (!sid) return;
    sendInput(sid, new TextEncoder().encode(data)).catch((e) => {
      console.error("[TerminalPane] send_input failed", e);
    });
  });

  // 切片 2.4:Ctrl+F 切换搜索栏（前端拦截 keydown,不让 xterm 接走）
  // 简化实现:监听 window keydown,xterm 不会消费 Ctrl 组合键。
  window.addEventListener("keydown", onWindowKeydown);

  // 尺寸变化:前端权威 resize_terminal（设计 §4.2 表格"终端尺寸"行）
  const ro = new ResizeObserver(() => {
    if (fit && term) {
      try {
        fit.fit();
        const { cols, rows } = term;
        const sid = props.sessionId;
        if (!sid) return;
        resizeTerminal(sid, cols, rows).catch((e) => {
          console.warn("[TerminalPane] resize_terminal failed", e);
        });
      } catch {
        /* ignore fit errors during teardown */
      }
    }
  });
  ro.observe(containerRef.value);
  detachSizeObserver = () => ro.disconnect();
});

onBeforeUnmount(() => {
  detachSizeObserver?.();
  detachSizeObserver = null;
  window.removeEventListener("keydown", onWindowKeydown);
  if (term) {
    themeStore.unregisterXterm(term);
    term.dispose();
  }
  term = null;
  channel = null;
});
</script>

<template>
  <div class="terminal-pane-wrapper">
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
    <div ref="containerRef" class="terminal-pane" />
  </div>
</template>

<style scoped>
.terminal-pane-wrapper {
  width: 100%;
  height: 100%;
  position: relative;
  background: #1e1e1e;
  overflow: hidden;
}
.terminal-pane {
  width: 100%;
  height: 100%;
  padding: 4px;
  box-sizing: border-box;
  overflow: hidden;
}
.search-bar {
  position: absolute;
  top: 8px;
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
</style>