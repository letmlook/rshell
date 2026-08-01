<script setup lang="ts">
/**
 * CustomTitleBar —— 切片 10
 *
 * 自定义窗口标题栏(对应 tauri.conf.json decorations: false):
 * - 左:RShell logo + 主菜单(会话/视图/帮助)
 * - 中:可拖动区域(app-region: drag)
 * - 右:窗口控件(最小化/最大化/关闭)
 *
 * 窗口控制通过 @tauri-apps/api/window 的 getCurrentWindow() 调用。
 */
import { onMounted, onBeforeUnmount, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ElDropdown, ElDropdownMenu, ElDropdownItem } from "element-plus";

const isMaximized = ref(false);
const currentLabel = ref("");

async function refreshMaximize() {
  try {
    isMaximized.value = await getCurrentWindow().isMaximized();
  } catch {
    /* headless 环境可能不可用 */
  }
}

async function minimize() {
  await getCurrentWindow().minimize();
}

async function toggleMaximize() {
  const w = getCurrentWindow();
  if (await w.isMaximized()) {
    await w.unmaximize();
  } else {
    await w.maximize();
  }
  await refreshMaximize();
}

async function close() {
  await getCurrentWindow().close();
}

let unlistenResize: (() => void) | null = null;

onMounted(async () => {
  await refreshMaximize();
  // 监听窗口变化以同步最大化按钮状态
  const w = getCurrentWindow();
  unlistenResize = await w.onResized(() => refreshMaximize());
});

onBeforeUnmount(() => {
  unlistenResize?.();
  unlistenResize = null;
});
</script>

<template>
  <header class="titlebar">
    <!-- 左:logo + 主菜单 -->
    <div class="titlebar-left">
      <div class="logo">
        <span class="logo-mark">⌬</span>
        <span class="logo-text">RShell</span>
      </div>
      <nav class="menus">
        <el-dropdown trigger="click">
          <span class="menu-item">会话</span>
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item @click="$emit('new-session')">新建会话</el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
        <el-dropdown trigger="click">
          <span class="menu-item">视图</span>
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item @click="$emit('toggle-sidebar')">切换侧边栏</el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
        <el-dropdown trigger="click">
          <span class="menu-item">帮助</span>
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item disabled>RShell v0.1.0 (切片 0-9)</el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
      </nav>
    </div>

    <!-- 中:可拖动区域 + 当前会话标签 -->
    <div class="titlebar-center" data-tauri-drag-region>
      <span class="current-label">{{ currentLabel || "RShell — 远程终端客户端" }}</span>
    </div>

    <!-- 右:窗口控件 -->
    <div class="titlebar-right">
      <button class="window-btn" @click="minimize" title="最小化" aria-label="最小化">
        <svg width="12" height="12" viewBox="0 0 12 12">
          <rect x="2" y="5.5" width="8" height="1" fill="currentColor" />
        </svg>
      </button>
      <button class="window-btn" @click="toggleMaximize" :title="isMaximized ? '还原' : '最大化'" aria-label="最大化">
        <svg width="12" height="12" viewBox="0 0 12 12">
          <rect x="2.5" y="2.5" width="7" height="7" fill="none" stroke="currentColor" stroke-width="1" />
        </svg>
      </button>
      <button class="window-btn close" @click="close" title="关闭" aria-label="关闭">
        <svg width="12" height="12" viewBox="0 0 12 12">
          <path d="M3 3 L9 9 M9 3 L3 9" stroke="currentColor" stroke-width="1" fill="none" />
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
}

.titlebar-left {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 0 12px;
  flex-shrink: 0;
}

.logo {
  display: flex;
  align-items: center;
  gap: 6px;
  font-weight: 600;
}
.logo-mark {
  font-size: 18px;
  color: var(--el-color-primary);
}
.logo-text {
  letter-spacing: 0.5px;
}

.menus {
  display: flex;
  gap: 4px;
}
.menu-item {
  padding: 4px 10px;
  border-radius: 4px;
  cursor: pointer;
  user-select: none;
}
.menu-item:hover {
  background: rgba(255, 255, 255, 0.06);
}

.titlebar-center {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  /* Tauri's data-tauri-drag-region 属性使整个区域可拖动窗口 */
}
.current-label {
  font-size: 11px;
  color: var(--rshell-titlebar-fg-muted, #8a8f99);
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
}
.window-btn:hover {
  background: rgba(255, 255, 255, 0.08);
}
.window-btn.close:hover {
  background: #e81123;
  color: white;
}
</style>