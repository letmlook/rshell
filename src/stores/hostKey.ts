/**
 * hostKey pinia store —— 切片 4
 *
 * 持有当前活跃的 HostKeyMismatch 请求 + 决策动作。
 * 后端经 `EventBus` → `app.emit("rshell://event")` 推 `HostKeyMismatch` 事件;
 * 本 store 监听后弹对话框,用户决策后调 `decideHostKey(decision_id, accept, permanent)`。
 */
import { defineStore } from "pinia";
import { ref } from "vue";
import { subscribeAppEvents } from "../ipc/events";
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
  let unlisten: (() => void) | null = null;
  let subscriptionGeneration = 0;

  async function subscribeEvents() {
    if (unlisten) return;
    const generation = ++subscriptionGeneration;
    const stop = await subscribeAppEvents((event) => {
      if (typeof event !== "string" && "HostKeyMismatch" in event) current.value = event.HostKeyMismatch;
    });
    if (generation === subscriptionGeneration) unlisten = stop;
    else stop();
  }

  function disposeEvents() { subscriptionGeneration++; unlisten?.(); unlisten = null; }

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

  return { current, history, subscribeEvents, disposeEvents, trustOnce, trustPermanent, reject, dismiss };
});
