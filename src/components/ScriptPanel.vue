<script setup lang="ts">
/**
 * ScriptPanel —— Rhai 脚本运行器
 *
 * 这不是完整的脚本 IDE（docs/08 明确「快速命令与触发器界面不代表完整脚本
 * IDE」）：只提供「选一个目标会话 → 写一段脚本 → 运行 → 看结果」这一条
 * 日常够用的链路。
 *
 * 能力边界（必须如实展示，不能让用户以为脚本能做更多事）：
 * - 脚本宿主只暴露一组 `rshell_*` 函数，没有文件读写、没有网络。
 * - 脚本有操作数与时间上限（`rshell_sleep` 受截止时间约束），不是无限执行。
 * - 失败的脚本会真实报错，成功与否都体现在输出区，不静默吞掉。
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { executeScript, ipcErrorMessage } from "../ipc/client";
import { subscribeAppEvents } from "../ipc/events";
import { useSessionsStore } from "../stores/sessions";
import type { ScriptResult, Uuid } from "../ipc/types";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const sessions = useSessionsStore();

const EXAMPLE = `// 列出全部会话 id
let ids = rshell_list_sessions();
rshell_log("会话数: " + ids.len());
for id in ids {
    rshell_log(id);
}
`;

const target = ref<Uuid | null>(null);
const code = ref<string>(EXAMPLE);
const running = ref(false);
const error = ref<string | null>(null);
const result = ref<ScriptResult | null>(null);

let unlisten: (() => void) | null = null;
let active = false;

const hostApi = [
  { sig: "rshell_log(msg)", desc: "写一条脚本日志" },
  { sig: "rshell_log_level(level, msg)", desc: "按级别写日志（error/warn/info/debug）" },
  { sig: "rshell_sleep(ms)", desc: "等待毫秒（受脚本时间上限约束）" },
  { sig: "rshell_send(session, text)", desc: "向指定会话发送文本" },
  { sig: "rshell_list_sessions()", desc: "返回全部会话 id 数组" },
  { sig: "rshell_execute_quick_command(cmd, session)", desc: "在指定会话执行一个快速命令" },
  { sig: "rshell_now_ms()", desc: "当前 Unix 毫秒时间戳" },
  { sig: "rshell_uuid_v4()", desc: "生成新的 UUID v4" },
  { sig: "rshell_parse_uuid(s)", desc: "解析 UUID，失败返回空串" },
  { sig: "rshell_version_compare(a, b)", desc: "比较两个版本串，返回 -1/0/1" },
];

const selectableSessions = computed(() => sessions.items);

async function run() {
  error.value = null;
  result.value = null;
  if (!target.value) {
    error.value = "请选择一个目标会话";
    return;
  }
  if (!code.value.trim()) {
    error.value = "脚本内容为空";
    return;
  }
  running.value = true;
  try {
    // 成功时结果通过 ScriptFinished 事件回来；失败时 invoke 直接拒绝
    await executeScript(code.value, target.value);
  } catch (e) {
    error.value = ipcErrorMessage(e);
  } finally {
    running.value = false;
  }
}

onMounted(async () => {
  active = true;
  await sessions.refresh();
  if (!target.value && sessions.items.length > 0) {
    target.value = sessions.items[0].id;
  }
  if (!active) return;
  const stop = await subscribeAppEvents((event) => {
    if (typeof event === "string") return;
    if ("ScriptFinished" in event) {
      result.value = event.ScriptFinished.result;
    }
  });
  if (active) unlisten = stop;
  else stop();
});

onBeforeUnmount(() => {
  active = false;
  unlisten?.();
  unlisten = null;
});
</script>

<template>
  <section class="script-panel">
    <h3 v-if="!props.embedded">脚本</h3>

    <p v-if="error" class="error" data-test="script-error">{{ error }}</p>
    <p v-if="result && !result.success && !error" class="error" data-test="script-failed">
      脚本失败：{{ result.error }}
    </p>

    <el-form size="small" label-position="top" @submit.prevent="run">
      <el-form-item label="目标会话">
        <el-select
          v-model="target"
          placeholder="选择目标会话"
          style="width: 100%"
          filterable
          data-test="script-session"
        >
          <el-option
            v-for="session in selectableSessions"
            :key="session.id"
            :label="session.name"
            :value="session.id"
          />
        </el-select>
      </el-form-item>

      <el-form-item label="脚本（Rhai）">
        <el-input
          v-model="code"
          type="textarea"
          :rows="10"
          placeholder="rshell_log(1 + 1)"
          data-test="script-code"
        />
      </el-form-item>

      <el-button type="primary" native-type="submit" :loading="running" data-test="script-run">
        运行
      </el-button>
    </el-form>

    <section v-if="result" class="result-block" data-test="script-result">
      <h4>{{ result.success ? "执行完成" : "执行失败" }}</h4>
      <pre v-if="result.output" class="result-output">{{ result.output }}</pre>
      <p v-if="result.error" class="result-error">{{ result.error }}</p>
    </section>

    <details class="api-block">
      <summary>脚本可用的宿主函数</summary>
      <ul>
        <li v-for="item in hostApi" :key="item.sig">
          <code>{{ item.sig }}</code>
          <span class="desc">{{ item.desc }}</span>
        </li>
      </ul>
      <p class="hint">
        脚本只能使用上面这些函数：没有文件读写、没有网络，也不能无限执行
        （操作数与时间都有上限）。
      </p>
    </details>
  </section>
</template>

<style scoped>
.script-panel {
  padding: 12px;
}
.script-panel h3 {
  margin: 0 0 12px;
  font-size: 14px;
}
.result-block {
  margin-top: 12px;
  padding: 8px 10px;
  border: 1px solid var(--rs-border);
  border-radius: 6px;
}
.result-block h4 {
  margin: 0 0 6px;
  font-size: 12px;
}
.result-output {
  margin: 0;
  padding: 6px;
  font-family: monospace;
  font-size: 11px;
  white-space: pre-wrap;
  word-break: break-all;
  background: var(--rs-bg-surface, #1b1b20);
  border-radius: 4px;
  max-height: 200px;
  overflow: auto;
}
.result-error {
  margin: 6px 0 0;
  font-size: 11px;
  color: var(--el-color-danger);
}
.api-block {
  margin-top: 12px;
  font-size: 11px;
}
.api-block summary {
  cursor: pointer;
  color: var(--rs-fg-muted);
}
.api-block ul {
  list-style: none;
  margin: 8px 0;
  padding: 0;
}
.api-block li {
  display: flex;
  gap: 8px;
  padding: 2px 0;
}
.api-block code {
  font-family: monospace;
  color: var(--rs-fg);
  white-space: nowrap;
}
.api-block .desc {
  color: var(--rs-fg-muted);
}
.hint {
  margin: 0;
  color: var(--rs-fg-muted);
  line-height: 1.6;
}
.error {
  color: var(--el-color-danger);
  font-size: 12px;
}
</style>
