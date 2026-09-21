export const WB_APPLY_MIN = 0.01;
export const WB_APPLY_MAX = 100;

const WB_SLIDER_MIN_CEILING = 0.1;
const WB_SLIDER_MAX_FLOOR = 3;

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

export function wbSliderBounds(r: number, g: number, b: number): { min: number; max: number } {
  const finite = [r, g, b].filter((v) => Number.isFinite(v));
  const hi = finite.length ? Math.max(...finite) : 1;
  const lo = finite.length ? Math.min(...finite) : 1;
  return {
    min: clamp(Math.floor(lo * 0.5 * 10) / 10, WB_APPLY_MIN, WB_SLIDER_MIN_CEILING),
    max: clamp(Math.ceil(hi * 1.5 * 10) / 10, WB_SLIDER_MAX_FLOOR, WB_APPLY_MAX),
  };
}

export function wbFactorsOutOfRange(r: number, g: number, b: number): string[] {
  return (
    [
      ["R", r],
      ["G", g],
      ["B", b],
    ] as const
  )
    .filter(([, v]) => !Number.isFinite(v) || v < WB_APPLY_MIN || v > WB_APPLY_MAX)
    .map(([name]) => name);
}
