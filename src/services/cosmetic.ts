import { withPreview } from "../infrastructure/tauri";
import type { CosmeticBatchResult, CosmeticCounts, CosmeticOptions, CosmeticResult } from "../shared/types/cosmetic";

export type CosmeticDefectKind = "hot" | "cold" | "listed";

export interface CosmeticDryRunResult {
  dry_run: true;
  path: string;
  png_path: string;
  previewUrl?: string;
  dimensions: [number, number];
  counts: CosmeticCounts;
  defects: [number, number, CosmeticDefectKind][];
  defects_total: number;
  dq_present: boolean;
  warning?: string | null;
  cfa_applied: boolean;
  cfa_source: string;
  elapsed_ms: number;
}

function toArgs(opts: CosmeticOptions): Record<string, unknown> {
  return {
    masterDarkPath: opts.masterDarkPath ?? null,
    config: opts.config,
    defectListText: opts.defectListText ?? null,
  };
}

export function cosmeticCorrect(path: string, outputDir: string | undefined, opts: CosmeticOptions): Promise<CosmeticResult> {
  return withPreview<CosmeticResult>("cosmetic_correct_cmd", outputDir, { path, ...toArgs(opts) });
}

export function cosmeticCorrectBatch(
  paths: string[],
  outputDir: string | undefined,
  opts: CosmeticOptions,
): Promise<CosmeticBatchResult> {
  return withPreview<CosmeticBatchResult>("cosmetic_correct_batch_cmd", outputDir, { paths, ...toArgs(opts) }, []);
}

export function cosmeticDryRun(path: string, outputDir: string | undefined, opts: CosmeticOptions): Promise<CosmeticDryRunResult> {
  return withPreview<CosmeticDryRunResult>("cosmetic_correct_cmd", outputDir, { path, ...toArgs(opts), dryRun: true });
}
