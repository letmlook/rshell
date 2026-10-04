/**
 * 会话级唯一 id 生成。
 *
 * 优先用 `crypto.randomUUID()`（WebView / 浏览器，localhost 与 tauri:// 都算
 * 安全上下文）。但 jsdom 等非浏览器环境未必实现它，直接调用会抛错并打断
 * 面板创建流程——因此保留 `getRandomValues` 兜底。
 *
 * 兜底也拿不到随机源时**抛错**，绝不退回时间戳/计数器：那种 id 可预测，
 * 且会和真实会话/面板 id 撞车。
 */

/** Web Crypto 是否可用（不依赖全局 crypto 一定存在） */
function webCrypto(): Crypto | undefined {
  const c = (globalThis as { crypto?: Crypto }).crypto;
  return c && typeof c.getRandomValues === "function" ? c : undefined;
}

/** 格式化为 v4 UUID 字符串 */
function formatV4(bytes: Uint8Array): string {
  bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
  bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 10x
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

export function newUuid(): string {
  const c = webCrypto();
  if (c && typeof c.randomUUID === "function") return c.randomUUID();
  if (!c) throw new Error("无法生成唯一 id：当前环境缺少安全随机源");
  return formatV4(c.getRandomValues(new Uint8Array(16)));
}
