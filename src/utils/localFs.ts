/**
 * 本地目录条目的真实类型判定 —— 解决 `readDir` 不跟随符号链接的问题。
 *
 * `tauri-plugin-fs` 的 `readDir` 用 `std::fs::DirEntry::file_type()` 取类型，
 * 这个方法**不跟随符号链接**（`std::fs::metadata` 才跟随）。于是 macOS 上
 * `/tmp`、`/var`、`/etc`、`/home` 这些「指向 `/private/*` 的符号链接」会被
 * 报成 `isDirectory: false`：
 *
 *   - 路径选择器的目录模式按 `isDirectory` 过滤 → 这些目录**在列表里彻底消失**，
 *     用户选不到 `/tmp`（真机实测：`/` 下只列出 Applications/bin/cores/…，
 *     唯独没有 `tmp`）；
 *   - 文件面板把符号链接目录画成文件图标，双击进不去；
 *   - 目录树按 `isDirectory` 过滤 → 树里也少一层。
 *
 * `stat` 用 `std::fs::metadata()`，**跟随符号链接**，拿到的才是真实类型。
 * 所以凡是已经取到 `stat` 结果的地方，直接用 `info.isDirectory`；只拿到
 * `readDir` 条目的地方（目录树），用本模块的 `entryIsDirectory` 补一次 stat。
 *
 * stat 失败（条目刚被删、权限被拒）时退回 `readDir` 的判断，不凭空造类型。
 */
import { stat } from "@tauri-apps/plugin-fs";

/**
 * 判断一个 `readDir` 条目指向的是不是目录（跟随符号链接）。
 *
 * @param fullPath 条目的绝对路径
 * @param fallback stat 失败时使用的兜底值（通常是 `readDir` 的 `isDirectory`）
 */
export async function entryIsDirectory(
  fullPath: string,
  fallback: boolean,
): Promise<boolean> {
  try {
    const info = await stat(fullPath);
    return info.isDirectory;
  } catch {
    return fallback;
  }
}
