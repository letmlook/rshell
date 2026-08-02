export const DEFAULT_SIDEBAR_WIDTH = 280;
export const MIN_SIDEBAR_WIDTH = 200;
export const MAX_SIDEBAR_WIDTH = 360;
export const SIDEBAR_RESIZE_STEP = 8;
export const MAIN_AREA_TARGET_WIDTH = 360;

function finiteOr(value: number, fallback: number): number {
  return Number.isFinite(value) ? value : fallback;
}

export function clampSidebarWidth(
  value: number,
  upperBound = MAX_SIDEBAR_WIDTH,
): number {
  const safeUpper = Math.max(MIN_SIDEBAR_WIDTH, finiteOr(upperBound, MAX_SIDEBAR_WIDTH));
  const safeValue = finiteOr(value, DEFAULT_SIDEBAR_WIDTH);
  return Math.min(Math.max(safeValue, MIN_SIDEBAR_WIDTH), safeUpper);
}

export function maxSidebarWidthForViewport(viewportWidth: number): number {
  if (!Number.isFinite(viewportWidth)) return MAX_SIDEBAR_WIDTH;
  return Math.max(
    MIN_SIDEBAR_WIDTH,
    Math.min(MAX_SIDEBAR_WIDTH, viewportWidth - MAIN_AREA_TARGET_WIDTH),
  );
}

export function resizeSidebarWithKey(
  current: number,
  key: string,
  upperBound = MAX_SIDEBAR_WIDTH,
): number | null {
  if (key === "Home") return MIN_SIDEBAR_WIDTH;
  if (key === "End") return clampSidebarWidth(MAX_SIDEBAR_WIDTH, upperBound);
  if (key === "ArrowLeft") return clampSidebarWidth(current - SIDEBAR_RESIZE_STEP, upperBound);
  if (key === "ArrowRight") return clampSidebarWidth(current + SIDEBAR_RESIZE_STEP, upperBound);
  return null;
}
