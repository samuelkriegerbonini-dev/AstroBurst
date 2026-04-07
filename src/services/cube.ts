import { typedInvoke, withPreview, getOutputDir } from "../infrastructure/tauri";
import type { CubeDims, CubeProcessResult, CubeSpectrum } from "../shared/types/cube";

const CUBE_PREVIEWS: [string, string][] = [
  ["collapsed_path", "collapsedPreviewUrl"],
  ["collapsed_median_path", "collapsedMedianPreviewUrl"],
];

export function processCube(path: string, outputDir?: string, frameStep = 5): Promise<CubeProcessResult> {
  return withPreview<CubeProcessResult>("process_cube_cmd", outputDir, { path, frameStep }, CUBE_PREVIEWS);
}

export function processCubeLazy(path: string, outputDir?: string, frameStep = 5): Promise<CubeProcessResult> {
  return withPreview<CubeProcessResult>("process_cube_lazy_cmd", outputDir, { path, frameStep }, CUBE_PREVIEWS);
}

export function getCubeInfo(path: string): Promise<CubeDims> {
  return typedInvoke<CubeDims>("get_cube_info", { path });
}

export async function getCubeFrame(
  path: string,
  frameIndex: number,
  outputPath: string,
  outputFits?: string,
): Promise<{ output_path: string; fits_path?: string }> {
  const dir = await getOutputDir();
  const resolve = (p: string) => p.startsWith("./output") ? p.replace("./output", dir) : p;
  return typedInvoke<{ output_path: string; fits_path?: string }>("get_cube_frame", {
    path,
    frameIndex,
    outputPath: resolve(outputPath),
    outputFits: outputFits ? resolve(outputFits) : undefined,
  });
}

export function getCubeSpectrum(path: string, x: number, y: number): Promise<CubeSpectrum> {
  return typedInvoke<CubeSpectrum>("get_cube_spectrum", { path, x, y });
}
