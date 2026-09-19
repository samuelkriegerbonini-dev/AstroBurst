import { withPreview } from "../infrastructure/tauri";
import type { CosmeticBatchResult, CosmeticOptions, CosmeticResult } from "../shared/types/cosmetic";

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
