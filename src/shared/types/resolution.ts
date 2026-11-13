export type ResolutionVerdict = "unresolved" | "marginal" | "resolved";

export type ResolutionSizeBasis = "fwhm" | "half_light_radius";

export interface ResolutionRequest {
  psf_fwhm_arcsec?: number | null;
  psf_reference?: string | null;
  pixel_scale_arcsec?: number | null;
  source_fwhm_arcsec?: number | null;
  source_half_light_radius_arcsec?: number | null;
  psf_half_light_radius_arcsec?: number | null;
  snr?: number | null;
  redshift?: number | null;
  psf_fwhm_rel_err?: number | null;
  resolved_elements?: number | null;
  min_pixels_per_fwhm?: number | null;
  scale_of_interest_kpc?: number | null;
  h0_km_s_mpc?: number | null;
  omega_m?: number | null;
}

export interface ResolutionCheck {
  verdict: ResolutionVerdict | null;
  size_basis: ResolutionSizeBasis | null;
  psf_fwhm_arcsec: number | null;
  psf_reference: string | null;
  pixels_per_fwhm: number | null;
  nyquist_sampled: boolean | null;
  psf_area_px: number | null;
  intrinsic_size_arcsec: number | null;
  snr_floor_fraction: number | null;
  psf_floor_fraction: number;
  floor_fraction: number | null;
  size_floor_arcsec: number | null;
  resolution_elements: number | null;
  kpc_per_arcsec: number | null;
  kpc_per_pixel: number | null;
  angular_diameter_distance_mpc: number | null;
  psf_fwhm_kpc: number | null;
  intrinsic_size_kpc: number | null;
  size_floor_kpc: number | null;
  scale_of_interest_in_psf: number | null;
  snr: number | null;
  redshift: number | null;
  psf_fwhm_rel_err: number;
  resolved_elements: number;
  min_pixels_per_fwhm: number;
  scale_of_interest_kpc: number;
  h0_km_s_mpc: number;
  omega_m: number;
  omega_nu_massive: number;
  omega_rad: number;
  cosmology: string;
  notes: string[];
  warnings: string[];
}

export interface ImagePsfStar {
  x: number;
  y: number;
  fwhm_px: number;
  half_light_radius_px: number | null;
  peak_snr: number;
  on_locus: boolean;
}

export interface SeeingCard {
  key: string;
  value_arcsec: number;
}

export interface ImagePsfFwhm {
  fwhm_px: number;
  fwhm_px_scatter: number | null;
  fwhm_arcsec: number | null;
  fwhm_arcsec_scatter: number | null;
  half_light_radius_px: number | null;
  half_light_radius_arcsec: number | null;
  n_stars: number;
  n_measured: number;
  n_candidates: number;
  n_detected: number;
  max_stars: number;
  model: string;
  pixel_scale_arcsec: number | null;
  pixels_per_fwhm: number;
  undersampled: boolean;
  header_values: SeeingCard[];
  stars: ImagePsfStar[];
  notes: string[];
  warnings: string[];
  elapsed_ms: number;
}

export type ResolutionPsfSource = "image" | "typed" | "header";

export interface ResolutionSettings {
  redshift: number | null;
  h0: number;
  omegaM: number;
  psfSource: ResolutionPsfSource;
  psfFwhmArcsec: number | null;
  eps: number;
  qResolved: number;
  minPixelsPerFwhm: number;
  scaleOfInterestKpc: number;
}
