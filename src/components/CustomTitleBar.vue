<script setup lang="ts">
/**
 * CustomTitleBar —— 切片 10 完整版
 *
 * 自定义窗口标题栏(对应 tauri.conf.json decorations: false),对齐全功能:
 * - 拖动区(双击切换最大化)
 * - 缩放边缘(8 方向)
 * - 左:logo + 主菜单(会话/视图/帮助),每项可点击
 * - 中:可拖动区 + 当前会话名
 * - 右:设置 + 最小化 + 最大化/还原 + 关闭
 */
import { onMounted, onBeforeUnmount, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useSessionsStore } from "../stores/sessions";

const emit = defineEmits<{
  (e: "new-session"): void;
  (e: "toggle-sidebar"): void;
  (e: "open-key-manager"): void;
  (e: "open-theme-panel"): void;
  (e: "open-plugin-panel"): void;
  (e: "open-transfer-queue"): void;
  (e: "open-quick-commands"): void;
  (e: "open-triggers"): void;
  (e: "open-tunnels"): void;
  (e: "about"): void;
}>();

const store = useSessionsStore();
const isMaximized = ref(false);
const currentLabel = ref("");

async function refreshMaximize() {
  try {
    isMaximized.value = await getCurrentWindow().isMaximized();
  } catch {
    /* headless 环境不可用 */
  }
}

async function minimize() {
  try {
    await getCurrentWindow().minimize();
  } catch (e) {
    console.warn("minimize failed", e);
  }
}

async function toggleMaximize() {
  try {
    const w = getCurrentWindow();
    if (await w.isMaximized()) {
      await w.unmaximize();
    } else {
      await w.maximize();
    }
    await refreshMaximize();
  } catch (e) {
    console.warn("toggleMaximize failed", e);
  }
}

async function close() {
  try {
    await getCurrentWindow().close();
  } catch (e) {
    console.warn("close failed", e);
  }
}

async function onDragDoubleClick() {
  await toggleMaximize();
}

let unlistenResize: (() => void) | null = null;

onMounted(async () => {
  await refreshMaximize();
  const w = getCurrentWindow();
  unlistenResize = await w.onResized(() => refreshMaximize());
});

onBeforeUnmount(() => {
  unlistenResize?.();
  unlistenResize = null;
});

defineExpose({
  refreshMaximize,
});
</script>

