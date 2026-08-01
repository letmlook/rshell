/**
 * hostKey pinia store —— 切片 4
 *
 * 持有当前活跃的 HostKeyMismatch 请求 + 决策动作。
 * 后端经 `EventBus` → `app.emit("rshell://event")` 推 `HostKeyMismatch` 事件;
 * 本 store 监听后弹对话框,用户决策后调 `decideHostKey(decision_id, accept, permanent)`。
 */
import { defineStore } from "pinia";
import { ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { decideHostKey } from "../ipc/client";

export interface HostKeyRequest {
  decision_id: string;
  host: string;
  port: number;
  key_type: string;
  expected: string;
  received: string;
  public_key_blob: string;
}

export const useHostKeyStore = defineStore("hostKey", () => {
  const current = ref<HostKeyRequest | null>(null);
  const history = ref<HostKeyRequest[]>([]); // 已处理但留作审计

  async function subscribeEvents() {
    await listen<HostKeyRequest>("rshell://event", (msg) => {
      const payload = msg.payload;
      if (payload && "decision_id" in payload && "received" in payload) {
        current.value = payload;
      }
    });
  }

  async function trustOnce() {
    if (!current.value) return;
    const req = current.value;
    history.value.push(req);
    current.value = null;
    await decideHostKey(req.decision_id, true, false);
  }

  async function trustPermanent() {
    if (!current.value) return;
    const req = current.value;
    history.value.push(req);
    current.value = null;
    await decideHostKey(req.decision_id, true, true);
  }

  async function reject() {
    if (!current.value) return;
    const req = current.value;
    history.value.push(req);
    current.value = null;
    await decideHostKey(req.decision_id, false, false);
  }

  function dismiss() {
    current.value = null;
  }

  return { current, history, subscribeEvents, trustOnce, trustPermanent, reject, dismiss };
});