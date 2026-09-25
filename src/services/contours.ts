import { typedInvoke } from "../infrastructure/tauri";
import type { ContourOptions, ContourResult } from "../shared/types/contours";

export function contourLines(path: string, opts: ContourOptions): Promise<ContourResult> {
  return typedInvoke<ContourResult>("contour_lines_cmd", {
    path,
    mode: opts.mode,
    levels: opts.levels ?? null,
    nLevels: opts.nLevels ?? null,
    lo: opts.lo ?? null,
    hi: opts.hi ?? null,
    sigmaMultiples: opts.sigmaMultiples ?? null,
    smoothSigma: opts.smoothSigma ?? null,
    bin: opts.bin ?? null,
    excludeDq: opts.excludeDq ?? false,
  });
}
