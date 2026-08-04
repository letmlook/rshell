<script setup lang="ts">
/**
 * ThemePanel —— 切片 3
 *
 * 选择应用主题 + 终端配色方案。前端管展开态/排序列/过滤词（设计 §5）；
 * scheme → CSS 变量映射（见 utils/themeCss.ts）由前端计算，避免后端发整
 * 套颜色 JSON。
 *
 * 注意：store.refresh() + subscribeEvents() 由 App.vue 启动时统一调用，
 * 这里不再重复触发，否则第二次 invoke('list_themes') 是无意义的往返。
 */
import { useThemeStore } from "../stores/theme";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const store = useThemeStore();
</script>

<template>
  <aside class="theme-panel">
    <h3 v-if="!props.embedded">主题</h3>
    <p v-if="store.error" class="error">{{ store.error }}</p>
    <section>
      <label>应用主题</label>
      <el-select v-model="store.currentTheme" @change="store.applyTheme" :loading="store.loading">
        <el-option
          v-for="name in store.availableThemes"
          :key="name"
          :label="name"
          :value="name"
        />
      </el-select>
    </section>
    <section>
      <label>终端配色方案</label>
      <el-select v-model="store.currentScheme" @change="store.applyScheme" :loading="store.loading">
        <el-option
          v-for="name in store.availableSchemes"
          :key="name"
          :label="name"
          :value="name"
        />
      </el-select>
    </section>
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
.error {
  color: var(--el-color-danger);
  font-size: 12px;
}
</style>