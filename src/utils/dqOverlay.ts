export const DQ_OVERLAY_RGB: [number, number, number] = [255, 64, 64];
export const DQ_OVERLAY_ALPHA = 0.45;

export function paintDqMask(
  cells: Uint8Array,
  width: number,
  height: number,
  rgb: [number, number, number],
  alpha: number,
  out: Uint8ClampedArray,
): void {
  const n = width * height;
  const needed = n * 4;
  if (out.length < needed) {
    throw new Error(`paintDqMask: output needs ${needed} bytes, got ${out.length}`);
  }
  if (cells.length < n) {
    throw new Error(`paintDqMask: cells need ${n} entries, got ${cells.length}`);
  }
  const a = Math.round(255 * alpha);
  const [r, g, b] = rgb;
  for (let i = 0; i < n; i++) {
    const o = i * 4;
    if (cells[i] !== 0) {
      out[o] = r;
      out[o + 1] = g;
      out[o + 2] = b;
      out[o + 3] = a;
    } else {
      out[o] = 0;
      out[o + 1] = 0;
      out[o + 2] = 0;
      out[o + 3] = 0;
    }
  }
}
