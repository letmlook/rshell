<script setup lang="ts">
/**
 * ComposePanel —— 撰写窗格
 *
 * 在一处编辑多行文本，一次性发到当前会话 / 全部会话 / 选定会话。
 * 典型用途：给一批机器统一下发一段配置或命令。
 *
 * 后端语义：
 * - 全部目标失败 → invoke 直接报错（不静默成功）。
 * - 部分失败 → invoke 成功，但每个失败会话会发 `ComposeSendFailed` 事件，
 *   界面据此逐条提示。
 * - 目标为空（没有可发送的会话）→ 报错，不假装已发送。
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { ipcErrorMessage, sendComposeText } from "../ipc/client";
import { subscribeAppEvents } from "../ipc/events";
import { useSessionsStore } from "../stores/sessions";
import type { ComposeTarget, Uuid } from "../ipc/types";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const sessions = useSessionsStore();

type TargetMode = "current" | "all" | "selected";

const content = ref("");
const mode = ref<TargetMode>("current");
const selected = ref<Uuid[]>([]);
const error = ref<string | null>(null);
const failures = ref<string[]>([]);
const sent = ref(false);
const sending = ref(false);

let unlisten: (() => void) | null = null;
let activeFlag = false;

const currentSession = computed(() => sessions.current);

function nameOf(id: Uuid): string {
  const found = sessions.items.find((s) => s.id === id);
  return found ? found.name : id.slice(0, 8);
}

function buildTarget(): ComposeTarget {
  switch (mode.value) {
    case "current":
      return "CurrentSession";
    case "all":
      return "AllSessions";
    case "selected":
      return { SelectedSessions: selected.value };
  }
}

async function send() {
  error.value = null;
  failures.value = [];
  sent.value = false;

  if (!content.value) {
    error.value = "请先输入要发送的内容";
    return;
  }
  if (mode.value === "current" && !currentSession.value) {
    error.value = "请先在左侧选择一个当前会话";
    return;
  }
  if (mode.value === "selected" && selected.value.length === 0) {
    error.value = "请至少选择一个目标会话";
    return;
  }

  sending.value = true;
  try {
    await sendComposeText(
      content.value,
      buildTarget(),
      mode.value === "current" ? currentSession.value?.id ?? null : null,
    );
    sent.value = true;
    error.value = null;
  } catch (e) {
    error.value = ipcErrorMessage(e);
  } finally {
    sending.value = false;
  }
}

onMounted(async () => {
  activeFlag = true;
  await sessions.refresh();
  if (!activeFlag) return;
  const stop = await subscribeAppEvents((event) => {
    if (typeof event === "string") return;
    if ("ComposeSendFailed" in event) {
      const { session_id, error: reason } = event.ComposeSendFailed;
      failures.value = [...failures.value, `会话「${nameOf(session_id)}」：${reason}`];
    }
  });
  if (activeFlag) unlisten = stop;
  else stop();
});

onBeforeUnmount(() => {
  activeFlag = false;
  unlisten?.();
  unlisten = null;
});
</script>

<template>
  <section class="compose-panel">
    <h3 v-if="!props.embedded">撰写窗格</h3>

    <p v-if="error" class="error" data-test="compose-error">{{ error }}</p>
    <p v-if="sent && !error" class="notice" data-test="compose-sent">已发送</p>
    <ul v-if="failures.length" class="failures" data-test="compose-failures">
      <li v-for="failure in failures" :key="failure">{{ failure }}</li>
    </ul>

    <el-form size="small" label-position="top" @submit.prevent="send">
      <el-form-item label="发送目标">
        <el-radio-group v-model="mode" data-test="compose-target">
          <el-radio value="current" data-test="compose-target-current">当前会话</el-radio>
          <el-radio value="all" data-test="compose-target-all">全部会话</el-radio>
          <el-radio value="selected" data-test="compose-target-selected">选定会话</el-radio>
        </el-radio-group>
      </el-form-item>

      <p v-if="mode === 'current'" class="hint" data-test="compose-current-hint">
        当前会话：{{ currentSession ? currentSession.name : "（未选择）" }}
      </p>

      <el-form-item v-if="mode === 'selected'" label="目标会话">
        <el-select
          v-model="selected"
          multiple
          placeholder="选择一个或多个会话"
          style="width: 100%"
          data-test="compose-session-select"
        >
          <el-option
            v-for="session in sessions.items"
            :key="session.id"
            :label="session.name"
            :value="session.id"
          />
        </el-select>
      </el-form-item>

      <el-form-item label="内容">
        <el-input
          v-model="content"
          type="textarea"
          :rows="8"
          placeholder="输入要发送的文本（支持多行）"
          data-test="compose-content"
        />
      </el-form-item>

      <el-button
        type="primary"
        native-type="submit"
        :loading="sending"
        data-test="compose-send"
      >
        发送
      </el-button>
    </el-form>
  </section>
</template>

<style scoped>
.compose-panel {
  padding: 12px;
}
.compose-panel h3 {
  margin: 0 0 12px;
  font-size: 14px;
}
.hint {
  margin: 0 0 10px;
  font-size: 12px;
  color: var(--rs-fg-muted);
}
.failures {
  margin: 0 0 10px;
  padding-left: 18px;
  font-size: 12px;
  color: var(--el-color-warning);
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
