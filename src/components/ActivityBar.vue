<script setup lang="ts">
/**
 * ActivityBar —— 切片 10
 *
 * 最左侧窄条(类似 VSCode),点击图标切换中间二级面板的内容。
 * 折叠状态由父组件 v-model:expanded 控制。
 */
const props = defineProps<{
  active: string;
  expanded: boolean;
}>();

const emit = defineEmits<{
  (e: "update:active", panel: string): void;
  (e: "update:expanded", value: boolean): void;
}>();

const items = [
  { id: "sessions", icon: "▤", label: "会话" },
  { id: "files", icon: "▥", label: "文件" },
  { id: "keys", icon: "⚷", label: "密钥" },
  { id: "tools", icon: "⚒", label: "工具" },
  { id: "settings", icon: "⚙", label: "设置" },
];

function pick(id: string) {
  emit("update:active", id);
  if (!props.expanded) emit("update:expanded", true);
}
</script>

<template>
  <nav class="activity-bar" :class="{ collapsed: !expanded }">
    <div class="activity-bar-top">
      <button
        v-for="item in items"
        :key="item.id"
        :class="['activity-item', { active: active === item.id }]"
        :title="item.label"
        @click="pick(item.id)"
      >
        <span class="activity-icon">{{ item.icon }}</span>
      </button>
    </div>
    <div class="activity-bar-bottom">
      <button
        class="activity-toggle"
        :title="expanded ? '折叠侧边栏' : '展开侧边栏'"
        @click="emit('update:expanded', !expanded)"
      >
        <span class="activity-icon">{{ expanded ? "«" : "»" }}</span>
      </button>
    </div>
  </nav>
</template>

<style scoped>
.activity-bar {
  display: flex;
  flex-direction: column;
  width: 220px;
  background: var(--rshell-sidebar-bg, #1a1d22);
  border-right: 1px solid var(--rshell-border, #2c313a);
  transition: width 0.15s ease;
}
.activity-bar.collapsed {
  width: 0;
  border-right: none;
  overflow: hidden;
}
.activity-bar-top {
  flex: 1;
  display: flex;
  flex-direction: column;
  padding: 4px 0;
}
.activity-bar-bottom {
  padding: 4px 0;
  border-top: 1px solid var(--rshell-border, #2c313a);
}
.activity-item,
.activity-toggle {
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: 10px;
  width: 100%;
  height: 36px;
  padding: 0 16px;
  background: transparent;
  border: none;
  color: var(--rshell-sidebar-fg-muted, #9097a3);
  cursor: pointer;
  font-size: 13px;
  text-align: left;
  transition: background 0.1s, color 0.1s;
}
.activity-item:hover,
.activity-toggle:hover {
  background: rgba(255, 255, 255, 0.04);
  color: var(--rshell-sidebar-fg, #e6e6e6);
}
.activity-item.active {
  color: var(--rshell-sidebar-fg, #ffffff);
  background: var(--rshell-sidebar-active-bg, rgba(64, 158, 255, 0.15));
  border-left: 2px solid var(--el-color-primary);
  padding-left: 14px;
}
.activity-icon {
  font-size: 16px;
  width: 20px;
  text-align: center;
}
</style>