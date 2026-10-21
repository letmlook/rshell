<script setup lang="ts">
/**
 * TunnelPanel —— 切片 8
 *
 * 本地端口转发与动态 SOCKS5 隧道管理。
 *
 * 覆盖后端已实现但此前没有入口的三件事：
 * - 挂起 / 恢复隧道（suspend / resume）：挂起会撤销已建立的转发连接并停止接受新连接。
 * - 待定隧道的恢复：应用重启后不抢占端口，只把用户意图记在 tunnels.toml 里，
 *   由用户在这里显式恢复（此前界面只显示被跳过的旧规则，可恢复的规则从不显示）。
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import {
  listTunnels,
  listPendingTunnels,
  createTunnel,
  closeTunnel,
  suspendTunnel,
  resumeTunnel,
  restoreTunnel,
} from "../ipc/client";
import { ipcErrorMessage } from "../ipc/client";
import { subscribeAppEvents } from "../ipc/events";
import type { Uuid, PortForwardRule, ActiveTunnelInfo, TunnelState } from "../ipc/types";
import { confirmDialog } from "../utils/dialog";
import { useSessionsStore } from "../stores/sessions";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const sessions = useSessionsStore();

const items = ref<ActiveTunnelInfo[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);
const unsupported = ref<string[]>([]);
/** 待恢复的隧道规则：[会话 id, 规则] */
const pending = ref<Array<[Uuid, PortForwardRule]>>([]);
/** 正在执行动作的隧道 id，避免重复点击 */
const pendingIds = ref<Set<Uuid>>(new Set());

// 事件订阅的生命周期（与 QuickCommandPanel 同款）：异步订阅完成可能晚于
// unmount，用 active 门闩保证晚到的订阅被立即释放。
let unlisten: (() => void) | null = null;
let active = false;

const draftType = ref<"Local" | "Dynamic">("Local");
const draftSession = ref<Uuid | null>(null);
const draftBind = ref("127.0.0.1:8080");
const draftTarget = ref("localhost:80");

/** 可选会话：只有真实存在的连接信息才能建隧道 */
const selectableSessions = computed(() => sessions.items);

async function refresh() {
  loading.value = true;
  try {
    const [activeTunnels, pendingInfo] = await Promise.all([
      listTunnels(),
      listPendingTunnels(),
    ]);
    items.value = activeTunnels;
    pending.value = pendingInfo.rules;
    unsupported.value = pendingInfo.unsupported.map(
      (rule) => `${rule.session_id}: ${rule.reason}`,
    );
    error.value = null;
  } catch (e) {
    error.value = ipcErrorMessage(e);
  } finally {
    loading.value = false;
  }
}

function markPending(id: Uuid, on: boolean) {
  const next = new Set(pendingIds.value);
  if (on) next.add(id);
  else next.delete(id);
  pendingIds.value = next;
}

function parseEndpoint(ep: string): { host: string; port: number } {
  const idx = ep.lastIndexOf(":");
  if (idx < 0) return { host: ep, port: 0 };
  return { host: ep.slice(0, idx), port: parseInt(ep.slice(idx + 1), 10) || 0 };
}

/**
 * 判断监听地址是否为本机回环（PROB-23，与后端 is_loopback_bind_address 一致）：
 * 接受 localhost、127.0.0.0/8、::1（含 [::1] 方括号写法）；其余一律按非回环处理。
 */
function isLoopbackHost(host: string): boolean {
  const bare = host.trim().replace(/^\[/, "").replace(/\]$/, "");
  if (bare.toLowerCase() === "localhost") return true;
  const ipv4 = bare.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
  if (ipv4) return ipv4[1] === "127";
  const lower = bare.toLowerCase();
  return lower === "::1" || lower === "0:0:0:0:0:0:0:1";
}

/** 当前监听地址是否会把隧道暴露到本机之外（PROB-23） */
const nonLoopbackBind = computed(() => {
  const host = parseEndpoint(draftBind.value).host.trim();
  return host !== "" && !isLoopbackHost(host);
});

/**
 * 监听字段的行内错误。提交过一次后才显示：面板刚打开就飘红字是噪音。
 *
 * 空 host（`:8080`）是死路，必须拦在提交之前（R3-16）：`TcpListener::bind(":port")`
 * 会绑定**所有**网卡，而后端 `is_loopback_bind_address("")` 判 false 并要求
 * `allow_non_loopback`——而界面对空 host 既不弹暴露确认也置不了这个标志
 * （保守判定不变：`""` 不算回环，也绝不放行到 bind）。结果是这条输入永远建不出
 * 隧道，提示还指向一个界面上不存在的选项。改为在表单里要求写明主机名。
 */
const bindSubmitted = ref(false);
const bindError = computed<string | null>(() => {
  if (!bindSubmitted.value) return null;
  if (parseEndpoint(draftBind.value).host.trim() !== "") return null;
  return '监听地址必须写明主机名（如 127.0.0.1:8080）：只写 ":8080" 会绑定所有网卡，界面不会提供暴露确认';
});

