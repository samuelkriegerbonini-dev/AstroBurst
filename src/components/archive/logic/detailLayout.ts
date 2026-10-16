export const DETAIL_WIDTH_DEFAULT = 420;
export const DETAIL_WIDTH_MIN = 360;
export const DETAIL_WIDTH_MAX = 640;
export const DETAIL_KEY_STEP_PX = 16;
export const NARROW_BELOW_PX = 1100;

export function clampDetailWidth(width: number): number {
  if (!Number.isFinite(width)) return DETAIL_WIDTH_DEFAULT;
  return Math.round(Math.min(DETAIL_WIDTH_MAX, Math.max(DETAIL_WIDTH_MIN, width)));
}

export function resizeWidth(startWidth: number, startX: number, x: number): number {
  return clampDetailWidth(startWidth + (startX - x));
}

export function keyResize(width: number, key: string): number | null {
  switch (key) {
    case "ArrowLeft":
      return clampDetailWidth(width + DETAIL_KEY_STEP_PX);
    case "ArrowRight":
      return clampDetailWidth(width - DETAIL_KEY_STEP_PX);
    case "Home":
      return DETAIL_WIDTH_DEFAULT;
    default:
      return null;
  }
}

export function isNarrow(width: number): boolean {
  return width > 0 && width < NARROW_BELOW_PX;
}
