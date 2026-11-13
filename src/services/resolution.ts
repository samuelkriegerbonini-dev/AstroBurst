import { typedInvoke } from "../infrastructure/tauri";
import type { ImagePsfFwhm, ResolutionCheck, ResolutionRequest } from "../shared/types/resolution";

export function resolutionCheck(request: ResolutionRequest): Promise<ResolutionCheck> {
  return typedInvoke<ResolutionCheck>("resolution_check_cmd", { request });
}

export function imagePsfFwhm(path: string, maxStars?: number): Promise<ImagePsfFwhm> {
  return typedInvoke<ImagePsfFwhm>("image_psf_fwhm_cmd", { path, maxStars: maxStars ?? null });
}
