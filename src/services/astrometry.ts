import { typedInvoke } from "../infrastructure/tauri";
import type {
  WcsInfo,
  PlateSolveOptions,
  PixelToWorldResult,
  PointingOverlapResult,
  SkyFrame,
  SkySeparationResult,
  WcsGridError,
  WcsGridResult,
  WorldToPixelResult,
} from "../shared/types/astrometry";

export type {
  WcsInfo,
  PlateSolveOptions,
  PixelToWorldResult,
  PointingOverlapResult,
  SkyFrame,
  SkySeparationResult,
  WcsGrid,
  WcsGridError,
  WcsGridResult,
  WorldToPixelResult,
} from "../shared/types/astrometry";

export const DEFAULT_GRID_DENSITY = 3;

export function isWcsGridError(result: WcsGridResult): result is WcsGridError {
  return typeof (result as WcsGridError).error === "string";
}

export function gridLines(
  path: string,
  frame: SkyFrame = "icrs",
  density: number = DEFAULT_GRID_DENSITY,
): Promise<WcsGridResult> {
  return typedInvoke<WcsGridResult>("grid_lines_cmd", { path, frame, density });
}

export interface PlateSolveResult {
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

export function pixelToWorld(
  path: string,
  points: [number, number][],
  frame: SkyFrame = "icrs",
): Promise<PixelToWorldResult> {
  return typedInvoke<PixelToWorldResult>("pixel_to_world_cmd", { path, points, frame });
}

export function worldToPixel(
  path: string,
  points: [number, number][],
  frame: SkyFrame = "icrs",
): Promise<WorldToPixelResult> {
  return typedInvoke<WorldToPixelResult>("world_to_pixel_cmd", { path, points, frame });
}

export function skySeparation(
  path: string | null,
  a: [number, number],
  b: [number, number],
  pixel: boolean,
): Promise<SkySeparationResult> {
  return typedInvoke<SkySeparationResult>("sky_separation_cmd", { path, a, b, pixel });
}
