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
import { computed, onMounted, ref } from "vue";
import { useThemeStore } from "../stores/theme";
import {
  copyOnSelect,
  setCopyOnSelect,
  setTerminalAppearance,
  terminalAppearance,
  TERMINAL_FONT_PRESETS,
} from "../utils/terminalPrefs";
import { importColorScheme, ipcErrorMessage } from "../ipc/client";
import { pickLocalPath } from "../utils/pathPicker";
import { parseColorSchemeText } from "../utils/colorScheme";
import { ElMessage } from "element-plus/es/components/message/index.mjs";
import { readTextFile } from "@tauri-apps/plugin-fs";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const store = useThemeStore();

const schemes = computed(() => store.availableSchemes);
const currentScheme = computed(() => store.currentScheme);

const importing = ref(false);
const importError = ref<string | null>(null);
const importNotice = ref<string | null>(null);

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

/**
 * 导入自定义配色方案。
 *
 * 文件是 JSON，颜色为 0xRRGGBB 整数。后端导入后会落盘，重启仍可用；
 * 解析或导入失败都把真实原因显示出来，不静默失败。
 */
async function importScheme() {
  importError.value = null;
  importNotice.value = null;
  const path = await pickLocalPath({
    mode: "file",
    title: "选择配色方案文件（JSON）",
    accept: (name) => name.toLowerCase().endsWith(".json"),
  });
  if (!path) return;

  importing.value = true;
  try {
    const text = await readTextFile(path);
    const scheme = parseColorSchemeText(text);
    await importColorScheme(scheme);
    importNotice.value = `已导入配色方案「${scheme.name}」`;
    await store.refresh();
  } catch (e) {
    importError.value = ipcErrorMessage(e);
  } finally {
    importing.value = false;
  }
}

function onFontSizeChange(next: number | string | undefined) {
  const value = Number(next);
  if (!Number.isFinite(value)) return;
  setTerminalAppearance({ fontSize: value });
}

function onFontFamilyChange(next: string) {
  if (!next) return;
  setTerminalAppearance({ fontFamily: next });
}

function onCursorStyleChange(next: string) {
  setTerminalAppearance({
    cursorStyle: next as "block" | "underline" | "bar",
  });
}

function onCursorBlinkChange(next: boolean | string | number) {
  setTerminalAppearance({ cursorBlink: next === true || next === "true" });
}

function onScrollbackChange(next: number | string | undefined) {
  const value = Number(next);
  if (!Number.isFinite(value)) return;
  setTerminalAppearance({ scrollback: value });
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
        <el-button
          size="small"
          :loading="importing"
          data-test="ts-scheme-import"
          @click="importScheme"
        >
          导入…
        </el-button>
      </div>
      <p v-if="importError" class="error" role="alert" data-test="ts-scheme-import-error">{{ importError }}</p>
      <p v-if="importNotice && !importError" class="notice" data-test="ts-scheme-import-notice">{{ importNotice }}</p>
      <p class="hint">
        配色由后端已安装的方案决定，切换对所有已打开的标签立即生效。
        导入的方案会保存到本机，重启后仍在方案列表中。
      </p>
    </section>

    <section>
      <label for="ts-font-size">终端外观</label>
      <div class="row">
        <span class="row-text">字号</span>
        <el-input-number
          id="ts-font-size"
          :model-value="terminalAppearance.fontSize"
          :min="8"
          :max="32"
          size="small"
          style="width: 110px"
          data-test="ts-font-size"
          @change="onFontSizeChange"
        />
        <span class="row-text">光标</span>
        <el-select
          :model-value="terminalAppearance.cursorStyle"
          size="small"
          style="width: 110px"
          data-test="ts-cursor-style"
          @change="onCursorStyleChange"
        >
          <el-option label="方块" value="block" />
          <el-option label="下划线" value="underline" />
          <el-option label="竖线" value="bar" />
        </el-select>
      </div>
      <div class="row">
        <span class="row-text">字体</span>
        <el-select
          :model-value="terminalAppearance.fontFamily"
          size="small"
          style="width: 200px"
          data-test="ts-font-family"
          @change="onFontFamilyChange"
        >
          <el-option
            v-for="preset in TERMINAL_FONT_PRESETS"
            :key="preset.value"
            :label="preset.label"
            :value="preset.value"
          />
        </el-select>
      </div>
      <div class="row">
        <span class="row-text">回滚行数</span>
        <el-input-number
          :model-value="terminalAppearance.scrollback"
          :min="0"
          :max="200000"
          :step="1000"
          size="small"
          style="width: 140px"
          data-test="ts-scrollback"
          @change="onScrollbackChange"
        />
        <el-switch
          :model-value="terminalAppearance.cursorBlink"
          size="small"
          data-test="ts-cursor-blink"
          aria-label="光标闪烁"
          @update:model-value="onCursorBlinkChange"
        />
        <span class="row-text">光标闪烁</span>
      </div>
      <p class="hint">
        外观改动对已打开的终端立即生效，不必关掉重开。偏好存在本机，不随连接信息走。
      </p>
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
.notice {
  color: var(--el-color-success);
  font-size: 12px;
}
</style>
