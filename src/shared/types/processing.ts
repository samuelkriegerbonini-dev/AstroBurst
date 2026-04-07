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
  model_png: string;
  previewUrl?: string;
  modelUrl?: string;
  dimensions: [number, number];
  elapsed_ms: number;
}

export interface WaveletResult {
  png_path: string;
  previewUrl?: string;
  dimensions: [number, number];
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

export interface ArcsinhResult {
  png_path: string;
  previewUrl?: string;
  dimensions: [number, number];
  elapsed_ms: number;
}

export interface MaskedStretchResult {
  png_path: string;
  previewUrl?: string;
  dimensions: [number, number];
  elapsed_ms: number;
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
}

export interface StarDetectionResult {
  stars: Array<{
    x: number;
    y: number;
    flux: number;
    fwhm?: number;
    snr?: number;
  }>;
  count: number;
  elapsed_ms: number;
}
