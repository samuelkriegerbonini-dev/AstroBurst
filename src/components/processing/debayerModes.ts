import { debayerBatch, debayerFits, extractCfa, extractCfaBatch } from "../../services/processing";
import type {
  CfaExtractBatchResult,
  CfaExtractMode,
  CfaExtractPlane,
  CfaExtractResult,
  DebayerBatchResult,
  DebayerResult,
} from "../../services/processing";

export const RECONSTRUCT_MODES = [
  { value: "bilinear", label: "Bilinear (full resolution)" },
  { value: "superpixel", label: "Super-pixel (half resolution)" },
] as const;

export const EXTRACT_MODES = [
  { value: "green", label: "Green (mean of the two G sites)" },
  { value: "ha", label: "Hα (red sites)" },
  { value: "ha_oiii", label: "Hα + OIII" },
  { value: "split", label: "Split CFA (four site planes)" },
] as const;

export type DebayerMethod = (typeof RECONSTRUCT_MODES)[number]["value"];
export type PanelMode = DebayerMethod | CfaExtractMode;
export type PanelRunResult = DebayerResult | CfaExtractResult;
export type PanelBatchResult = DebayerBatchResult | CfaExtractBatchResult;

export interface PlaneRow {
  tag: string;
  name: string;
  path: string;
}

export function isExtractMode(mode: PanelMode): mode is CfaExtractMode {
  return EXTRACT_MODES.some((extract) => extract.value === mode);
}

export function isExtractResult(result: PanelRunResult): result is CfaExtractResult {
  return "planes" in result;
}

export function runPanelMode(mode: PanelMode, path: string, outputDir: string, pattern: string): Promise<PanelRunResult> {
  const chosen = pattern || undefined;
  if (isExtractMode(mode)) return extractCfa(path, outputDir, { mode, pattern: chosen });
  return debayerFits(path, outputDir, { method: mode, pattern: chosen });
}

export function runPanelBatch(mode: PanelMode, paths: string[], outputDir: string, pattern: string): Promise<PanelBatchResult> {
  const chosen = pattern || undefined;
  if (isExtractMode(mode)) return extractCfaBatch(paths, outputDir, { mode, pattern: chosen });
  return debayerBatch(paths, outputDir, { method: mode, pattern: chosen });
}

export function planeRows(result: { planes?: CfaExtractPlane[] }): PlaneRow[] {
  return (result.planes ?? []).map((plane) => ({
    tag: plane.tag,
    name: plane.fits_path.split(/[/\\]/).pop() ?? plane.fits_path,
    path: plane.fits_path,
  }));
}
