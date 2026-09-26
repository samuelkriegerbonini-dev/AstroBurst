export const SKY_WINDOW_SIGMA_BELOW = 5;
export const SKY_WINDOW_SIGMA_ABOVE = 50;

export interface HistogramRange {
  lo: number;
  hi: number;
}

export interface HistogramFrame {
  min: number;
  max: number;
}

export interface SkyWindowInput {
  median: number;
  sigma: number;
  dataMin: number;
  dataMax: number;
}

export function histogramSkyWindow({ median, sigma, dataMin, dataMax }: SkyWindowInput): HistogramRange | null {
  if (![median, sigma, dataMin, dataMax].every(Number.isFinite) || sigma <= 0) return null;
  const lo = Math.max(median - SKY_WINDOW_SIGMA_BELOW * sigma, dataMin);
  const hi = Math.min(median + SKY_WINDOW_SIGMA_ABOVE * sigma, dataMax);
  return hi > lo ? { lo, hi } : null;
}

function clampUnit(value: number): number {
  return Math.max(0, Math.min(1, value));
}

function usableFrame(frame: HistogramFrame): boolean {
  return Number.isFinite(frame.min) && Number.isFinite(frame.max) && frame.max > frame.min;
}

export function normToWindow(
  norm: number,
  frame: HistogramFrame,
  window: HistogramRange | null,
): { x: number; pinned: boolean } {
  if (!window || !usableFrame(frame)) return { x: norm, pinned: false };
  const value = frame.min + norm * (frame.max - frame.min);
  const x = (value - window.lo) / (window.hi - window.lo);
  const eps = 1e-9;
  if (x < -eps) return { x: 0, pinned: true };
  if (x > 1 + eps) return { x: 1, pinned: true };
  return { x: clampUnit(x), pinned: false };
}

export function windowToNorm(x: number, frame: HistogramFrame, window: HistogramRange | null): number {
  if (!window || !usableFrame(frame)) return clampUnit(x);
  const value = window.lo + clampUnit(x) * (window.hi - window.lo);
  return clampUnit((value - frame.min) / (frame.max - frame.min));
}

export type StfMarker = "shadow" | "midtone" | "highlight";

export interface StfPoints {
  shadow: number;
  midtone: number;
  highlight: number;
}

export interface MarkerPosition {
  x: number;
  pinned: boolean;
}

export const STF_MIN_GAP = 0.01;
export const STF_MIDTONE_MARGIN = 0.001;

const MARKER_ORDER: StfMarker[] = ["shadow", "midtone", "highlight"];
const TIE_TOLERANCE = 1e-12;

export function windowSpanNorm(frame: HistogramFrame, window: HistogramRange | null): number {
  if (!window || !usableFrame(frame)) return 1;
  const span = (window.hi - window.lo) / (frame.max - frame.min);
  return Number.isFinite(span) && span > 0 ? Math.min(1, span) : 1;
}

export function stfMinGap(frame: HistogramFrame, window: HistogramRange | null): number {
  return STF_MIN_GAP * windowSpanNorm(frame, window);
}

export function stfMidtoneFloor(range: number, frame: HistogramFrame, window: HistogramRange | null): number {
  if (!(range > 0)) return STF_MIDTONE_MARGIN;
  return STF_MIDTONE_MARGIN * Math.min(1, windowSpanNorm(frame, window) / range);
}

export function stfMarkerPositions(
  stf: StfPoints,
  frame: HistogramFrame,
  window: HistogramRange | null,
): Record<StfMarker, MarkerPosition> {
  return {
    shadow: normToWindow(stf.shadow, frame, window),
    midtone: normToWindow(stf.shadow + stf.midtone * (stf.highlight - stf.shadow), frame, window),
    highlight: normToWindow(stf.highlight, frame, window),
  };
}

export function pickStfMarker(
  pointerX: number,
  stf: StfPoints,
  frame: HistogramFrame,
  window: HistogramRange | null,
  threshold: number,
): StfMarker | null {
  const positions = stfMarkerPositions(stf, frame, window);
  const distances = MARKER_ORDER.map((marker) => Math.abs(pointerX - positions[marker].x));
  const best = Math.min(...distances);
  if (!(best <= threshold)) return null;
  const tied = MARKER_ORDER.filter((_, i) => distances[i] - best <= TIE_TOLERANCE);
  return pointerX >= 0.5 ? tied[0] : tied[tied.length - 1];
}

function clampMidtone(midtone: number, shadow: number, highlight: number, frame: HistogramFrame, window: HistogramRange | null): number {
  const floor = stfMidtoneFloor(highlight - shadow, frame, window);
  return Math.max(floor, Math.min(1 - floor, midtone));
}

export function constrainStf(stf: StfPoints, frame: HistogramFrame, window: HistogramRange | null): StfPoints {
  const gap = stfMinGap(frame, window);
  const shadow = Math.max(0, Math.min(stf.shadow, 1 - gap));
  const highlight = Math.min(1, Math.max(stf.highlight, shadow + gap));
  return { shadow, midtone: clampMidtone(stf.midtone, shadow, highlight, frame, window), highlight };
}

export function dragStfMarker(
  marker: StfMarker,
  pointerX: number,
  stf: StfPoints,
  frame: HistogramFrame,
  window: HistogramRange | null,
): StfPoints | null {
  const n = windowToNorm(pointerX, frame, window);
  const gap = stfMinGap(frame, window);
  let next: StfPoints;
  if (marker === "shadow") {
    next = { ...stf, shadow: Math.max(0, Math.min(n, stf.highlight - gap)) };
  } else if (marker === "highlight") {
    next = { ...stf, highlight: Math.min(1, Math.max(n, stf.shadow + gap)) };
  } else {
    const range = stf.highlight - stf.shadow;
    if (!(range > 0)) return null;
    next = { ...stf, midtone: clampMidtone((n - stf.shadow) / range, stf.shadow, stf.highlight, frame, window) };
  }
  const before = stfMarkerPositions(stf, frame, window)[marker];
  const after = stfMarkerPositions(next, frame, window)[marker];
  if (before.pinned && after.pinned && before.x === after.x) return null;
  return next;
}
