import { typedInvoke } from "../infrastructure/tauri";
import type { ImageDepth, ImageDepthOptions } from "../shared/types/depth";

export function imageDepth(path: string, options: ImageDepthOptions = {}): Promise<ImageDepth> {
  return typedInvoke<ImageDepth>("image_depth_cmd", {
    path,
    radiusArcsec: options.radiusArcsec ?? null,
    radiusPx: options.radiusPx ?? null,
    nApertures: options.nApertures ?? null,
    nSigma: options.nSigma ?? null,
    seed: options.seed ?? null,
    detectNsigma: options.detectNsigma ?? null,
    psfFwhmArcsec: options.psfFwhmArcsec ?? null,
    excludeDq: options.excludeDq ?? null,
  });
}
