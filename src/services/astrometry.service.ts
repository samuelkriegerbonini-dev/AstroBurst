import { safeInvoke } from "../infrastructure/tauri";
import type { WcsInfo, PlateSolveOptions, WorldCoord, PixelCoord } from "../shared/types/astrometry.types";

export type { WcsInfo, PlateSolveOptions, WorldCoord, PixelCoord } from "../shared/types/astrometry.types";

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

export async function pixelToWorld(path: string, x: number, y: number): Promise<WorldCoord> {
  return safeInvoke("pixel_to_world", { path, x, y });
}

export async function worldToPixel(path: string, ra: number, dec: number): Promise<PixelCoord> {
  return safeInvoke("world_to_pixel", { path, ra, dec });
}
