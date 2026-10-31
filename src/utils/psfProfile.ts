import type { EmpiricalPsfStar } from "../shared/types/processing";

function coordinate(value: number): string {
  return String(Number(value.toFixed(2)));
}

export function profilePoints(profile: readonly number[], width: number, height: number): string {
  const peak = profile.reduce((max, value) => (Number.isFinite(value) && value > max ? value : max), 0);
  if (!(peak > 0)) return "";
  const step = profile.length > 1 ? width / (profile.length - 1) : 0;
  return profile
    .map((value, index) => (Number.isFinite(value) ? `${coordinate(index * step)},${coordinate(height * (1 - Math.max(value, 0) / peak))}` : null))
    .filter((point): point is string => point !== null)
    .join(" ");
}

export function paintKernel(canvas: HTMLCanvasElement, kernel: readonly (readonly number[])[]): void {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const rows = kernel.length;
  const cols = rows > 0 ? kernel[0].length : 0;
  canvas.width = cols;
  canvas.height = rows;
  const maxVal = Math.max(...kernel.flat());
  if (maxVal === 0) return;
  const imgData = ctx.createImageData(cols, rows);
  for (let y = 0; y < rows; y++) {
    for (let x = 0; x < cols; x++) {
      const v = Math.floor((kernel[y][x] / maxVal) * 255);
      const idx = (y * cols + x) * 4;
      imgData.data[idx] = v;
      imgData.data[idx + 1] = v;
      imgData.data[idx + 2] = v;
      imgData.data[idx + 3] = 255;
    }
  }
  ctx.putImageData(imgData, 0, 0);
}

export function starStatusLabel(star: Pick<EmpiricalPsfStar, "used" | "rejected" | "centroid">): string {
  if (!star.used) return `rejected: ${star.rejected ?? "unknown reason"}`;
  return star.centroid === "peak" ? "used (peak fallback)" : "used (quadratic centroid)";
}
