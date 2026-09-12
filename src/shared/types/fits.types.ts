import type { FileStatus } from "./queue";
import type { HistogramData } from "./analysis";
import type { DqTableName } from "./dq";

export type PlaneKind = "hdu" | "array";

export interface PlaneInfo {
  kind: PlaneKind;
  index: number | null;
  key: string | null;
  extname: string | null;
  extver: number | null;
  is_dq: boolean;
  is_err: boolean;
  dq_ref: string | null;
  err_ref: string | null;
  source_path: string;
  dq_table: DqTableName | null;
}

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
  wcs_updates: Record<string, unknown>;
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
  wcs_updates?: Record<string, unknown>;
  resampled?: ResampleResult | null;
  resampledPath?: string | null;
  is_rgb?: boolean;
  stf_r?: StfParams | null;
  stf_g?: StfParams | null;
  stf_b?: StfParams | null;
  image_ref?: string;
  plane?: PlaneInfo | null;
}

export interface ProcessedFile {
  id: string;
  name: string;
  path: string;
  sourcePath: string;
  imageRef: string | null;
  size: number;
  status: FileStatus;
  result: ProcessResult | null;
  error: string | null;
  startedAt: number | null;
  finishedAt: number | null;
}
