/**
 * Keyed host-key decision store.
 *
 * A backend handshake is identified by decision_id. Requests are kept in a
 * map so deciding/cancelling one cannot overwrite or dismiss another.
 */
import { defineStore } from "pinia";
import { ref } from "vue";
import { subscribeAppEvents } from "../ipc/events";
import { cancelHostKey, decideHostKey } from "../ipc/client";

export interface HostKeyRequest {
  decision_id: string;
  host: string;
  port: number;
  key_type: string;
  expected: string;
  received: string;
  public_key_blob: string;
}

const NIL_DECISION_ID = "00000000-0000-0000-0000-000000000000";
const TERMINAL_STATES = new Set([
  "cancelled",
  "expired",
  "dismissed",
]);

export const useHostKeyStore = defineStore("hostKey", () => {
  const requests = ref<Map<string, HostKeyRequest>>(new Map());
  // `current` is retained as the dialog's selected request for compatibility;
  // all lifecycle mutations still dispatch by its explicit id.
  const current = ref<HostKeyRequest | null>(null);
  const history = ref<HostKeyRequest[]>([]);
  const error = ref<string | null>(null);
  const errorDecisionId = ref<string | null>(null);
  const decisionErrors = ref<Map<string, string>>(new Map());
  let unlisten: (() => void) | null = null;
  let subscriptionGeneration = 0;

  function syncCurrent() {
    current.value = requests.value.values().next().value ?? null;
  }

  function removeRequest(id: string, recordHistory = true) {
    const request = requests.value.get(id);
    if (!request) return false;
    requests.value.delete(id);
    requests.value = new Map(requests.value);
    if (recordHistory) history.value.push(request);
    if (errorDecisionId.value === id) {
      errorDecisionId.value = null;
      error.value = null;
    }
    if (current.value?.decision_id === id) syncCurrent();
    return true;
  }

  async function subscribeEvents() {
    if (unlisten) return;
    const generation = ++subscriptionGeneration;
    const stop = await subscribeAppEvents((event) => {
      if (typeof event === "string") return;
      if ("HostKeyMismatch" in event) {
        const request = event.HostKeyMismatch;
        if (request.decision_id === NIL_DECISION_ID) return;
        // Duplicate/late events for an id already decided are ignored.
        if (requests.value.has(request.decision_id)) return;
        requests.value.set(request.decision_id, request);
        requests.value = new Map(requests.value);
        decisionErrors.value.delete(request.decision_id);
        decisionErrors.value = new Map(decisionErrors.value);
        error.value = null;
        errorDecisionId.value = null;
        syncCurrent();
        return;
      }
      if ("HostKeyDecisionStateChanged" in event) {
        const { decision_id, state } = event.HostKeyDecisionStateChanged;
        // Unknown/stale ids are deliberately harmless and cannot clear a
        // different valid request.
        if (!requests.value.has(decision_id)) return;
        if (TERMINAL_STATES.has(state.toLowerCase())) removeRequest(decision_id);
      }
    });
    if (generation === subscriptionGeneration) unlisten = stop;
    else stop();
  }

  function disposeEvents() {
    subscriptionGeneration++;
    unlisten?.();
    unlisten = null;
  }

  function setError(id: string, value: unknown) {
    const message = value instanceof Error ? value.message : String(value);
    error.value = message;
    errorDecisionId.value = id;
    decisionErrors.value.set(id, message);
    decisionErrors.value = new Map(decisionErrors.value);
  }

  /** Submit an explicit decision; successful settlement removes only this id. */
  async function submit(decision_id: string, accept: boolean, permanent: boolean) {
    error.value = null;
    errorDecisionId.value = null;
    decisionErrors.value.delete(decision_id);
    decisionErrors.value = new Map(decisionErrors.value);
    try {
      await decideHostKey(decision_id, accept, permanent);
    } catch (e) {
      setError(decision_id, e);
      return false;
    }
    if (!removeRequest(decision_id)) {
      // This also supports the legacy dialog test seam where `current` was
      // assigned directly rather than through a mismatch event.
      if (current.value?.decision_id === decision_id) current.value = null;
    }
    if (!current.value) error.value = null;
    return true;
  }

  async function trustOnce(id = current.value?.decision_id) {
    if (id) return submit(id, true, false);
    return false;
  }

  async function trustPermanent(id = current.value?.decision_id) {
    if (id) return submit(id, true, true);
    return false;
  }

  async function reject(id = current.value?.decision_id) {
    if (id) return submit(id, false, false);
    return false;
  }

  /** Explicit cancellation never submits an accepting decision. */
  async function cancel(id = current.value?.decision_id) {
    if (!id) return false;
    // R3-T3 reviewer (Minor UX bug): if the request has already been removed
    // from the store (e.g. user double-clicks cancel, or backend already
    // dropped it via Expired / Cancelled event), the user's intent — cancel —
    // is already satisfied. Skip dispatch so the second call cannot overwrite
    // the cleared error state with a misleading "decision no longer pending"
    // message from a NotFound response.
    if (!requests.value.has(id)) {
      if (errorDecisionId.value === id) {
        error.value = null;
        errorDecisionId.value = null;
      }
      return true;
    }
    error.value = null;
    errorDecisionId.value = null;
    try {
      await cancelHostKey(id);
    } catch (e) {
      setError(id, e);
      return false;
    }
    removeRequest(id);
    if (!current.value) error.value = null;
    return true;
  }

  /** Dismiss a stale dialog locally; cancellation is explicit and keyed. */
  async function dismiss(id = current.value?.decision_id) {
    return cancel(id);
  }

  return {
    requests,
    current,
    history,
    error,
    decisionErrors,
    subscribeEvents,
    disposeEvents,
    submit,
    trustOnce,
    trustPermanent,
    reject,
    cancel,
    dismiss,
  };
});