<template>
  <header
    class="titlebar"
    :class="{ 'is-maximized': isMaximized }"
    data-tauri-drag-region
    style="-webkit-app-region: drag"
  >
    <!-- 左:logo + 主菜单 -->
    <div class="titlebar-left" data-tauri-drag-region="false" style="-webkit-app-region: no-drag">
      <div class="logo">
        <span class="logo-mark">⌬</span>
        <span class="logo-text">RShell</span>
      </div>
      <nav class="menus">
        <!-- 会话菜单 -->
        <el-dropdown trigger="click">
          <span class="menu-item">会话</span>
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item @click="emit('new-session')">
                <span class="mi-icon">＋</span>新建会话
              </el-dropdown-item>
              <el-dropdown-item @click="store.refresh()">
                <span class="mi-icon">↻</span>刷新列表
              </el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>

        <!-- 视图菜单 -->
        <el-dropdown trigger="click">
          <span class="menu-item">视图</span>
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item @click="emit('toggle-sidebar')">
                <span class="mi-icon">▤</span>切换侧边栏
              </el-dropdown-item>
              <el-dropdown-item @click="emit('open-theme-panel')">
                <span class="mi-icon">🎨</span>主题
              </el-dropdown-item>
              <el-dropdown-item @click="emit('open-key-manager')">
                <span class="mi-icon">⚷</span>SSH 密钥
              </el-dropdown-item>
              <el-dropdown-item @click="emit('open-plugin-panel')">
                <span class="mi-icon">⚙</span>插件
              </el-dropdown-item>
              <el-dropdown-item @click="emit('open-transfer-queue')">
                <span class="mi-icon">⇄</span>传输队列
              </el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>

        <!-- 工具菜单 -->
        <el-dropdown trigger="click">
          <span class="menu-item">工具</span>
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item @click="emit('open-quick-commands')">
                <span class="mi-icon">⚡</span>快速命令
              </el-dropdown-item>
              <el-dropdown-item @click="emit('open-triggers')">
                <span class="mi-icon">⚡</span>触发器
              </el-dropdown-item>
              <el-dropdown-item @click="emit('open-tunnels')">
                <span class="mi-icon">⇄</span>端口转发
              </el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>

        <!-- 帮助菜单 -->
        <el-dropdown trigger="click">
          <span class="menu-item">帮助</span>
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item disabled>RShell v0.1.0</el-dropdown-item>
              <el-dropdown-item @click="emit('about')">
                <span class="mi-icon">ⓘ</span>关于
              </el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
      </nav>
    </div>

    <!-- 中:可拖动区域 + 当前会话标签 -->
    <div
      class="titlebar-center"
      data-tauri-drag-region
      style="-webkit-app-region: drag"
      @dblclick="onDragDoubleClick"
    >
      <span class="current-label">
        {{ currentLabel || (store.current?.name ?? "RShell — 远程终端客户端") }}
      </span>
    </div>
    <div class="titlebar-right" data-tauri-drag-region="false" style="-webkit-app-region: no-drag">
      <button
        class="window-btn"
        title="最小化"
        aria-label="最小化"
        @click="minimize"
      >
        <svg width="12" height="12" viewBox="0 0 12 12">
          <rect x="2" y="5.5" width="8" height="1" fill="currentColor" />
        </svg>
      </button>
      <button
        class="window-btn"
        :title="isMaximized ? '还原' : '最大化'"
        :aria-label="isMaximized ? '还原' : '最大化'"
        @click="toggleMaximize"
      >
        <svg v-if="!isMaximized" width="12" height="12" viewBox="0 0 12 12">
          <rect
            x="2.5"
            y="2.5"
            width="7"
            height="7"
            fill="none"
            stroke="currentColor"
            stroke-width="1"
          />
        </svg>
        <svg v-else width="12" height="12" viewBox="0 0 12 12">
          <rect
            x="3.5"
            y="3.5"
            width="6"
            height="6"
            fill="none"
            stroke="currentColor"
            stroke-width="1"
          />
          <path
            d="M3.5 3.5 V2 H10.5 V8.5 H9"
            fill="none"
            stroke="currentColor"
            stroke-width="1"
          />
        </svg>
      </button>
      <button
        class="window-btn close"
        title="关闭"
        aria-label="关闭"
        @click="close"
      >
        <svg width="12" height="12" viewBox="0 0 12 12">
          <path
            d="M3 3 L9 9 M9 3 L3 9"
            stroke="currentColor"
            stroke-width="1"
            fill="none"
          />
        </svg>
      </button>
    </div>
  </header>
</template>

<style scoped>
.titlebar {
  display: flex;
  align-items: stretch;
  height: 36px;
  background: var(--rshell-titlebar-bg, #1f2329);
  color: var(--rshell-titlebar-fg, #e6e6e6);
  border-bottom: 1px solid var(--rshell-border, #2c313a);
  user-select: none;
  -webkit-user-select: none;
  font-size: 12px;
  margin: 0;
  padding: 0;
  flex-shrink: 0;
}

.titlebar-left {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 0 8px;
  flex-shrink: 0;
}

.logo {
  display: flex;
  align-items: center;
  gap: 6px;
  font-weight: 600;
  padding: 0 6px;
  height: 100%;
  cursor: default;
}
.logo-mark {
  font-size: 18px;
  color: var(--el-color-primary);
  line-height: 36px;
}
.logo-text {
  letter-spacing: 0.5px;
  line-height: 36px;
}

.menus {
  display: flex;
  gap: 0;
  height: 100%;
}
.menu-item {
  padding: 0 10px;
  border-radius: 0;
  cursor: pointer;
  user-select: none;
  line-height: 36px;
  height: 36px;
  display: inline-flex;
  align-items: center;
  transition: background 0.1s;
}
.menu-item:hover {
  background: rgba(255, 255, 255, 0.06);
}

.mi-icon {
  display: inline-block;
  width: 16px;
  text-align: center;
  margin-right: 6px;
  opacity: 0.85;
}

.titlebar-center {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  min-width: 0;
}
.titlebar-center:active {
  cursor: grabbing;
}

.current-label {
  font-size: 11px;
  color: var(--rshell-titlebar-fg-muted, #8a8f99);
  user-select: none;
  pointer-events: none;
}

.titlebar-right {
  display: flex;
  align-items: stretch;
  flex-shrink: 0;
}

.window-btn {
  width: 46px;
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  color: inherit;
  cursor: pointer;
  transition: background 0.1s;
  padding: 0;
  margin: 0;
}
.window-btn:hover {
  background: rgba(255, 255, 255, 0.08);
}
.window-btn.close:hover {
  background: #e81123;
  color: white;
}
</style>