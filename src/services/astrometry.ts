import { typedInvoke, withPreview } from "../infrastructure/tauri";
import { cancelProgress } from "./progress";
import type {
  WcsInfo,
  PlateSolveOptions,
  PixelToWorldResult,
  PointingOverlapResult,
  SkyFrame,
  SkySeparationResult,
  SolveHints,
  WcsCard,
  WcsGridError,
  WcsGridResult,
  WcsParity,
  WorldToPixelResult,
  WriteSolvedWcsResult,
} from "../shared/types/astrometry";

export type {
  WcsInfo,
  PlateSolveOptions,
  PlateSolver,
  SolveHints,
  PixelToWorldResult,
  PointingOverlapResult,
  SkyFrame,
  SkySeparationResult,
  WcsGrid,
  WcsGridError,
  WcsCard,
  WcsGridResult,
  WorldToPixelResult,
  WriteSolvedWcsResult,
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

export interface PlateSolveAnnotation {
  type: string;
  names: string[];
  pixelx: number;
  pixely: number;
  radius?: number | null;
}

export interface PlateSolveResult {
  center_ra: number;
  center_dec: number;
  orientation?: number | null;
  pixel_scale_arcsec: number;
  field_of_view_w_arcmin: number;
  field_of_view_h_arcmin: number;
  wcs_cards: WcsCard[];
  annotations?: PlateSolveAnnotation[];
  solver?: string | null;
  tier?: number | null;
  catalog?: string | null;
  matched?: number | null;
  predicted?: number | null;
  rms_px?: number | null;
  sip_order?: number | null;
  sip_max_err_px?: number | null;
  hint_source?: string | null;
  elapsed_ms?: number | null;
  parity?: WcsParity | null;
  warnings?: string[];
}

export const LOCAL_NO_SOLUTION_CODE = "local_no_solution";
export const LOCAL_MISSING_HINT_CODE = "local_missing_hint";
export const LOCAL_NO_CATALOGUE_CODE = "local_no_catalogue";

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
    solver: opts.solver ?? null,
    nearRadiusDeg: opts.nearRadiusDeg ?? null,
  });
}

export function getSolveHints(path: string): Promise<SolveHints> {
  return typedInvoke<SolveHints>("solve_hints_cmd", { path });
}

export const PLATE_SOLVE_PROGRESS_EVENT = "plate-solve-progress";

export function cancelPlateSolve(): Promise<boolean> {
  return cancelProgress(PLATE_SOLVE_PROGRESS_EVENT);
}

export function writeSolvedWcs(path: string, wcsCards: readonly WcsCard[], outputDir?: string): Promise<WriteSolvedWcsResult> {
  return withPreview<WriteSolvedWcsResult>("write_solved_wcs_cmd", outputDir, { path, wcsCards });
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
