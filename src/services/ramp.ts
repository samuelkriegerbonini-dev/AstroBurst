import { typedInvoke, withPreview, getOutputDir } from "../infrastructure/tauri";
import { cancelProgress } from "./progress";
import type { CubeSpectrum } from "../shared/types/cube";
import type {
  QuickSlopeParams,
  QuickSlopeResult,
  RampFrameResult,
  RampOpenInfo,
  RampPixelFit,
  RampPixelSeries,
  RampTables,
  RateComparison,
} from "../shared/types/ramp";

export const QSLOPE_PROGRESS_EVENT = "qslope-progress";

const DEFAULT_OUTPUT_PREFIX = "./output";

export function getRampInfo(path: string): Promise<RampOpenInfo> {
  return typedInvoke<RampOpenInfo>("ramp_info_cmd", { path });
}

export function getRampPixelSeries(path: string, x: number, y: number, integration: number): Promise<RampPixelSeries> {
  return typedInvoke<RampPixelSeries>("ramp_pixel_series_cmd", { path, x, y, integration });
}

export function getRampTables(path: string, maxRows = 200): Promise<RampTables> {
  return typedInvoke<RampTables>("ramp_tables_cmd", { path, maxRows });
}

export async function getRampFrame(path: string, frameIndex: number, outputPath: string, outputFits?: string): Promise<RampFrameResult> {
  const dir = await getOutputDir();
  const resolve = (p: string) => (p.startsWith(DEFAULT_OUTPUT_PREFIX) ? p.replace(DEFAULT_OUTPUT_PREFIX, dir) : p);
  return typedInvoke<RampFrameResult>("ramp_frame_cmd", {
    path,
    frameIndex,
    outputPath: resolve(outputPath),
    outputFits: outputFits ? resolve(outputFits) : undefined,
  });
}

export function getRampPixelFit(
  path: string,
  x: number,
  y: number,
  integration: number,
  params: QuickSlopeParams,
): Promise<RampPixelFit> {
  return typedInvoke<RampPixelFit>("ramp_pixel_fit_cmd", { path, x, y, integration, params });
}

export function runQuickSlope(
  path: string,
  outputDir: string | undefined,
  integration: number,
  params: QuickSlopeParams,
): Promise<QuickSlopeResult> {
  return withPreview<QuickSlopeResult>("ramp_quick_slope_cmd", outputDir, { path, integration, params });
}

export function compareWithRate(
  qslopePath: string,
  ratePath: string,
  outputDir: string | undefined,
  uncalRows: [number, number] | null,
): Promise<RateComparison> {
  return withPreview<RateComparison>("ramp_compare_rate_cmd", outputDir, { qslopePath, ratePath, uncalRows }, [
    ["ratio_png_path", "ratioPreviewUrl"],
  ]);
}

export function cancelQuickSlope(): Promise<boolean> {
  return cancelProgress(QSLOPE_PROGRESS_EVENT);
}

export function rampSpectrumFromSeries(series: RampPixelSeries): CubeSpectrum {
  return { values: series.values, wavelengths: [], x: series.x, y: series.y, is_spectral: false, flux_jy: null };
}
