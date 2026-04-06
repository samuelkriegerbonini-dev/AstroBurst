export interface CubeDims {
  naxis3: number;
  frame_count: number;
  axis_labels?: string[];
  spectral_classification?: {
    is_spectral: boolean;
    reason: string | null;
  };
}

export interface CubeProcessResult {
  collapsed_path: string;
  collapsed_median_path?: string;
  collapsedPreviewUrl?: string;
  collapsedMedianPreviewUrl?: string;
  frame_count: number;
  elapsed_ms: number;
}

export interface CubeSpectrum {
  wavelengths: number[];
  values: number[];
  x: number;
  y: number;
  unit?: string;
}
