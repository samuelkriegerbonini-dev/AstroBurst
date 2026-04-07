export interface CubeDims {
  width: number;
  height: number;
  frames: number;
  frame_count: number;
  bitpix?: number;
  axis_labels?: string[];
  wavelengths?: number[];
  spectral_classification?: {
    is_spectral: boolean;
    reason: string | null;
    axis_type?: string | null;
    axis_unit?: string | null;
    channel_count?: number;
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
