<script setup lang="ts">
/**
 * TriggerEditor —— 切片 7.3
 *
 * 触发器列表和动作编辑；远端输出匹配后由后端执行动作。
 *
 * 后端的 `TriggerCondition` 有两种（正则匹配 / 精确匹配）、`TriggerAction`
 * 有四种（发送文本 / 显示通知 / 断开连接 / 记录到文件），四种动作都已实现
 * （`rshell-core/src/session/service.rs`）。此前界面只能创建「正则 + 发送文本」
 * 一种，其余三种动作没有任何入口——已实现的能力等于没有。现在创建表单覆盖
 * 全部组合。
 *
 * 条件标签按**实际行为**命名：后端的 `ExactMatch` 实现为「输出片段包含该文本」
 * （输出是任意分片，等值比较几乎不会命中），因此界面叫「包含文本」而不是
 * 「精确匹配」，免得用户以为只有整段相等才会触发。
 */
import { onBeforeUnmount, onMounted, ref } from "vue";
import { listTriggers, createTrigger, deleteTrigger, toggleTrigger } from "../ipc/client";
import { subscribeAppEvents } from "../ipc/events";
import type { Trigger, TriggerAction, TriggerCondition, Uuid } from "../ipc/types";
import { confirmDialog } from "../utils/dialog";
import { pickLocalPath } from "../utils/pathPicker";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

function triggerPattern(t: Trigger): string {
  const c = t.condition as { RegexAppear?: string; ExactMatch?: string };
  if (c.RegexAppear !== undefined) return c.RegexAppear;
  if (c.ExactMatch !== undefined) return `exact: ${c.ExactMatch}`;
  return "(none)";
}

const items = ref<Trigger[]>([]);
/** 触发器名称：此前恒等于匹配条件，列表里「名称」与「正则」两列内容重复 */
const newName = ref("");
const newPattern = ref("");
const newText = ref("");
const loading = ref(false);
const error = ref<string | null>(null);
let unlisten: (() => void) | null = null;
let active = false;

/** 创建表单的类别选择 */
type ConditionKind = "regex" | "exact";
type ActionKind = "send" | "notify" | "disconnect" | "log";

const newCondition = ref<ConditionKind>("regex");
const newAction = ref<ActionKind>("send");
/** 「记录到文件」用的绝对路径 */
const newLogPath = ref("");

