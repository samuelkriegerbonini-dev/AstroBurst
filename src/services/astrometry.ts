import { typedInvoke } from "../infrastructure/tauri";
import type { WcsInfo, PlateSolveOptions, PixelToWorldResult, PointingOverlapResult } from "../shared/types/astrometry";

export type { WcsInfo, PlateSolveOptions, PixelToWorldResult, PointingOverlapResult } from "../shared/types/astrometry";

export interface PlateSolveResult {
  success: boolean;
  center_ra: number;
  center_dec: number;
  orientation: number;
  pixel_scale_arcsec: number;
  field_of_view_w_arcmin: number;
  field_of_view_h_arcmin: number;
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

export function checkPointingOverlap(paths: string[], threshold?: number): Promise<PointingOverlapResult> {
  return typedInvoke<PointingOverlapResult>("check_pointing_overlap_cmd", {
    paths,
    threshold: threshold ?? null,
  });
}

export function pixelToWorld(path: string, points: [number, number][]): Promise<PixelToWorldResult> {
  return typedInvoke<PixelToWorldResult>("pixel_to_world_cmd", { path, points });
}