async function add() {
  bindSubmitted.value = true;
  if (bindError.value) return; // 监听地址无法判断暴露面：不发出必然失败的请求
  if (!draftSession.value) {
    error.value = "请先在主视图选择会话";
    return;
  }
  const bind = parseEndpoint(draftBind.value);
  const target = parseEndpoint(draftTarget.value);
  // 设计 §4.2 的 PortForwardRule 是单一 struct,通过 direction 字段区分。
  const rule: PortForwardRule = {
    bind_address: bind.host,
    bind_port: bind.port,
    remote_host: target.host,
    remote_port: target.port,
    direction: draftType.value,
    allow_non_loopback: false,
  };
  // PROB-23：非回环监听会把端口转发 / 无认证 SOCKS5 代理暴露给局域网，
  // 必须经用户确认并携带 allow_non_loopback 标志，否则后端拒绝创建。
  if (nonLoopbackBind.value) {
    const exposed = await confirmDialog({
      title: "将暴露给局域网",
      message: `监听地址「${bind.host}」不是回环地址：隧道将暴露给局域网，同网段任何主机都能使用该 SSH 连接转发（SOCKS5 代理无认证）。确认继续？`,
      confirmText: "确认暴露并创建",
      danger: true,
    });
    if (!exposed) return; // 用户取消：不创建
    rule.allow_non_loopback = true;
  }
  try {
    await createTunnel(draftSession.value, rule);
    error.value = null;
    await refresh();
  } catch (e) {
    error.value = ipcErrorMessage(e);
  }
}

async function remove(id: Uuid) {
  const confirmed = await confirmDialog({
    title: "关闭隧道",
    message: "关闭后监听端口立即释放，已建立的转发连接会被撤销。",
    confirmText: "关闭",
    danger: true,
  });
  if (!confirmed) return;
  markPending(id, true);
  try {
    await closeTunnel(id);
    error.value = null;
    await refresh();
  } catch (e) {
    error.value = ipcErrorMessage(e);
  } finally {
    markPending(id, false);
  }
}

/** 挂起：撤销已建立的转发连接并停止接受新连接 */
async function suspend(id: Uuid) {
  markPending(id, true);
  try {
    await suspendTunnel(id);
    error.value = null;
    await refresh();
  } catch (e) {
    error.value = ipcErrorMessage(e);
  } finally {
    markPending(id, false);
  }
}

/** 恢复挂起的隧道（后端会换新的取消通道，避免恢复出来即处于已取消状态） */
async function resume(id: Uuid) {
  markPending(id, true);
  try {
    await resumeTunnel(id);
    error.value = null;
    await refresh();
  } catch (e) {
    error.value = ipcErrorMessage(e);
  } finally {
    markPending(id, false);
  }
}

/**
 * 恢复重启前记录的待定隧道。
 *
 * 后端要求该会话当前已连接（要拿到 SSH 连接才能建转发），未连接时直接报错，
 * 这里把真实原因透出来，不假装已恢复。
 */
async function restore(sessionId: Uuid, rule: PortForwardRule) {
  markPending(sessionId, true);
  try {
    await restoreTunnel(sessionId, rule);
    error.value = null;
    await refresh();
  } catch (e) {
    error.value = ipcErrorMessage(e);
  } finally {
    markPending(sessionId, false);
  }
}

function sessionName(id: Uuid): string {
  return sessions.items.find((s) => s.id === id)?.name ?? id.slice(0, 8);
}

function ruleLabel(rule: PortForwardRule): string {
  const direction = rule.direction === "Dynamic" ? "Dynamic" : "Local";
  const target =
    rule.direction === "Dynamic"
      ? "SOCKS5"
      : `${rule.remote_host}:${rule.remote_port}`;
  return `${direction} ${rule.bind_address}:${rule.bind_port} → ${target}`;
}

/** 隧道状态的真实文案：Error 变体要显示原因，不能只显示 "0" */
function stateLabel(state: TunnelState): string {
  if (typeof state === "string") {
    return state === "Active" ? "运行中" : state === "Suspended" ? "已挂起" : state;
  }
  if (state && typeof state === "object" && "Error" in state) {
    return `错误：${state.Error}`;
  }
  return "—";
}

/** 只有挂起态可以恢复 */
function canResume(state: TunnelState): boolean {
  return state === "Suspended";
}

/** 只有运行态可以挂起 */
function canSuspend(state: TunnelState): boolean {
  return state === "Active";
}

