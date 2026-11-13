import type { ImageDepthOptions } from "../../shared/types/depth";

export type DepthRadiusUnit = "arcsec" | "px";

export interface DepthFields {
  radius: number | null;
  radiusUnit: DepthRadiusUnit;
  nApertures: number;
  nSigma: number;
  seed: number;
  detectNsigma: number;
  psfFwhmArcsec: number | null;
  useMask: boolean;
}

export const DEFAULT_DEPTH_FIELDS: DepthFields = {
  radius: null,
  radiusUnit: "arcsec",
  nApertures: 500,
  nSigma: 5,
  seed: 1,
  detectNsigma: 3,
  psfFwhmArcsec: null,
  useMask: true,
};

export function depthOptions(fields: DepthFields): ImageDepthOptions {
  return {
    radiusArcsec: fields.radiusUnit === "arcsec" ? fields.radius : null,
    radiusPx: fields.radiusUnit === "px" ? fields.radius : null,
    nApertures: fields.nApertures,
    nSigma: fields.nSigma,
    seed: fields.seed,
    detectNsigma: fields.detectNsigma,
    psfFwhmArcsec: fields.psfFwhmArcsec,
    excludeDq: fields.useMask ? null : false,
  };
}
