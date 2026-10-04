/**
 * 本地目录记忆：记住传输工作区上次打开的目录，下次进入时自动恢复。
 *
 * 存 `localStorage` 而非后端持久化：这是纯 UI 偏好（不是凭据、不是会话数据），
 * 没有跨进程/跨设备一致性要求，走 IPC 往返只会让首屏多一次阻塞等待。
 * 存储不可用（隐私模式、配额满、WebView 禁用）时全部降级为「不记忆」，
 * 绝不抛错打断界面。
 */

/** 存储键；带版本号便于将来改结构时自然失效旧值。 */
export const LAST_LOCAL_DIR_KEY = "rshell.transfer.lastLocalDir.v1";

/**
 * 归一化并校验待持久化的目录。
 *
 * 只接受绝对路径：相对路径在不同工作目录下含义不同，恢复时会指向错误位置。
 * 去掉尾部多余分隔符（根目录 `/` 除外），避免 `C:\foo\` 与 `C:\foo` 存成两条。
 * 空串与纯空白视为无效（等于没选过目录）。
 */
export function normalizeLocalDir(path: string | null | undefined): string | null {
  const raw = (path ?? "").trim();
  if (!raw) return null;
  // Windows 盘符（`C:\`、`C:/`）与 UNC 开头（`\\server\share`）也算绝对路径
  const isAbsolute = raw.startsWith("/") || raw.startsWith("\\") || /^[A-Za-z]:[\\/]/.test(raw);
  if (!isAbsolute) return null;
  // 去掉尾部分隔符，但保留根：`/` → `/`，`C:\` → `C:\`，`\\server\share` → `\\server\share`
  const trimmed = raw.replace(/[\\/]+$/, "");
  if (!trimmed) return raw.startsWith("/") ? "/" : raw;
  // 去掉后仍是 `C:` 形式说明原本是盘符根 `C:\`
  if (/^[A-Za-z]:$/.test(trimmed)) return `${trimmed}\\`;
  return trimmed;
}

/** 从 localStorage 读取上次目录；无记录、无效值或存储不可用时返回 null */
export function loadLastLocalDir(storage: Pick<Storage, "getItem"> | null | undefined): string | null {
  if (!storage) return null;
  try {
    return normalizeLocalDir(storage.getItem(LAST_LOCAL_DIR_KEY));
  } catch {
    // 存储被禁用/配额异常：降级为不记忆
    return null;
  }
}

/** 写入上次目录；无效路径不写入，存储异常静默忽略（不打断界面） */
export function saveLastLocalDir(
  storage: Pick<Storage, "setItem"> | null | undefined,
  path: string | null | undefined,
): boolean {
  if (!storage) return false;
  const normalized = normalizeLocalDir(path);
  if (!normalized) return false;
  try {
    storage.setItem(LAST_LOCAL_DIR_KEY, normalized);
    return true;
  } catch {
    return false;
  }
}
