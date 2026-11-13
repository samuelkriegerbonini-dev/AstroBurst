export interface DepthAperture {
  x: number;
  y: number;
  sum: number;
  formal_err: number | null;
  usable: boolean;
}

export interface ImageDepth {
  radius_arcsec: number | null;
  radius_px: number;
  pixel_scale_arcsec: number | null;
  n_requested: number;
  n_placed: number;
  n_used: number;
  seed: number;
  n_sigma: number;
  detect_nsigma: number;
  masked_fraction: number;
  median_sum_native: number;
  sigma_empty_native: number;
  sigma_clipped_native: number;
  n_after_clip: number;
  sigma_empty_rel_err: number;
  sigma_formal_native: number | null;
  f_corr: number | null;
  limit_native: number;
  limit_jy: number | null;
  limit_ab: number | null;
  psf_fwhm_arcsec: number | null;
  psf_limit_formal_native: number | null;
  psf_limit_formal_ab: number | null;
  sigma_pix_median_native: number | null;
  sigma_pix_spread: number | null;
  bunit: string | null;
  calibration_origin: string | null;
  err_origin: string | null;
  dq_source: string | null;
  definition: string;
  apertures: DepthAperture[];
  notes: string[];
  warnings: string[];
  elapsed_ms: number;
  detected_plane_pixels?: number | null;
}

export interface ImageDepthOptions {
  radiusArcsec?: number | null;
  radiusPx?: number | null;
  nApertures?: number | null;
  nSigma?: number | null;
  seed?: number | null;
  detectNsigma?: number | null;
  psfFwhmArcsec?: number | null;
  excludeDq?: boolean | null;
}
