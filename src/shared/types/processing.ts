export interface DeconvolveResult {
  png_path: string;
  fits_path?: string;
  previewUrl?: string;
  dimensions: [number, number];
  elapsed_ms: number;
  iterations_run: number;
  convergence?: number;
}

export interface BackgroundResult {
  corrected_png: string;
  corrected_fits?: string;
  cache_key?: string;
  model_png: string;
  previewUrl?: string;
  modelUrl?: string;
  dimensions: [number, number];
  sample_count?: number;
  rms_residual?: number;
  elapsed_ms: number;
  axis?: "rows" | "cols" | "both" | null;
}

export interface WaveletResult {
  png_path: string;
  fits_path?: string;
  previewUrl?: string;
  dimensions: [number, number];
  scales_processed?: number;
  noise_estimate?: number;
  elapsed_ms: number;
}

export interface PsfStar {
  x: number;
  y: number;
  peak: number;
  flux: number;
  fwhm: number;
  ellipticity: number;
  snr: number;
}

export interface PsfEstimate {
  kernel_size: number;
  average_fwhm: number;
  average_ellipticity: number;
  spread_pixels: number;
  stars_used: PsfStar[];
  stars_rejected: number;
  kernel: number[][] | null;
}

export type PsfStarSource =
  | { kind: "regions"; path: string }
  | { kind: "ra_dec"; text: string }
  | { kind: "pixels"; stars: { name: string; x: number; y: number }[] }
  | { kind: "auto"; num_stars?: number | null };

export type PsfStackMethod = "median" | "mean";

export type PsfRadiusUnit = "pixel" | "arcsec";

export interface EmpiricalPsfOptions {
  cutout_size?: number;
  background_border?: number;
  background_sigma?: number;
  clip_negative?: boolean;
  noise_threshold?: number | null;
  radial_cleaning?: boolean;
  cleaning_sigma?: number;
  cleaning_consecutive_bins?: number;
  normalization_radius?: number;
  normalization_radius_unit?: PsfRadiusUnit;
  psf_size?: number;
  oversampling?: number | null;
  stack?: PsfStackMethod;
}

export interface EmpiricalPsfStar {
  name: string;
  x: number;
  y: number;
  used: boolean;
  rejected: string | null;
  centroid: "quadratic" | "peak" | null;
  centre_x: number | null;
  centre_y: number | null;
  background: number | null;
  background_std: number | null;
  cleaning_radius: number | null;
  aperture_flux: number | null;
  profile: number[];
}

export interface EmpiricalPsfReport {
  source: PsfStarSource["kind"];
  filter: string | null;
  dataset: string | null;
  pixel_scale_arcsec: number | null;
  normalization_radius_px: number;
  psf_size: number;
  fwhm_px: number | null;
  fwhm_arcsec: number | null;
  stars_used: number;
  stars_rejected: number;
  stars: EmpiricalPsfStar[];
  kernel: number[][];
  profile: number[];
  oversampling: number | null;
  oversampled_size: number | null;
  output_path: string | null;
  oversampled_path: string | null;
  warnings: string[];
  elapsed_ms: number;
}

export interface ArcsinhResult {
  png_path: string;
  fits_path?: string;
  previewUrl?: string;
  stretch_factor?: number;
  dimensions: [number, number];
  elapsed_ms: number;
}

export interface MaskedStretchChannelStats {
  iterations_run: number;
  final_background: number;
  converged: boolean;
}

export interface MaskedStretchResult {
  png_path: string;
  previewUrl?: string;
  dimensions: [number, number];
  elapsed_ms: number;
  iterations_run?: number;
  converged?: boolean;
  channels?: { r: MaskedStretchChannelStats; g: MaskedStretchChannelStats; b: MaskedStretchChannelStats };
}

export interface SpccResult {
  r_factor: number;
  g_factor: number;
  b_factor: number;
  stars_matched: number;
  stars_total: number;
  elapsed_ms: number;
  avg_color_index?: number;
  white_reference?: string;
  catalog_name?: string;
  is_synthetic_catalog?: boolean;
  wavelengths_nm?: [number, number, number];
  wavelength_source?: "filters" | "default";
  catalog_source?: string | null;
}

export interface StarDetectionResult {
  stars: Array<{
    x: number;
    y: number;
    flux: number;
    fwhm: number;
    snr: number;
    eccentricity?: number;
    peak?: number;
    npix?: number;
  }>;
  background_median?: number;
  background_sigma?: number;
  threshold_sigma?: number;
  image_width?: number;
  image_height?: number;
  elapsed_ms: number;
  n_detected?: number;
}

export const BACKGROUND_PROGRESS_EVENT = "background-progress";
export const WAVELET_PROGRESS_EVENT = "wavelet-progress";
export const DECONV_PROGRESS_EVENT = "deconv-progress";
