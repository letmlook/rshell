<script setup lang="ts">
/**
 * FileBrowserPanel —— 切片 5.2
 *
 * 双面板:本地 (el-tree) + 远端 (lazy load)。
 * 切片 5.2 雏形 —— 实际本地目录读取与 SFTP 列出留到切片 5.3+
 * (后者需 sessions 建立连接,前者需 tauri-plugin-fs 调用)。
 */
import { ref } from "vue";

const localRoot = ref("/");
const remoteRoot = ref("/");
</script>

<template>
  <section class="file-browser">
    <header>
      <el-input v-model="localRoot" placeholder="本地路径" size="small" />
      <el-input v-model="remoteRoot" placeholder="远端路径" size="small" />
    </header>
    <div class="panes">
      <div class="pane">
        <h4>本地</h4>
        <p class="hint">本地目录浏览（tauri-plugin-fs）—— 切片 5.3+ 接入</p>
      </div>
      <div class="pane">
        <h4>远端</h4>
        <p class="hint">远端目录浏览（SFTP BrowseRemoteDir）—— 切片 5.3+ 接入</p>
      </div>
    </div>
  </section>
</template>

<style scoped>
.file-browser {
  display: flex;
  flex-direction: column;
  height: 100%;
}
header {
  display: flex;
  gap: 8px;
  padding: 8px;
  border-bottom: 1px solid var(--el-border-color);
}
.panes {
  flex: 1;
  display: grid;
  grid-template-columns: 1fr 1fr;
  min-height: 0;
}
.pane {
  padding: 12px;
  border-right: 1px solid var(--el-border-color);
  overflow: auto;
}
.pane:last-child { border-right: none; }
h4 {
  margin: 0 0 8px;
  font-size: 13px;
}
.hint {
  color: var(--el-text-color-secondary);
  font-size: 12px;
}
</style>