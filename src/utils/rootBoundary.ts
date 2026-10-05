/**
 * 已授权根目录的边界判定 —— 文件面板的**唯一**越界判据（R3-10）。
 *
 * 为什么要单独抽一个模块：`PathBar.commitEdit`（手输路径回车）与
 * `FileBrowserPane.navigateTo`（面包屑 / 目录树 / 双击 / 上级）都能把路径
 * 推进 `internalLocalPath`，而后者就是 `enqueueDownload` 的落盘目录。两处
 * 曾经各自写了一份「前缀字符串比较」，结论互相矛盾：
 *   - 前缀比较不折叠 `.` / `..`，`/home/user/../etc` 仍以 `/home/user/` 开头 → 放行；
 *   - `navigateTo` 只判 `/`，Windows 根 `C:\data` 反被自己的入口挡住。
 * 共用本模块后，两个入口不可能再分叉。
 *
 * 归一化是**词法**的：只做「统一分隔符 → 折叠重复分隔符 → 去掉结尾分隔符 →
 * 消解 `.` / `..` 段」，不碰文件系统、不解析符号链接。符号链接交给操作系统的
 * 权限与 Tauri scope 去管，这里只保证「用户输入的字面路径没有跳出已授权根」。
 * （远端侧另有 `validate_remote_mutation_path` 拒 `..`，前端这边必须自己收紧，
 * 否则两侧标准不一致。）
 */

/** 归一化结果：根/前缀 + 已消解 `..` 的路径段 */
export interface NormalizedPath {
  /**
   * 路径前缀：
   * - Windows 盘符统一成 `C:\`（大写，便于比较）；
   * - 绝对路径的 POSIX/Windows 单分隔符都归一成 `\`；
   * - UNC 前缀 `\\` 保留双反斜杠，不能塌成 `\server`；
   * - 相对路径为空串（调用方据此判定「无法证明在根下」）。
   */
  prefix: string;
  /** 消解 `.` / `..` 之后的路径段；`/` 与根 `C:\` 都是空数组 */
  segments: string[];
  /** 前缀是否为 Windows 盘符（`C:\`）——这类路径的比较不区分大小写 */
  windowsDrive: boolean;
}

/**
 * 词法归一化：消解 `.` / `..`，不访问文件系统。
 *
 * `..` 越过根/盘符时被**钳制**在根（等同 `cd /..` 停在 `/`）：这样
 * `/home/user/../../../etc` 归一化成 `/etc`，随后的根比较会明确拒绝它，
 * 而不会被当成「相对根的合法子路径」。
 */
export function normalizePathSegments(path: string): NormalizedPath {
  const raw = path.trim();
  const drive = raw.match(/^([A-Za-z]:)[\\/]?/);
  if (drive) {
    return { prefix: `${drive[1].toUpperCase()}\\`, segments: split(raw.slice(drive[0].length)), windowsDrive: true };
  }
  // UNC（\\server\share）必须保留双反斜杠，否则会被读成 \server\share
  if (raw.startsWith("\\\\")) {
    return { prefix: "\\\\", segments: split(raw.slice(2)), windowsDrive: true };
  }
  if (raw.startsWith("/") || raw.startsWith("\\")) {
    return { prefix: "\\", segments: split(raw.slice(1)), windowsDrive: false };
  }
  // 相对路径：没有前缀，也没有可用来证明「在根下」的信息
  return { prefix: "", segments: split(raw), windowsDrive: false };
}

function split(rest: string): string[] {
  const out: string[] = [];
  for (const segment of rest.split(/[\\/]+/)) {
    if (!segment || segment === ".") continue;
    if (segment === "..") {
      out.pop();
      continue;
    }
    out.push(segment);
  }
  return out;
}

/**
 * 归一化后的 `candidate` 是否落在归一化后的 `root` 之内（等于根也算）。
 *
 * 逐段比较而不是字符串前缀：前缀比较会把同名前缀的兄弟目录放进来
 * （根 `/home/user` 时 `/home/user2` 通过）。任何无法证明在根下的输入都返回
 * false —— 相对路径、不同盘符、根本身不是绝对路径，都判越界。
 */
export function isWithinRoot(candidate: string, root: string): boolean {
  if (!candidate || !root) return false;
  const c = normalizePathSegments(candidate);
  const r = normalizePathSegments(root);
  // 前缀不同 = 不同根（盘符不同 / UNC vs 本地 / 相对路径），无法证明在根下
  if (!r.prefix || c.prefix !== r.prefix) return false;
  if (c.segments.length < r.segments.length) return false;
  const fold = (value: string) => (r.windowsDrive ? value.toLowerCase() : value);
  for (let i = 0; i < r.segments.length; i += 1) {
    // Windows 路径不区分大小写：`C:\Data` 与 `C:\data` 是同一个目录
    if (fold(c.segments[i]) !== fold(r.segments[i])) return false;
  }
  return true;
}
