export interface PixelMathSlot {
  name: string;
  path: string;
}

export interface PixelMathStats {
  min: number;
  max: number;
  mean: number;
  median: number;
  sigma: number;
  mad: number;
  valid_count: number;
}

export interface PixelMathResult {
  png_path: string;
  fits_path: string;
  previewUrl?: string;
  dimensions: [number, number];
  elapsed_ms: number;
  stats: PixelMathStats | null;
  non_finite_count: number;
  warnings?: string[];
  cleaned_files?: number;
  cleaned_bytes?: number;
}

export interface PixelMathValidation {
  ok: boolean;
  message: string | null;
  position: number | null;
  length: number | null;
}
