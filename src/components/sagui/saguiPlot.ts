import type { SaguiRegionalBand } from "../../shared/types/sagui";

export function regionAt(result: { width: number; height: number; labels: number[] }, x: number, y: number): number | null {
  if (x < 0 || y < 0 || x >= 1 || y >= 1 || !Number.isFinite(x) || !Number.isFinite(y)) return null;
  const col = Math.floor(x * result.width);
  const row = result.height - 1 - Math.floor(y * result.height);
  return result.labels[row * result.width + col] ?? null;
}

export function regionColor(region: number): [number, number, number] {
  if (region === 0) return [24, 24, 27];
  const hue = (region * 137.508 % 360) / 60;
  const x = 170 * (1 - Math.abs(hue % 2 - 1));
  const components = hue < 1 ? [170, x, 0] : hue < 2 ? [x, 170, 0] : hue < 3 ? [0, 170, x] : hue < 4 ? [0, x, 170] : hue < 5 ? [x, 0, 170] : [170, 0, x];
  return components.map((value) => Math.round(value + 65)) as [number, number, number];
}

export function sedDomain(bands: SaguiRegionalBand[]) {
  const valid = bands.filter((band) => Number.isFinite(band.pivot_um) && Number.isFinite(band.flux));
  const xs = valid.map((band) => band.pivot_um);
  const ys = valid.flatMap((band) => { const error = band.error !== null && Number.isFinite(band.error) && band.error >= 0 ? band.error : 0; return [band.flux - error, band.flux + error]; });
  const xLow = xs.length ? Math.min(...xs) : 0;
  const xHigh = xs.length ? Math.max(...xs) : 1;
  const yLow = ys.length ? Math.min(0, ...ys) : 0;
  const yHigh = ys.length ? Math.max(0, ...ys) : 1;
  const xPad = (xHigh - xLow || Math.abs(xLow) || 1) * 0.1;
  const yPad = (yHigh - yLow || Math.abs(yLow) || 1) * 0.1;
  return { xMin: xLow - xPad, xMax: xHigh + xPad, yMin: yLow - yPad, yMax: yHigh + yPad };
}
