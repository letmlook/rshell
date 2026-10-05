/**
 * sessions pinia store —— 切片 1.3 雏形
 *
 * 持有会话列表、当前选中会话 id、连接状态映射。
 * 切片 1.3 仅承载切片 1 必须的状态;其余(传输队列、密钥等)在切片 3+ 单独建 store。
 */
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import type { SessionConfig, SessionCredential, SessionLoadIssue, Uuid } from "../ipc/types";
import {
  listSessions,
  createSession,
  connectSession,
  disconnectSession,
  deleteSession,
  listSessionLoadIssues,
  retrySessionLoad,
  updateSession,
} from "../ipc/client";
import { subscribeAppEvents } from "../ipc/events";
import { newUuid } from "../utils/uuid";

type ConnectionStateValue = "disconnected" | "connecting" | "connected" | "failed";

export const useSessionsStore = defineStore("sessions", () => {
  const items = ref<SessionConfig[]>([]);
  const currentId = ref<Uuid | null>(null);
  const connectionState = ref<Map<Uuid, ConnectionStateValue>>(new Map());
  const searchKeyword = ref(""); // 切片 3：会话列表过滤词（设计 §5）
  const loading = ref(false);
  const error = ref<string | null>(null);
  const loadIssues = ref<SessionLoadIssue[]>([]);
  const retryingLoad = ref(false);
  let unlisten: (() => void) | null = null;
  let subscriptionGeneration = 0;

  const current = computed<SessionConfig | null>(() =>
    currentId.value ? items.value.find((s) => s.id === currentId.value) ?? null : null,
  );

  async function refresh() {
    loading.value = true;
    error.value = null;
    try {
      const [sessions, issues] = await Promise.all([listSessions(), listSessionLoadIssues()]);
      items.value = sessions;
      loadIssues.value = issues;
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  async function create(cfg: SessionConfig, credential: SessionCredential | null) {
    const id = await createSession(cfg, credential);
    await refresh();
    return id;
  }

  async function retryLoad() {
    retryingLoad.value = true;
    try {
      await retrySessionLoad();
      await refresh();
    } catch (e) {
      error.value = String(e);
    } finally {
      retryingLoad.value = false;
    }
  }

  async function updateCredential(id: Uuid, credential: SessionCredential) {
    const config = items.value.find((session) => session.id === id);
    if (!config) throw new Error("会话不存在，请刷新后重试");
    await updateSession(id, config, { Set: credential });
    await refresh();
  }

  /**
   * 新会话 ID。后端 CreateSession 直接采用 `config.id`（重复会被拒为
   * "Session already exists"），所以复制会话必须在前端生成新 id，
   * 否则会覆盖原会话。
   *
   * 拿不到安全随机源时抛错，不退回时间戳/计数器——那样的 id 可预测，
   * 且会和真实会话撞上。
   */
  function newSessionId(): Uuid {
    return newUuid() as Uuid;
  }

  /** 在已用名称后追加序号，保证复制出来的会话名不撞车 */
  function uniqueSessionName(base: string): string {
    const taken = new Set(items.value.map((session) => session.name));
    if (!taken.has(base)) return base;
    for (let i = 2; i < 1000; i += 1) {
      const candidate = `${base} ${i}`;
      if (!taken.has(candidate)) return candidate;
    }
    throw new Error(`名称 ${base} 的副本已达上限，请先重命名或删除部分会话`);
  }

  /**
   * 复制会话：同一份连接信息（主机/端口/用户/协议/认证方式/分组）建一个新条目。
   *
   * 凭据**不复制**：密码与私钥口令存在系统钥匙串，接口不回读明文，
   * 也没有后端复制命令。这里传 `null` 凭据建会话，由前端在创建后弹凭据
   * 对话框让用户为新条目补录一次——界面据此提示，不假装已经能连。
   * 用密钥文件的会话（PublicKey + 无口令）无需补录，可直接使用。
   */
  async function duplicate(id: Uuid): Promise<Uuid> {
    const source = items.value.find((session) => session.id === id);
    if (!source) throw new Error("会话不存在，请刷新后重试");
    const copy: SessionConfig = {
      ...source,
      id: newSessionId(),
      name: uniqueSessionName(`${source.name} 副本`),
    };
    return create(copy, null);
  }

  /**
   * R3-05：同一会话的并发 connect 复用同一个 in-flight Promise。
   *
   * 旧实现没有去重，两个调用方各自发一次 `connect_session`。更麻烦的是调用方
   * 只能靠 `connectionState === "connecting"` 猜测连接是否在进行，于是
   * `openTabSession` 会选择「跳过等待、直接建面板」——那时后端还没把会话放进
   * `connections`，`open_terminal` 必然 NotFound，标签页就此空白（见 App.vue）。
   * 让 connect 可等待，调用方就不必再靠状态猜。
   */
  const inflightConnects = new Map<Uuid, Promise<void>>();

  async function connect(id: Uuid) {
    const existing = inflightConnects.get(id);
    if (existing) return existing;

    connectionState.value.set(id, "connecting");
    connectionState.value = new Map(connectionState.value); // trigger reactivity

    const task = (async () => {
      try {
        await connectSession(id);
      } catch (e) {
        connectionState.value.set(id, "failed");
        connectionState.value = new Map(connectionState.value);
        throw e;
      } finally {
        inflightConnects.delete(id);
      }
    })();
    inflightConnects.set(id, task);
    return task;
  }

  /** 会话是否正在握手中（含本进程发起的连接） */
  function isConnecting(id: Uuid): boolean {
    return connectionState.value.get(id) === "connecting" || inflightConnects.has(id);
  }

  async function disconnect(id: Uuid) {
    try {
      await disconnectSession(id);
    } catch (e) {
      error.value = String(e);
      throw e;
    }
    error.value = null;
    connectionState.value.set(id, "disconnected");
    connectionState.value = new Map(connectionState.value);
  }

  async function deleteSessionById(id: Uuid) {
    try {
      await deleteSession(id);
    } catch (e) {
      error.value = String(e);
      throw e;
    }
    error.value = null;
    await refresh();
  }

  /** 订阅后端事件总线,实时更新 connectionState(设计 §4.3 流程 A)。*/
  async function subscribeEvents() {
    if (unlisten) return;
    const generation = ++subscriptionGeneration;
    const stop = await subscribeAppEvents((event) => {
      if (typeof event !== "string" && "ConnectionStateChanged" in event) {
        const payload = event.ConnectionStateChanged;
        const normalized = payload.state.toLowerCase() as ConnectionStateValue;
        connectionState.value.set(payload.session_id, normalized);
        connectionState.value = new Map(connectionState.value);
      } else if (event === "SessionListChanged") {
        void refresh();
      }
    });
    if (generation === subscriptionGeneration) unlisten = stop;
    else stop();
  }

  function disposeEvents() { subscriptionGeneration++; unlisten?.(); unlisten = null; }

  return {
    items,
    currentId,
    current,
    connectionState,
    searchKeyword,
    loading,
    error,
    loadIssues,
    retryingLoad,
    retryLoad,
    updateCredential,
    duplicate,
    refresh,
    create,
    connect,
    isConnecting,
    disconnect,
    delete: deleteSessionById,
    subscribeEvents,
    disposeEvents,
  };
});
