/**
 * 终端偏好：选中即复制。
 *
 * 存 `localStorage` 而非后端：这是纯 UI 偏好（不是凭据、不是会话数据），
 * 与 lastLocalDir 同理，走 IPC 往返只会让首屏多一次阻塞等待。
 * 存储不可用（隐私模式、配额满、WebView 禁用）时降级为「进程内有效」，
 * 绝不抛错打断终端。
 *
 * 响应式导出而非只读函数：设置面板改开关时，已经打开的终端要**立即**
 * 生效，不必等面板重挂。只在读取时查一次 localStorage 的写法做不到这点。
 */
import { ref } from "vue";

export const COPY_ON_SELECT_KEY = "rshell.terminal.copyOnSelect.v1";

/** 存储里只认 "true"/"false"；缺失、损坏、非字符串一律回落到默认值 */
export function normalizeCopyOnSelect(raw: unknown, fallback = true): boolean {
  if (raw === "true") return true;
  if (raw === "false") return false;
  return fallback;
}

/** 从 localStorage 读取「选中即复制」；无记录或读取失败时返回默认值 */
export function loadCopyOnSelect(
  storage: Pick<Storage, "getItem"> | null | undefined,
  fallback = true,
): boolean {
  if (!storage) return fallback;
  try {
    return normalizeCopyOnSelect(storage.getItem(COPY_ON_SELECT_KEY), fallback);
  } catch {
    return fallback;
  }
}

/** 写入「选中即复制」；返回是否真的落盘（配额满时为 false，界面仍应显示新状态） */
export function saveCopyOnSelect(
  storage: Pick<Storage, "setItem"> | null | undefined,
  value: boolean,
): boolean {
  if (!storage) return false;
  try {
    storage.setItem(COPY_ON_SELECT_KEY, value ? "true" : "false");
    return true;
  } catch {
    return false;
  }
}

/** localStorage 可能不存在（SSR / 测试环境），统一走这个取值口 */
export function defaultLocalStorage(): Pick<Storage, "getItem" | "setItem"> | null {
  return typeof localStorage === "undefined" ? null : localStorage;
}

/**
 * 当前生效的「选中即复制」。设置面板写它，终端面板 watch 它。
 * 模块级单例：与 dialog/transferLog 同一套取舍，跨组件共享一份偏好。
 */
export const copyOnSelect = ref(loadCopyOnSelect(defaultLocalStorage()));

/** 改开关：更新内存态并落盘；落盘失败只说明「不记住」，不影响本次运行 */
export function setCopyOnSelect(value: boolean): boolean {
  copyOnSelect.value = value;
  return saveCopyOnSelect(defaultLocalStorage(), value);
}
