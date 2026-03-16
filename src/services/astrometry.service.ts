import { safeInvoke } from "../infrastructure/tauri";

export function plateSolve(path: string, options: Record<string, any> = {}) {
  return safeInvoke("plate_solve_cmd", { path, ...options });
}

export function getWcsInfo(path: string) {
  return safeInvoke("get_wcs_info", { path });
}

export function pixelToWorld(path: string, x: number, y: number) {
  return safeInvoke("pixel_to_world", { path, x, y });
}
