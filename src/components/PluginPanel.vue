<script setup lang="ts">
import { onMounted, ref } from "vue";
import { listPlugins, scanPlugins, loadPlugin, unloadPlugin } from "../ipc/client";
import type { PluginInfo } from "../ipc/types";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const items = ref<PluginInfo[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);

function stateLabel(p: PluginInfo): string {
  return ({ Discovered: "已发现", Loaded: "已加载", Active: "已启用", Disabled: "已禁用", Error: "错误" })[p.state];
}

/**
 * 插件类型 → 可读说法。
 *
 * `plugin_type` 后端一直有填（Builtin / Wasm / DynamicLib），此前界面上完全不
 * 显示 —— 用户看不出装的是哪种插件。Rust 枚举直接显示英文不直观，这里翻成中文。
 */
const PLUGIN_TYPE_LABELS: Record<string, string> = {
  Builtin: "内置",
  Wasm: "WASM 模块",
  DynamicLib: "动态库",
};

function pluginTypeLabel(type: string): string {
  return PLUGIN_TYPE_LABELS[type] ?? type;
}

/**
 * 清单声明的权限名 → 可读说法。
 *
 * 权限名是 Rust 枚举的 Debug 输出（`NetworkAccess` 之类），直接显示给用户
 * 不直观；这里翻成中文。
 */
const PERMISSION_LABELS: Record<string, string> = {
  NetworkAccess: "网络访问",
  FileSystemAccess: "文件系统",
  SessionAccess: "会话",
  TerminalAccess: "终端",
  ClipboardAccess: "剪贴板",
  ProcessExecution: "进程执行",
};

function permissionLabel(raw: string): string {
  return PERMISSION_LABELS[raw] ?? raw;
}

/**
 * 扩展点名 → 可读说法。
 *
 * 后端把扩展点也用 Debug 输出传过来（`ToolPanel { id: "x", title: "验收" }`），
 * 直接显示等于把 Rust 内部结构甩给用户，而且这串文本很长，会把卡片撑破、
 * 让侧栏出现横向滚动条。这里翻成「工具面板（验收）」这样的说法。
 */
const EXTENSION_LABELS: Record<string, string> = {
  Protocol: "协议",
  Theme: "主题",
  ColorScheme: "配色",
  ToolPanel: "工具面板",
  QuickCommand: "快速命令",
  FileAction: "文件操作",
  TriggerAction: "触发器动作",
  StatusBar: "状态栏",
};

/**
 * 把扩展点的 Debug 串翻成一句话。
 *
 * 取值优先级按「哪个字段是给人看的」排：title > name > scheme > position > id；
 * 都取不到就把串里所有引号内容拼起来（例如 `QuickCommand { commands: ["a","b"] }`）。
 */
function extensionLabel(raw: string): string {
  const variant = /^([A-Za-z]+)/.exec(raw)?.[1] ?? "";
  const base = EXTENSION_LABELS[variant] ?? variant;
  if (!variant) return raw;
  const fields: Record<string, string> = {};
  for (const m of raw.matchAll(/(\w+):\s*"([^"]*)"/g)) fields[m[1]] = m[2];
  const quoted = [...raw.matchAll(/"([^"]*)"/g)].map((m) => m[1]);
  const detail =
    fields.title ??
    fields.name ??
    fields.scheme ??
    fields.position ??
    fields.id ??
    (quoted.length > 0 ? quoted.join(" / ") : "");
  return detail ? `${base}（${detail}）` : base;
}

