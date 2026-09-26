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

/** 全零 decision_id：后端发出的不可决策告警 */
const NIL_DECISION_ID = "00000000-0000-0000-0000-000000000000";

export const useHostKeyStore = defineStore("hostKey", () => {
  const current = ref<HostKeyRequest | null>(null);
  const history = ref<HostKeyRequest[]>([]); // 已处理但留作审计
  /** 最近一次决策提交失败的原因；非空时由对话框就地展示，对话框保持打开 */
  const error = ref<string | null>(null);
  let unlisten: (() => void) | null = null;
  let subscriptionGeneration = 0;

  async function subscribeEvents() {
    if (unlisten) return;
    const generation = ++subscriptionGeneration;
    const stop = await subscribeAppEvents((event) => {
      if (typeof event === "string" || !("HostKeyMismatch" in event)) return;
      const request = event.HostKeyMismatch;
      // nil decision_id 表示「已知密钥指纹变化」的单向告警，没有可回写的
      // 决策通道，弹决策框只会让按钮全部失败 —— 这里显式忽略。
      if (request.decision_id === NIL_DECISION_ID) return;
      error.value = null;
      current.value = request;
    });
    if (generation === subscriptionGeneration) unlisten = stop;
    else stop();
  }

  function disposeEvents() { subscriptionGeneration++; unlisten?.(); unlisten = null; }

  /** 提交决策；成功才收起对话框，失败保留对话框并记录错误供界面展示。 */
  async function submit(decision_id: string, accept: boolean, permanent: boolean) {
    error.value = null;
    try {
      await decideHostKey(decision_id, accept, permanent);
    } catch (e) {
      error.value = String(e);
      return;
    }
    const req = current.value;
    if (req) history.value.push(req);
    current.value = null;
  }

  async function trustOnce() {
    if (current.value) await submit(current.value.decision_id, true, false);
  }

  async function trustPermanent() {
    if (current.value) await submit(current.value.decision_id, true, true);
  }

  async function reject() {
    if (current.value) await submit(current.value.decision_id, false, false);
  }

  function dismiss() {
    current.value = null;
    error.value = null;
  }

  return {
    current,
    history,
    error,
    subscribeEvents,
    disposeEvents,
    trustOnce,
    trustPermanent,
    reject,
    dismiss,
  };
});
