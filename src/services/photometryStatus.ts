import { typedInvoke } from "../infrastructure/tauri";
import type { PhotometryStatus } from "../shared/types/photometryStatus";
import type { UserZeroPoint } from "./analysis";

export interface PhotometryStatusOptions {
  excludeDq?: boolean;
  zeroPoint?: UserZeroPoint | null;
}

export function photometryStatus(path: string, options: PhotometryStatusOptions = {}): Promise<PhotometryStatus> {
  return typedInvoke<PhotometryStatus>("photometry_status_cmd", {
    path,
    excludeDq: options.excludeDq ?? false,
    zeroPoint: options.zeroPoint ?? null,
  });
}