async function refresh() {
  loading.value = true;
  error.value = null;
  try {
    items.value = await listPlugins();
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function scan() {
  try {
    await scanPlugins();
    await refresh();
  } catch (e) {
    error.value = String(e);
  }
}

async function load(id: string) {
  try {
    await loadPlugin(id);
    await refresh();
  } catch (e) {
    error.value = String(e);
  }
}

async function unload(id: string) {
  try {
    await unloadPlugin(id);
    await refresh();
  } catch (e) {
    error.value = String(e);
  }
}

onMounted(scan);
</script>

<template>
  <section class="plugin-panel">
    <header v-if="!props.embedded">
      <h3>插件 ({{ items.length }})</h3>
      <el-button-group size="small">
        <el-button @click="refresh" :loading="loading">刷新</el-button>
        <el-button @click="scan">扫描</el-button>
      </el-button-group>
    </header>

    <p class="hint">仅加载受信任的本地 WASM 插件。插件目录位于应用数据目录的 plugins 下。</p>
    <p class="hint hint-caution" data-test="plugin-capability-note">
      「已加载」只表示模块<strong>编译并登记</strong>成功：当前宿主不向 WASM guest
      提供任何宿主能力（网络 / 文件系统 / 会话 / 终端 / 剪贴板 / 进程均未实现），
      清单里声明的权限<strong>不会生效</strong>。
    </p>
    <p v-if="error" class="error">{{ error }}</p>

    <el-empty v-if="items.length === 0" description="暂未发现插件" :image-size="60" />
    <!--
      插件列表用卡片而不是表格。

      表格九列（版本 80 + 说明 140 + 权限 150 + 扩展点 120 + 状态 100 +
      失败原因 200 + 操作 220 = 1010px 固定列）**放不进默认侧栏
      （280px，最大 360px）**：没写宽度的「Plugin ID」「名称」两列宽度塌成 0
      （真机实测名称整列空白），最右侧的「加载 / 卸载」按钮落在视口外。
    -->
    <ul v-else class="rs-cards" data-test="plugin-list">
      <li
        v-for="row in items"
        :key="row.id"
        class="rs-card"
        :data-test="`plugin-card-${row.id}`"
      >
        <div class="rs-card-head">
          <span class="rs-card-title" :title="row.name">{{ row.name }}</span>
          <!--
            disable-transitions：el-tag 默认把内容包在 <Transition appear> 里，
            靠「双 requestAnimationFrame」收尾 enter-from 类。而文档不可见时
            （窗口不在当前 Space / 屏幕锁着）WebKit 会挂起 rAF —— 那个类就永远
            留在元素上，标签停在 opacity:0 + scaleX(0)，整块看不见。
            真机实测：插件状态/权限/扩展点三个标签整体不可见（AX 树里却有文字）。
            状态标签本来也不需要缩放动画，直接关掉过渡。
          -->
          <el-tag
            size="small"
            disable-transitions
            :type="row.state === 'Active' ? 'success' : row.state === 'Error' ? 'danger' : 'info'"
          >
            {{ stateLabel(row) }}
          </el-tag>
        </div>
        <div class="rs-card-body">
          <div class="rs-card-line">
            <span class="rs-card-label">类型</span>
            <span class="rs-card-value">{{ pluginTypeLabel(row.plugin_type) }}</span>
          </div>
          <div class="rs-card-line">
            <span class="rs-card-label">版本</span>
            <span class="rs-card-value">{{ row.version }}</span>
          </div>
          <div v-if="row.description || row.author" class="rs-card-line">
            <span class="rs-card-label">说明</span>
            <span class="rs-card-value" :title="row.description || row.author">
              {{ row.description || row.author }}
            </span>
          </div>
          <div v-if="row.permissions && row.permissions.length" class="rs-card-line">
            <span class="rs-card-label">权限</span>
            <span class="rs-card-tags">
              <el-tag
                v-for="perm in row.permissions"
                :key="perm"
                size="small"
                disable-transitions
                type="warning"
              >
                {{ permissionLabel(perm) }}
              </el-tag>
            </span>
          </div>
          <div v-if="row.extensions && row.extensions.length" class="rs-card-line">
            <span class="rs-card-label">扩展点</span>
            <span class="rs-card-tags">
              <el-tag
                v-for="ext in row.extensions"
                :key="ext"
                size="small"
                disable-transitions
                :title="ext"
              >
                {{ extensionLabel(ext) }}
              </el-tag>
            </span>
          </div>
          <div v-if="row.error" class="rs-card-line">
            <span class="rs-card-label">失败</span>
            <span class="rs-card-value rs-card-value-wrap plugin-error">{{ row.error }}</span>
          </div>
        </div>
        <div class="rs-card-meta">
          <span class="rs-card-value" :title="row.id">ID {{ row.id }}</span>
        </div>
        <div class="rs-card-actions">
          <el-button
            v-if="row.state === 'Discovered' || row.state === 'Disabled'"
            size="small"
            @click="load(row.id)"
          >
            加载
          </el-button>
          <el-button v-if="row.state === 'Loaded' || row.state === 'Active'" size="small" @click="unload(row.id)">
            卸载
          </el-button>
        </div>
      </li>
    </ul>
  </section>
</template>

<style scoped>
.plugin-panel {
  padding: 12px;
}
header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
}
h3 {
  margin: 0;
  font-size: 14px;
}
.hint {
  color: var(--el-text-color-secondary);
  font-size: 12px;
  margin: 0 0 12px;
  background: var(--el-fill-color-light);
  padding: 8px;
  border-radius: 4px;
}
.error {
  color: var(--el-color-danger);
  font-size: 12px;
}
/* 插件加载失败的原因：此前是普通灰，看起来像普通说明文字 */
.plugin-error {
  color: var(--el-color-danger);
}
</style>
