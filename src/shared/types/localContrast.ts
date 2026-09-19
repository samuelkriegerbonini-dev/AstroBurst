export type LheHistogramBits = 8 | 10 | 12;

export interface LheConfig {
  kernelRadius: number;
  contrastLimit: number;
  amount: number;
  histBits: LheHistogramBits;
  circular: boolean;
}

export interface HdrConfig {
  layers: number;
  iterations: number;
  overdrive: number;
  inverted: boolean;
  toLightness: boolean;
  deringing: boolean;
  deringingAmount: number;
}

export interface LocalContrastResult {
  png_path: string;
  fits_path?: string;
  previewUrl?: string;
  dimensions: [number, number];
  elapsed_ms: number;
}

export const LHE_PROGRESS_EVENT = "lhe-progress";
export const HDR_PROGRESS_EVENT = "hdr-progress";

export const LHE_LIMITS = {
  kernelRadius: { min: 16, max: 512, step: 1 },
  contrastLimit: { min: 1, max: 64, step: 0.5 },
  amount: { min: 0, max: 1, step: 0.05 },
} as const;

export const HDR_LIMITS = {
  layers: { min: 2, max: 8, step: 1 },
  iterations: { min: 1, max: 4, step: 1 },
  overdrive: { min: 0, max: 1, step: 0.05 },
  deringingAmount: { min: 0, max: 1, step: 0.05 },
} as const;

export const DEFAULT_LHE_CONFIG: LheConfig = {
  kernelRadius: 64,
  contrastLimit: 2.0,
  amount: 1.0,
  histBits: 8,
  circular: true,
};

export const DEFAULT_HDR_CONFIG: HdrConfig = {
  layers: 6,
  iterations: 1,
  overdrive: 0.0,
  inverted: false,
  toLightness: true,
  deringing: false,
  deringingAmount: 0.5,
};
