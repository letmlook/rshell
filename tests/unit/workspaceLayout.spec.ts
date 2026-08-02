import { describe, expect, it } from "vitest";
import {
  DEFAULT_SIDEBAR_WIDTH,
  MIN_SIDEBAR_WIDTH,
  MAX_SIDEBAR_WIDTH,
  SIDEBAR_RESIZE_STEP,
  clampSidebarWidth,
  maxSidebarWidthForViewport,
  resizeSidebarWithKey,
} from "../../src/utils/workspaceLayout";

describe("workspace layout constants", () => {
  it("uses the confirmed 280/200/360 geometry", () => {
    expect(DEFAULT_SIDEBAR_WIDTH).toBe(280);
    expect(MIN_SIDEBAR_WIDTH).toBe(200);
    expect(MAX_SIDEBAR_WIDTH).toBe(360);
    expect(SIDEBAR_RESIZE_STEP).toBe(8);
  });
});

describe("clampSidebarWidth", () => {
  it("clamps values to the normal range", () => {
    expect(clampSidebarWidth(100)).toBe(200);
    expect(clampSidebarWidth(280)).toBe(280);
    expect(clampSidebarWidth(500)).toBe(360);
  });

  it("falls back to the default for non-finite input", () => {
    expect(clampSidebarWidth(Number.NaN)).toBe(280);
    expect(clampSidebarWidth(Number.POSITIVE_INFINITY)).toBe(280);
    expect(clampSidebarWidth(Number.NEGATIVE_INFINITY)).toBe(280);
  });

  it("honors a valid dynamic upper bound without violating the minimum", () => {
    expect(clampSidebarWidth(340, 300)).toBe(300);
    expect(clampSidebarWidth(340, 120)).toBe(200);
  });
});

describe("maxSidebarWidthForViewport", () => {
  it("preserves the 360px main-area target when possible", () => {
    expect(maxSidebarWidthForViewport(1280)).toBe(360);
    expect(maxSidebarWidthForViewport(700)).toBe(340);
  });

  it("returns the minimum when the viewport cannot fit both preferred regions", () => {
    expect(maxSidebarWidthForViewport(500)).toBe(200);
    expect(maxSidebarWidthForViewport(Number.NaN)).toBe(360);
  });
});

describe("resizeSidebarWithKey", () => {
  it("moves by 8px and clamps", () => {
    expect(resizeSidebarWithKey(280, "ArrowRight")).toBe(288);
    expect(resizeSidebarWithKey(280, "ArrowLeft")).toBe(272);
    expect(resizeSidebarWithKey(200, "ArrowLeft")).toBe(200);
    expect(resizeSidebarWithKey(360, "ArrowRight")).toBe(360);
  });

  it("supports Home and End and ignores unrelated keys", () => {
    expect(resizeSidebarWithKey(280, "Home")).toBe(200);
    expect(resizeSidebarWithKey(280, "End")).toBe(360);
    expect(resizeSidebarWithKey(280, "Enter")).toBeNull();
  });
});
