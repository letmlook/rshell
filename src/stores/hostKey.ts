/**
 * hostKey pinia store —— 切片 4
 *
 * 持有当前活跃的 HostKeyMismatch 请求 + 决策动作。
 * 后端经 `EventBus` → `app.emit("rshell://event")` 推 `AppEvent`:
 * 由于 `AppEvent` 是 serde 默认 external-tagged enum,前端拿到的 payload 是
 * `{"HostKeyMismatch": {decision_id, host, ...}}` 形态 —— 必须经
 * `makeDispatcher` 解包才能拿到内层字段,不能直接在 listener 里读
 * `payload.decision_id`。用户决策后调 `decideHostKey(decision_id, accept, permanent)`。
 */
import { defineStore } from "pinia";
import { ref } from "vue";
import { makeDispatcher, subscribeAppEvents, type UnlistenFn } from "../ipc/events";
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

  let unlisten: UnlistenFn | null = null;

  async function subscribeEvents(): Promise<UnlistenFn> {
    if (unlisten) return unlisten;
    unlisten = await subscribeAppEvents(
      makeDispatcher({
        onHostKeyMismatch: (data) => {
          current.value = data as HostKeyRequest;
        },
      }),
    );
    return unlisten;
  }

  function unsubscribeEvents() {
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
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

  return {
    current,
    history,
    subscribeEvents,
    unsubscribeEvents,
    trustOnce,
    trustPermanent,
    reject,
    dismiss,
  };
});