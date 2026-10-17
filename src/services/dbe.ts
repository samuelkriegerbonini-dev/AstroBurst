import { withPreview } from "../infrastructure/tauri";
import type { DbeConfig, DbeResult } from "../shared/types/dbe";
import type { HistoryScope, OpSpec } from "../shared/types/ops";
import { recordOp } from "./ops";
import { asBackgroundResult, type RecordedBackgroundResult, type RecordedResult } from "./processing";

export function extractBackgroundDbe(
  path: string,
  outputDir: string | undefined,
  config: DbeConfig,
): Promise<DbeResult> {
  return withPreview<DbeResult>("extract_background_dbe_cmd", outputDir, { path, config }, [
    ["corrected_png", "previewUrl"],
    ["model_png", "modelUrl"],
  ]);
}

export function dbeConfigOf(op: OpSpec): DbeConfig | null {
  return op.op === "background" && op.model === "spline" ? op.config : null;
}

export type RecordedDbeResult = RecordedBackgroundResult & Partial<RecordedResult<Omit<DbeResult, "cache_key" | "model_fits">>>;

export async function recordBackgroundDbe(scope: HistoryScope, path: string, config: DbeConfig): Promise<RecordedDbeResult> {
  return asBackgroundResult(await recordOp(scope, path, { op: "background", v: 1, model: "spline", config })) as RecordedDbeResult;
}
