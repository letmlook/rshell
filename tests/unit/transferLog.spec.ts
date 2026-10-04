import { beforeEach, describe, expect, it } from "vitest";
import {
  MAX_TRANSFER_LOG_ENTRIES,
  appendTransferLog,
  clearTransferLog,
  formatTransferLogTime,
  useTransferLog,
} from "../../src/utils/transferLog";

/**
 * 日志 tab 之前只有一行占位文案，用户看不到任何真实事件。这里的用例把
 * 「真事件进缓冲、超容量丢最旧、清空归零」三条契约钉住。
 */
describe("transferLog 环形缓冲", () => {
  beforeEach(() => clearTransferLog());

  it("按写入顺序保留日志并带上等级与时间戳", () => {
    const { entries } = useTransferLog();
    appendTransferLog("info", "上传入队 1 个文件 → /var/log");
    appendTransferLog("error", "传输失败：a.log", "Permission denied");

    expect(entries.value).toHaveLength(2);
    expect(entries.value[0].message).toContain("上传入队");
    expect(entries.value[0].level).toBe("info");
    expect(entries.value[1].level).toBe("error");
    expect(entries.value[1].detail).toBe("Permission denied");
    expect(entries.value[0].seq).toBeLessThan(entries.value[1].seq);
  });

  it("超过容量时丢最旧的条目，并如实记录丢了多少条", () => {
    const { entries, droppedCount } = useTransferLog();
    for (let i = 0; i < MAX_TRANSFER_LOG_ENTRIES + 5; i += 1) {
      appendTransferLog("info", `entry-${i}`);
    }

    expect(entries.value).toHaveLength(MAX_TRANSFER_LOG_ENTRIES);
    expect(droppedCount.value).toBe(5);
    // 保留的是最近 500 条：最早的 5 条已被丢弃
    expect(entries.value[0].message).toBe("entry-5");
    expect(entries.value.at(-1)?.message).toBe(`entry-${MAX_TRANSFER_LOG_ENTRIES + 4}`);
  });

  it("清空后缓冲与丢弃计数一起归零", () => {
    const { entries, droppedCount } = useTransferLog();
    for (let i = 0; i < MAX_TRANSFER_LOG_ENTRIES + 2; i += 1) appendTransferLog("info", `e-${i}`);
    expect(droppedCount.value).toBeGreaterThan(0);

    clearTransferLog();

    expect(entries.value).toEqual([]);
    expect(droppedCount.value).toBe(0);
  });

  it("时间格式化对非法时间戳不显示 Invalid Date", () => {
    expect(formatTransferLogTime(Number.NaN)).toBe("--:--:--");
    expect(formatTransferLogTime(Number.POSITIVE_INFINITY)).toBe("--:--:--");
    const stamp = formatTransferLogTime(new Date(2026, 0, 2, 3, 4, 5).getTime());
    expect(stamp).toBe("03:04:05");
  });
});
