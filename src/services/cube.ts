import { typedInvoke, withPreview, getOutputDir, getPreviewUrl } from "../infrastructure/tauri";
import type {
  CollapseRangeMode,
  CollapseRangeResult,
  CubeDims,
  CubeSpectrum,
  MomentConfig,
  MomentMapsResult,
  RegionSpectrum,
} from "../shared/types/cube";
import type { RegionShape } from "../shared/types/regions";
import { parseImageRef } from "../utils/imageRef";

interface RawCubeSpectrum {
  spectrum?: number[];
  values?: number[];
  wavelengths?: number[] | null;
  is_spectral?: boolean;
}

export async function releaseCubes(paths: readonly string[]): Promise<void> {
  const sources = [...new Set(paths.map((p) => parseImageRef(p).path))];
  const settled = await Promise.allSettled(sources.map((path) => typedInvoke<void>("release_cube_cmd", { path })));
  settled.forEach((s, i) => {
    if (s.status === "rejected") console.warn(`[AstroBurst] Could not release the cube mapping of ${sources[i]}:`, s.reason);
  });
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

export function toCubeSpectrum(raw: RawCubeSpectrum, x: number, y: number): CubeSpectrum {
  return {
    values: raw.values ?? raw.spectrum ?? [],
    wavelengths: raw.wavelengths ?? [],
    x,
    y,
    is_spectral: raw.is_spectral,
  };
}

export async function getCubeSpectrum(path: string, x: number, y: number): Promise<CubeSpectrum> {
  const raw = await typedInvoke<RawCubeSpectrum>("get_cube_spectrum", { path, x, y });
  return toCubeSpectrum(raw, x, y);
}

export function getCubeSpectrumRegion(
  path: string,
  shape: RegionShape,
  background: RegionShape | null,
): Promise<RegionSpectrum> {
  return typedInvoke<RegionSpectrum>("get_cube_spectrum_region_cmd", { path, shape, background });
}

export function collapseCubeRange(
  path: string,
  outputDir: string | undefined,
  z0: number,
  z1: number,
  mode: CollapseRangeMode,
): Promise<CollapseRangeResult> {
  return withPreview<CollapseRangeResult>("collapse_cube_range_cmd", outputDir, { path, z0, z1, mode });
}

export async function computeMomentMaps(
  path: string,
  outputDir: string | undefined,
  config: MomentConfig,
): Promise<MomentMapsResult> {
  const dir = outputDir && outputDir !== "./output" ? outputDir : await getOutputDir();
  const result = await typedInvoke<MomentMapsResult>("moment_maps_cmd", { path, outputDir: dir, config });
  const [m0, m1, m2] = await Promise.all([
    getPreviewUrl(result.m0.png_path),
    getPreviewUrl(result.m1.png_path),
    getPreviewUrl(result.m2.png_path),
  ]);
  return {
    ...result,
    m0: { ...result.m0, previewUrl: m0 },
    m1: { ...result.m1, previewUrl: m1 },
    m2: { ...result.m2, previewUrl: m2 },
  };
}
