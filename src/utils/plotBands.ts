export const DEFAULT_BAND_COLOR = "rgba(161,161,170,0.14)";

export interface PlotBandSpan {
  x0: number;
  x1: number;
  color: string;
  label: string | null;
}

export function visibleBands(
  bands: readonly { x0: number; x1: number; color?: string; label?: string }[],
  domain: readonly [number, number],
): PlotBandSpan[] {
  const lo = Math.min(domain[0], domain[1]);
  const hi = Math.max(domain[0], domain[1]);
  const spans: PlotBandSpan[] = [];
  for (const band of bands) {
    if (!Number.isFinite(band.x0) || !Number.isFinite(band.x1)) continue;
    const x0 = Math.max(Math.min(band.x0, band.x1), lo);
    const x1 = Math.min(Math.max(band.x0, band.x1), hi);
    if (!(x1 > x0)) continue;
    spans.push({ x0, x1, color: band.color ?? DEFAULT_BAND_COLOR, label: band.label ? band.label : null });
  }
  return spans;
}
