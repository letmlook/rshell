/**
 * 字节数的可读说法。
 *
 * 传输面板与隧道面板都要显示字节数，此前各自写了一份 —— 两处格式一旦漂移，
 * 同一个数在两个面板里会显示成不同的样子。这里收敛成唯一实现。
 */
export function formatSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}
