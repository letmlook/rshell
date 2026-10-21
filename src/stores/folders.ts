/**
 * folders pinia store —— 连接分组
 *
 * 分组是「连接信息」这个可复用配方的一部分（见 docs/08 的术语定义）：
 * 一条连接通过 `SessionConfig::folder_id` 指向所属分组，`None` 表示「默认」。
 *
 * 后端此前只有悬空的 `folder_id` 字段，没有任何分组实体——界面上的分组树
 * 因此永远只有一个「默认」组。这里接上真正的分组 CRUD。
 */
import { defineStore } from "pinia";
import { computed, ref } from "vue";
import {
  createFolder,
  deleteFolder,
  listFolders,
  renameFolder,
} from "../ipc/client";
import { ipcErrorMessage } from "../ipc/client";
import { subscribeAppEvents } from "../ipc/events";
import type { Folder, Uuid } from "../ipc/types";

/** 未归组的连接所在的「默认」分组名 */
export const DEFAULT_FOLDER_LABEL = "默认";

export const useFoldersStore = defineStore("folders", () => {
  const items = ref<Folder[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

  let unlisten: (() => void) | null = null;
  let subscriptionGeneration = 0;

  const byId = computed(() => new Map(items.value.map((folder) => [folder.id, folder])));

  async function refresh() {
    loading.value = true;
    try {
      const list = await listFolders();
      // 后端异常返回（null / 非数组）不能把 store 变成不可迭代 —— 界面会整块崩掉
      items.value = Array.isArray(list) ? list : [];
      error.value = null;
    } catch (e) {
      error.value = ipcErrorMessage(e);
    } finally {
      loading.value = false;
    }
  }

  /** 新建分组，返回新分组 id；失败返回 null（原因写进 error） */
  async function create(name: string): Promise<Uuid | null> {
    const trimmed = name.trim();
    if (!trimmed) {
      error.value = "分组名不能为空";
      return null;
    }
    try {
      const id = await createFolder(trimmed);
      error.value = null;
      await refresh();
      return id;
    } catch (e) {
      error.value = ipcErrorMessage(e);
      return null;
    }
  }

  async function rename(folder_id: Uuid, name: string) {
    try {
      await renameFolder(folder_id, name.trim());
      error.value = null;
      await refresh();
      return true;
    } catch (e) {
      error.value = ipcErrorMessage(e);
      return false;
    }
  }

  /** 删除分组：组内的连接回到「默认」，不会被删除 */
  async function remove(folder_id: Uuid) {
    try {
      await deleteFolder(folder_id);
      error.value = null;
      await refresh();
      return true;
    } catch (e) {
      error.value = ipcErrorMessage(e);
      return false;
    }
  }

  /** 分组名；`folder_id` 为空或指向已删除的分组时返回「默认」 */
  function nameOf(folder_id: Uuid | null | undefined): string {
    if (!folder_id) return DEFAULT_FOLDER_LABEL;
    return byId.value.get(folder_id)?.name ?? DEFAULT_FOLDER_LABEL;
  }

  async function subscribeEvents() {
    if (unlisten) return;
    const generation = ++subscriptionGeneration;
    const stop = await subscribeAppEvents((event) => {
      if (event === "FolderListChanged") void refresh();
    });
    if (generation === subscriptionGeneration) unlisten = stop;
    else stop();
  }

  function disposeEvents() {
    subscriptionGeneration++;
    unlisten?.();
    unlisten = null;
  }

  return {
    items,
    loading,
    error,
    byId,
    refresh,
    create,
    rename,
    remove,
    nameOf,
    subscribeEvents,
    disposeEvents,
  };
});
