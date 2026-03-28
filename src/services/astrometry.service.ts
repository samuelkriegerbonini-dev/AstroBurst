import { safeInvoke } from "../infrastructure/tauri";
import type { WcsInfo, PlateSolveOptions } from "../shared/types/astrometry.types";

export type { WcsInfo, PlateSolveOptions } from "../shared/types/astrometry.types";

export async function plateSolve(path: string, opts: PlateSolveOptions = {}) {
  return safeInvoke("plate_solve_cmd", {
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

export async function getWcsInfo(path: string): Promise<WcsInfo> {
  return safeInvoke("get_wcs_info", { path });
}
