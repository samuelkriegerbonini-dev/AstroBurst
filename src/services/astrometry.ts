import { typedInvoke } from "../infrastructure/tauri";
import type { WcsInfo, PlateSolveOptions } from "../shared/types/astrometry";

export type { WcsInfo, PlateSolveOptions } from "../shared/types/astrometry";

export interface PlateSolveResult {
  wcs: WcsInfo;
  job_id?: number;
  elapsed_ms: number;
}

export function plateSolve(path: string, opts: PlateSolveOptions = {}): Promise<PlateSolveResult> {
  return typedInvoke<PlateSolveResult>("plate_solve_cmd", {
    path,
    apiKey: opts.apiKey ?? null,
    scaleLower: opts.scaleLower ?? null,
    scaleUpper: opts.scaleUpper ?? null,
    scaleUnits: opts.scaleUnits ?? null,
    downsampleFactor: opts.downsampleFactor ?? null,
    centerRa: opts.centerRa ?? null,
    centerDec: opts.centerDec ?? null,
    radius: opts.radius ?? null,
  });
}

export function getWcsInfo(path: string): Promise<WcsInfo> {
  return typedInvoke<WcsInfo>("get_wcs_info", { path });
}
