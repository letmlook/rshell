/**
 * 本地路径选择服务 —— 替掉 `tauri-plugin-dialog` 的 `open()`。
 *
 * 原生 `open()` 弹的是操作系统选择窗口，与应用外观割裂；且它的授权语义
 * （点选目录才写入 runtime scope）容易和「已记忆目录」冲突。改为应用内
 * 选择器后统一走已声明的 `fs:read-all`，外观与行为都可控。
 *
 * 与 dialog.ts 同样的结构：模块级请求槽 + Promise，渲染交给
 * `components/dialogs/PathPickerHost.vue`（在 App.vue 挂一次）。
 * 只有「一个选择窗」是合理语义，因此这里不做队列——后发的请求会顶掉前一个，
 * 调用方不应并发调 pickLocalPath。
 */
import { ref, type Ref } from "vue";

export interface PathPickerRequest {
  mode: "directory" | "file";
  title: string;
  /** 初始目录；不给则用用户主目录 */
  initialPath?: string;
  /** file 模式下的文件过滤；返回 false 的条目不列出 */
  accept?: (name: string) => boolean;
  /** 目录模式：是否提供「新建文件夹」 */
  allowCreateDirectory?: boolean;
}

const request = ref<PathPickerRequest | null>(null);
/** 当前选择请求；宿主与测试都直接读它 */
export { request };
let pending: ((value: string | null) => void) | null = null;

/** 打开选择窗；选中返回绝对路径，取消/关闭返回 null */
export function pickLocalPath(options: PathPickerRequest): Promise<string | null> {
  return new Promise<string | null>((resolve) => {
    pending = resolve;
    request.value = options;
  });
}

/** 由宿主在用户确认/取消时调用 */
export function resolvePathPicker(path: string | null): void {
  request.value = null;
  const resolve = pending;
  pending = null;
  resolve?.(path);
}

export function usePathPicker(): {
  request: Ref<PathPickerRequest | null>;
  pick: typeof pickLocalPath;
  resolve: typeof resolvePathPicker;
} {
  return { request, pick: pickLocalPath, resolve: resolvePathPicker };
}

/** 仅测试用 */
export function resetPathPicker(): void {
  request.value = null;
  pending = null;
}
