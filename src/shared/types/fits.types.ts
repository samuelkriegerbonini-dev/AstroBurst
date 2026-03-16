import type { FileStatus } from "./queue.types";
import type { HistogramData } from "./analysis.types";

export interface StfParams {
  shadow: number;
  midtone: number;
  highlight: number;
}

export interface AstroFile {
  name: string;
  path: string;
  size: number;
  lastModified?: number;
}

export interface ResampleResult {
  png_path: string;
  fits_path: string;
  previewUrl?: string;
  dimensions: [number, number];
  original_dimensions: [number, number];
  wcs_updates: Record<string, any>;
  stats: {
    min: number;
    max: number;
    mean: number;
    sigma: number;
  };
}

export interface ProcessResult {
  png_path: string;
  previewUrl: string;
  dimensions: [number, number];
  elapsed_ms: number;
  header?: Record<string, string> | null;
  histogram?: HistogramData | null;
  stf?: StfParams | null;
  stats?: {
    min: number;
    max: number;
    mean: number;
    sigma: number;
    median: number;
    mad?: number;
  } | null;
  original_dimensions?: [number, number];
  wcs_updates?: Record<string, any>;
  resampled?: ResampleResult | null;
  resampledPath?: string | null;
}

export interface ProcessedFile {
  id: string;
  name: string;
  path: string;
  size: number;
  status: FileStatus;
  result: ProcessResult | null;
  error: string | null;
  startedAt: number | null;
  finishedAt: number | null;
}
