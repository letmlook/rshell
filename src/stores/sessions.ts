/**
 * sessions pinia store —— 切片 1.3 雏形
 *
 * 持有会话列表、当前选中会话 id、连接状态映射。
 * 切片 1.3 仅承载切片 1 必须的状态;其余(传输队列、密钥等)在切片 3+ 单独建 store。
 */
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import type { SessionConfig, Uuid } from "../ipc/types";
import {
  listSessions,
  createSession,
  connectSession,
  disconnectSession,
  deleteSession,
} from "../ipc/client";
import { listen } from "@tauri-apps/api/event";

type ConnectionStateValue = "disconnected" | "connecting" | "connected" | "failed";

export const useSessionsStore = defineStore("sessions", () => {
  const items = ref<SessionConfig[]>([]);
  const currentId = ref<Uuid | null>(null);
  const connectionState = ref<Map<Uuid, ConnectionStateValue>>(new Map());
  const searchKeyword = ref(""); // 切片 3：会话列表过滤词（设计 §5）
  const masterPasswordRequired = ref(false); // 切片 6：监听 MasterPasswordRequired 事件
  const loading = ref(false);
  const error = ref<string | null>(null);

  const current = computed<SessionConfig | null>(() =>
    currentId.value ? items.value.find((s) => s.id === currentId.value) ?? null : null,
  );

  async function refresh() {
    loading.value = true;
    error.value = null;
    try {
      items.value = await listSessions();
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  async function create(cfg: SessionConfig) {
    const id = await createSession(cfg);
    await refresh();
    return id;
  }

  async function connect(id: Uuid) {
    connectionState.value.set(id, "connecting");
    connectionState.value = new Map(connectionState.value); // trigger reactivity
    try {
      await connectSession(id);
    } catch (e) {
      connectionState.value.set(id, "failed");
      connectionState.value = new Map(connectionState.value);
      throw e;
    }
  }

  async function disconnect(id: Uuid) {
    await disconnectSession(id);
    connectionState.value.set(id, "disconnected");
    connectionState.value = new Map(connectionState.value);
  }

  async function deleteSessionById(id: Uuid) {
    await deleteSession(id);
    await refresh();
  }

  /** 订阅后端事件总线,实时更新 connectionState(设计 §4.3 流程 A)。*/
  async function subscribeEvents() {
    await listen<{ kind: string; session_id?: Uuid; state?: string }>(
      "rshell://event",
      (e) => {
        const payload = e.payload;
        if (
          payload.kind === "ConnectionStateChanged" &&
          payload.session_id &&
          payload.state
        ) {
          const normalized = payload.state.toLowerCase() as ConnectionStateValue;
          connectionState.value.set(payload.session_id, normalized);
          connectionState.value = new Map(connectionState.value);
        }
      },
    );
  }

  return {
    items,
    currentId,
    current,
    connectionState,
    searchKeyword,
    masterPasswordRequired,
    loading,
    error,
    refresh,
    create,
    connect,
    disconnect,
    delete: deleteSessionById,
    subscribeEvents,
  };
});