async function refresh() {
  loading.value = true;
  try {
    items.value = await listTriggers();
    error.value = null;
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function add() {
  if (!newPattern.value.trim()) {
    error.value = "匹配条件不能为空";
    return;
  }
  // 名称可留空：留空时回落为匹配条件（与旧行为一致），填了才单独命名

  // 断开连接不需要参数；「记录到文件」的参数是路径而不是文本
  if (
    (newAction.value === "send" || newAction.value === "notify") &&
    !newText.value.trim()
  ) {
    error.value = "动作内容不能为空";
    return;
  }
  if (newAction.value === "log" && !newLogPath.value.trim()) {
    error.value = "请选择要写入的日志文件";
    return;
  }

  const condition: TriggerCondition =
    newCondition.value === "regex"
      ? { RegexAppear: newPattern.value.trim() }
      : { ExactMatch: newPattern.value.trim() };

  let action: TriggerAction;
  switch (newAction.value) {
    case "send":
      action = { SendText: newText.value };
      break;
    case "notify":
      action = { ShowNotification: newText.value };
      break;
    case "disconnect":
      action = "Disconnect";
      break;
    case "log":
      action = { LogToFile: newLogPath.value.trim() };
      break;
  }

  try {
    await createTrigger({
      id: crypto.randomUUID() as Uuid,
      name: newName.value.trim() || newPattern.value.trim(),
      enabled: true,
      condition,
      action,
    });
  } catch (e) {
    error.value = String(e);
    return;
  }
  newPattern.value = "";
  newText.value = "";
  newLogPath.value = "";
  await refresh();
}

/** 为「记录到文件」选一个日志文件（应用内路径选择器） */
async function pickLogFile() {
  const path = await pickLocalPath({
    mode: "file",
    title: "选择触发器日志文件",
    accept: (name) => name.endsWith(".log") || name.endsWith(".txt"),
  });
  if (path) newLogPath.value = path;
}

async function toggle(t: Trigger) {
  try { await toggleTrigger(t.id); } catch (e) { error.value = String(e); return; }
  await refresh();
}

async function remove(t: Trigger) {
  // R2-14：删除持久数据必须有确认流（与会话/密钥/快速命令/远程文件删除一致）。
  const ok = await confirmDialog({
    title: "删除触发器",
    message: `删除触发器「${t.name}」？此操作不可撤销。`,
    confirmText: "删除",
    danger: true,
  });
  if (!ok) return;
  try {
    await deleteTrigger(t.id);
  } catch (e) {
    error.value = String(e);
    return;
  }
  await refresh();
}

function actionLabel(t: Trigger): string {
  if (t.action === "Disconnect") return "断开连接";
  const a = t.action as {
    SendText?: string;
    ShowNotification?: string;
    Disconnect?: unknown;
    LogToFile?: string;
  };
  if (a.SendText !== undefined) return `发送文本（${a.SendText.length} 字符）`;
  if (a.ShowNotification !== undefined) return `通知：${a.ShowNotification}`;
  if (a.LogToFile) return `记录到文件：${a.LogToFile}`;
  return "(none)";
}

onMounted(async () => {
  active = true;
  await refresh();
  if (!active) return;
  const stop = await subscribeAppEvents((event) => {
    if (event === "TriggerListChanged") void refresh();
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
  <section class="trigger-editor">
    <header v-if="!props.embedded">
      <h3>触发器 ({{ items.length }})</h3>
      <el-button size="small" :loading="loading" @click="refresh">刷新</el-button>
    </header>
    <p v-if="error" class="error" role="alert">{{ error }}</p>
    <el-form inline size="small" class="add-form" @submit.prevent="add">
      <el-form-item label="名称">
        <el-input
          v-model="newName"
          placeholder="触发器名称"
          style="width: 130px"
          data-test="trigger-name"
        />
      </el-form-item>
      <el-form-item label="匹配">
        <el-select v-model="newCondition" style="width: 110px" data-test="trigger-condition">
          <el-option label="正则" value="regex" />
          <el-option label="包含文本" value="exact" />
        </el-select>
      </el-form-item>
      <el-form-item label="条件">
        <el-input
          v-model="newPattern"
          :placeholder="newCondition === 'regex' ? '^\\$' : '登录成功'"
          style="width: 140px"
          data-test="trigger-pattern"
        />
      </el-form-item>
      <el-form-item label="动作">
        <el-select v-model="newAction" style="width: 130px" data-test="trigger-action">
          <el-option label="发送文本" value="send" />
          <el-option label="显示通知" value="notify" />
          <el-option label="断开连接" value="disconnect" />
          <el-option label="记录到文件" value="log" />
        </el-select>
      </el-form-item>
      <el-form-item
        v-if="newAction === 'send' || newAction === 'notify'"
        label="内容"
      >
        <el-input
          v-model="newText"
          :placeholder="newAction === 'notify' ? '登录成功' : 'clear'"
          style="width: 160px"
          data-test="trigger-text"
        />
      </el-form-item>
      <el-form-item v-if="newAction === 'log'" label="日志文件">
        <el-input
          v-model="newLogPath"
          placeholder="绝对路径"
          style="width: 220px"
          data-test="trigger-log-path"
        />
        <el-button size="small" @click="pickLogFile">选择…</el-button>
      </el-form-item>
      <el-form-item>
        <el-button type="primary" native-type="submit" data-test="trigger-add">添加</el-button>
      </el-form-item>
    </el-form>
    <el-empty v-if="items.length === 0" description="暂无触发器" :image-size="60" />
    <!--
      触发器列表用卡片而不是表格。

      表格五列（名称 140 + 正则 140 + 启用 80 + 操作 80 = 440px 固定列）
      **放不进默认侧栏（280px，最大 360px）**：真机实测「启用」开关与
      「删除」按钮整列落在视口外 —— 用户既看不到触发器是否启用，也删不掉它。
    -->
    <ul v-else class="rs-cards" data-test="trigger-list">
      <li
        v-for="row in items"
        :key="row.id"
        class="rs-card"
        :data-test="`trigger-card-${row.id}`"
      >
        <div class="rs-card-head">
          <span class="rs-card-title" :title="row.name">{{ row.name }}</span>
          <el-switch
            :model-value="row.enabled"
            size="small"
            :aria-label="`启用触发器 ${row.name}`"
            @change="toggle(row)"
          />
        </div>
        <div class="rs-card-body">
          <div class="rs-card-line">
            <span class="rs-card-label">正则</span>
            <span class="rs-card-value rs-card-value-wrap" :title="triggerPattern(row)">
              {{ triggerPattern(row) }}
            </span>
          </div>
          <div class="rs-card-line">
            <span class="rs-card-label">动作</span>
            <span class="rs-card-value rs-card-value-wrap">{{ actionLabel(row) }}</span>
          </div>
        </div>
        <div class="rs-card-actions">
          <el-button size="small" type="danger" @click="remove(row)">删除</el-button>
        </div>
      </li>
    </ul>
  </section>
</template>

<style scoped>
.trigger-editor {
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
.add-form {
  margin-bottom: 12px;
}
.error {
  color: var(--el-color-danger);
  font-size: 12px;
}
</style>
