<script setup lang="ts">
/**
 * TerminalSettingsPanel —— 设置 → 终端
 *
 * 终端的**配置项**集中在这里，不再散在终端面板右上角的浮层里：
 *   - 终端配色方案：走后端 ListThemes 的 available_schemes + SetTerminalColorScheme
 *   - 选中即复制：存 localStorage，改完已打开的终端立即生效
 *
 * 终端面板本身只保留状态反馈（断开/连接中条、压暗）与操作入口（右键菜单），
 * 不再堆配置控件——设置是设置，终端是终端。
 */
import { computed, onMounted } from "vue";
import { useThemeStore } from "../stores/theme";
import { copyOnSelect, setCopyOnSelect } from "../utils/terminalPrefs";
import { ElMessage } from "element-plus/es/components/message/index.mjs";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const store = useThemeStore();

const schemes = computed(() => store.availableSchemes);
const currentScheme = computed(() => store.currentScheme);

function reportError(prefix: string) {
  if (store.error) ElMessage.error(`${prefix}：${store.error}`);
}

function onSchemeChange(name: string) {
  if (!name || name === store.currentScheme) return;
  // applyScheme 失败时 store 回滚并写 error，这里原样暴露，不假装切换成功
  void store.applyScheme(name).then(() => reportError("切换配色失败"));
}

/** 方案可能被导入/卸载过，显式点「重新加载」才发 IPC */
function reloadSchemes() {
  void store.refresh().then(() => reportError("重新加载配色方案失败"));
}

function onCopyOnSelectChange(next: boolean | string | number) {
  const value = next === true || next === "true";
  // 落盘失败只说明「下次启动不记住」，不影响本次运行
  if (!setCopyOnSelect(value)) {
    console.warn("[TerminalSettingsPanel] 选中即复制偏好未能持久化");
  }
}

onMounted(async () => {
  if (store.availableSchemes.length === 0) await store.refresh();
});
</script>

<template>
  <aside class="terminal-settings">
    <h3 v-if="!props.embedded">终端</h3>
    <p v-if="store.error" class="error" role="alert">{{ store.error }}</p>

    <section>
      <label for="ts-scheme">终端配色方案</label>
      <div class="row">
        <el-select
          id="ts-scheme"
          :model-value="currentScheme"
          :loading="store.loading"
          :disabled="schemes.length === 0"
          placeholder="暂无可用配色方案"
          data-test="ts-scheme"
          @change="onSchemeChange"
        >
          <el-option
            v-for="name in schemes"
            :key="name"
            :label="name"
            :value="name"
            :disabled="name === currentScheme"
          />
        </el-select>
        <el-button size="small" :loading="store.loading" data-test="ts-scheme-reload" @click="reloadSchemes">
          重新加载
        </el-button>
      </div>
      <p class="hint">配色由后端已安装的方案决定，切换对所有已打开的标签立即生效。</p>
    </section>

    <section>
      <label>剪贴板</label>
      <div class="row">
        <el-switch
          :model-value="copyOnSelect"
          size="small"
          data-test="ts-copy-on-select"
          aria-label="选中即复制"
          @update:model-value="onCopyOnSelectChange"
        />
        <span class="row-text">选中即复制</span>
      </div>
      <p class="hint">
        在终端里拖选文本后自动写入系统剪贴板（去抖 150ms）。剪贴板不可用时会提示一次，
        不会反复弹窗。此偏好存在本机，不随连接信息走。
      </p>
    </section>

    <section>
      <label>鼠标</label>
      <p class="hint">
        终端内没有选区时右键直接粘贴；有选区时右键弹出菜单（复制 / 粘贴 / 清屏），
        避免刚选中的内容被一次粘贴顶掉。
      </p>
    </section>
  </aside>
</template>

<style scoped>
.terminal-settings {
  padding: 12px;
  border-bottom: 1px solid var(--el-border-color);
}
.terminal-settings h3 {
  margin: 0 0 12px;
  font-size: 14px;
}
.terminal-settings section {
  margin-bottom: 16px;
}
.terminal-settings label {
  display: block;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin-bottom: 6px;
}
.row {
  display: flex;
  align-items: center;
  gap: 8px;
}
.row-text {
  font-size: 12px;
  color: var(--el-text-color-primary);
}
.row .el-select { flex: 1; min-width: 0; }
.hint {
  margin: 6px 0 0;
  font-size: 11px;
  line-height: 1.6;
  color: var(--el-text-color-secondary);
}
.error {
  color: var(--el-color-danger);
  font-size: 12px;
}
</style>