// R2-04：后端在会话断开时会把该会话隧道置 Error 并逐条发布
// TunnelStateChanged + ActiveTunnelsChanged（tunnel_manager deactivate_session_tunnels）。
// 面板打开期间不订阅这两个事件，状态列会一直停留旧的 Active 文案，直到手点刷新。
onMounted(async () => {
  active = true;
  await sessions.refresh();
  await refresh();
  if (!active) return;
  const stop = await subscribeAppEvents((event) => {
    if (
      event === "ActiveTunnelsChanged" ||
      (typeof event !== "string" && "TunnelStateChanged" in event)
    ) {
      void refresh();
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
  <section class="tunnel-panel">
    <header v-if="!props.embedded">
      <h3>端口转发 ({{ items.length }})</h3>
      <el-button size="small" :loading="loading" @click="refresh">刷新</el-button>
    </header>
    <p v-if="error" class="error">{{ error }}</p>
    <p v-for="message in unsupported" :key="message" class="error">
      旧隧道规则已跳过：{{ message }}
    </p>

    <el-form inline size="small" class="add-form" @submit.prevent="add">
      <el-form-item label="类型">
        <el-select v-model="draftType" style="width: 110px" data-test="tunnel-type">
          <el-option label="Local" value="Local" />
          <el-option label="Dynamic" value="Dynamic" />
        </el-select>
      </el-form-item>
      <el-form-item label="连接">
        <el-select
          v-model="draftSession"
          placeholder="选择连接信息"
          style="width: 180px"
          filterable
          data-test="tunnel-session"
        >
          <el-option
            v-for="session in selectableSessions"
            :key="session.id"
            :label="session.name"
            :value="session.id"
          />
        </el-select>
      </el-form-item>
      <el-form-item label="监听">
        <el-input v-model="draftBind" placeholder="host:port" style="width: 140px" data-test="tunnel-bind" />
        <p v-if="bindError" class="error" role="alert" data-test="tunnel-bind-error">{{ bindError }}</p>
      </el-form-item>
      <el-form-item v-if="draftType !== 'Dynamic'" label="目标">
        <el-input v-model="draftTarget" placeholder="host:port" style="width: 140px" data-test="tunnel-target" />
      </el-form-item>
      <el-form-item>
        <el-button type="primary" native-type="submit" data-test="tunnel-add">新建</el-button>
      </el-form-item>
    </el-form>

    <p v-if="nonLoopbackBind" class="warn">
      监听地址不是回环地址：创建时将要求二次确认，隧道将暴露给局域网（SOCKS5 代理无认证）。
    </p>

    <section v-if="pending.length" class="pending-block" data-test="tunnel-pending">
      <h4>待恢复的隧道 ({{ pending.length }})</h4>
      <p class="hint">
        应用重启后不自动抢占端口，这些规则来自上次的记录。恢复需要对应连接已建立 SSH 连接。
      </p>
      <ul class="pending-list">
        <li v-for="[sessionId, rule] in pending" :key="`${sessionId}-${rule.bind_address}-${rule.bind_port}`">
          <span class="pending-label">{{ ruleLabel(rule) }}</span>
          <span class="pending-session">{{ sessionName(sessionId) }}</span>
          <el-button
            size="small"
            :loading="pendingIds.has(sessionId)"
            data-test="tunnel-restore"
            @click="restore(sessionId, rule)"
          >
            恢复
          </el-button>
        </li>
      </ul>
    </section>

    <el-empty v-if="items.length === 0" description="暂无隧道" />
    <el-table v-else :data="items" stripe size="small">
      <el-table-column prop="id" label="Tunnel ID" width="120" />
      <el-table-column label="状态" min-width="120">
        <template #default="{ row }">
          {{ stateLabel(row.state) }}
        </template>
      </el-table-column>
      <el-table-column label="操作" width="190">
        <template #default="{ row }">
          <el-button
            size="small"
            :disabled="!canSuspend(row.state) || pendingIds.has(row.id)"
            data-test="tunnel-suspend"
            @click="suspend(row.id)"
          >
            挂起
          </el-button>
          <el-button
            size="small"
            :disabled="!canResume(row.state) || pendingIds.has(row.id)"
            data-test="tunnel-resume"
            @click="resume(row.id)"
          >
            恢复
          </el-button>
          <el-button size="small" type="danger" data-test="tunnel-close" @click="remove(row.id)">关闭</el-button>
        </template>
      </el-table-column>
    </el-table>
  </section>
</template>

<style scoped>
.tunnel-panel {
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
/* 字段级错误独占一行：el-form-item 内容是行内排版，红字跟在输入框右侧会挤掉标签 */
.add-form .error {
  display: block;
  margin: 2px 0 0;
}
.warn {
  color: var(--el-color-warning);
  font-size: 12px;
}
.pending-block {
  margin-bottom: 14px;
  padding: 8px 10px;
  border: 1px solid var(--rs-border);
  border-radius: 6px;
  background: var(--rs-bg-surface);
}
.pending-block h4 {
  margin: 0 0 4px;
  font-size: 12px;
}
.pending-block .hint {
  margin: 0 0 8px;
  font-size: 11px;
  color: var(--rs-fg-muted);
  line-height: 1.5;
}
.pending-list {
  list-style: none;
  margin: 0;
  padding: 0;
}
.pending-list li {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 0;
  font-size: 12px;
}
.pending-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pending-session {
  color: var(--rs-fg-muted);
  font-size: 11px;
}
</style>
