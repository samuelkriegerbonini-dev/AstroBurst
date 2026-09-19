export type DbeMode = "subtract" | "divide";

export interface DbeConfig {
  sampleRadius: number;
  tolerance: number;
  smoothing: number;
  autoGrid: number | null;
  manualSamples: [number, number][];
  rejectStars: boolean;
  mode: DbeMode;
  normalize: boolean;
  maxSamples: number;
}

export interface DbeSample {
  x: number;
  y: number;
  value: number | null;
  weight: number;
  rejected: boolean;
  reason: string | null;
  manual: boolean;
}

export interface DbeResult {
  corrected_png: string;
  model_png: string;
  corrected_fits: string;
  cache_key: string;
  model_fits: string;
  sample_count: number;
  rejected_count: number;
  rms_residual: number;
  elapsed_ms: number;
  dimensions: [number, number];
  samples: DbeSample[];
  previewUrl?: string;
  modelUrl?: string;
}
