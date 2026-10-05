<script setup lang="ts">
/**
 * TunnelPanel —— 切片 8
 *
 * 本地端口转发与动态 SOCKS5 隧道管理。
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { listTunnels, listPendingTunnels, createTunnel, closeTunnel } from "../ipc/client";
import { subscribeAppEvents } from "../ipc/events";
import type { Uuid, PortForwardRule, ActiveTunnelInfo } from "../ipc/types";
import { confirmDialog } from "../utils/dialog";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const items = ref<ActiveTunnelInfo[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);
const unsupported = ref<string[]>([]);

// 事件订阅的生命周期（与 QuickCommandPanel 同款）：异步订阅完成可能晚于
// unmount，用 active 门闩保证晚到的订阅被立即释放。
let unlisten: (() => void) | null = null;
let active = false;

const draftType = ref<"Local" | "Dynamic">("Local");
const draftSession = ref<Uuid | null>(null);
const draftBind = ref("127.0.0.1:8080");
const draftTarget = ref("localhost:80");

async function refresh() {
  loading.value = true;
  try {
    const [active, pending] = await Promise.all([listTunnels(), listPendingTunnels()]);
    items.value = active;
    unsupported.value = pending.unsupported.map(
      (rule) => `${rule.session_id}: ${rule.reason}`,
    );
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
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
    await refresh();
  } catch (e) {
    error.value = String(e);
  }
}

async function remove(id: Uuid) {
  try {
    await closeTunnel(id);
    await refresh();
  } catch (e) {
    error.value = String(e);
  }
}

// R2-04：后端在会话断开时会把该会话隧道置 Error 并逐条发布
// TunnelStateChanged + ActiveTunnelsChanged（tunnel_manager deactivate_session_tunnels）。
// 面板打开期间不订阅这两个事件，状态列会一直停留旧的 Active 文案，直到手点刷新。
onMounted(async () => {
  active = true;
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
        <el-select v-model="draftType" style="width: 110px">
          <el-option label="Local" value="Local" />
          <el-option label="Dynamic" value="Dynamic" />
        </el-select>
      </el-form-item>
      <el-form-item label="会话 ID">
        <el-input v-model="draftSession" placeholder="uuid" style="width: 180px" />
      </el-form-item>
      <el-form-item label="监听">
        <el-input v-model="draftBind" placeholder="host:port" style="width: 140px" />
        <p v-if="bindError" class="error" role="alert" data-test="tunnel-bind-error">{{ bindError }}</p>
      </el-form-item>
      <el-form-item v-if="draftType !== 'Dynamic'" label="目标">
        <el-input v-model="draftTarget" placeholder="host:port" style="width: 140px" />
      </el-form-item>
      <el-form-item>
        <el-button type="primary" native-type="submit">新建</el-button>
      </el-form-item>
    </el-form>

    <p v-if="nonLoopbackBind" class="warn">
      监听地址不是回环地址：创建时将要求二次确认，隧道将暴露给局域网（SOCKS5 代理无认证）。
    </p>
    <el-empty v-if="items.length === 0" description="暂无隧道" />
    <el-table v-else :data="items" stripe size="small">
      <el-table-column prop="id" label="Tunnel ID" width="120" />
      <el-table-column label="状态" width="100">
        <template #default="{ row }">
          {{ typeof row.state === "string" ? row.state : Object.keys(row.state || {})[0] || "—" }}
        </template>
      </el-table-column>
      <el-table-column label="操作" width="80">
        <template #default="{ row }">
          <el-button size="small" type="danger" @click="remove(row.id)">关闭</el-button>
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
</style>
