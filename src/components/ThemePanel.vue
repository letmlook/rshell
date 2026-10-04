<script setup lang="ts">
/**
 * ThemePanel —— 切片 3
 *
 * 选择应用主题（CSS 变量）。终端配色方案与「选中即复制」等终端配置在
 * 「设置 → 终端」面板，同一区域里只留一处入口，避免两个面板各改一半。
 */
import { onMounted } from "vue";
import { useThemeStore } from "../stores/theme";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const store = useThemeStore();

onMounted(async () => {
  await store.refresh();
});
</script>

<template>
  <aside class="theme-panel">
    <h3 v-if="!props.embedded">主题</h3>
    <p v-if="store.error" class="error">{{ store.error }}</p>
    <section>
      <label>应用主题</label>
      <el-select :model-value="store.currentTheme" @change="store.applyTheme" :loading="store.loading">
        <el-option
          v-for="name in store.availableThemes"
          :key="name"
          :label="name"
          :value="name"
        />
      </el-select>
    </section>
    <p class="hint">终端配色与剪贴板行为在「设置 → 终端」。</p>
  </aside>
</template>

<style scoped>
.theme-panel {
  padding: 12px;
  border-bottom: 1px solid var(--el-border-color);
}
.theme-panel h3 {
  margin: 0 0 12px;
  font-size: 14px;
}
.theme-panel section {
  margin-bottom: 12px;
}
.theme-panel label {
  display: block;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin-bottom: 4px;
}
.hint {
  margin: 0;
  font-size: 11px;
  color: var(--el-text-color-secondary);
}
.error {
  color: var(--el-color-danger);
  font-size: 12px;
}
</style>
