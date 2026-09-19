export type CosmeticReplacement = "median" | "mean";

export type CosmeticDefect =
  | { kind: "point"; x: number; y: number }
  | { kind: "column"; x: number; y0: number | null; y1: number | null }
  | { kind: "row"; y: number; x0: number | null; x1: number | null };

export interface CosmeticConfig {
  use_master_dark: boolean;
  dark_hot_sigma: number | null;
  dark_cold_sigma: number | null;
  auto_hot_sigma: number | null;
  auto_cold_sigma: number | null;
  defects: CosmeticDefect[];
  cfa: boolean;
  amount: number;
  replacement: CosmeticReplacement;
}

export interface CosmeticCounts {
  flagged: number;
  replaced: number;
  hot: number;
  cold: number;
  listed: number;
}

export interface CosmeticResult {
  path: string;
  png_path: string;
  fits_path?: string | null;
  previewUrl?: string;
  dimensions: [number, number];
  counts: CosmeticCounts;
  dq_present: boolean;
  warning?: string | null;
  elapsed_ms: number;
}

export interface CosmeticBatchItem {
  path: string;
  png_path?: string;
  fits_path?: string | null;
  dimensions?: [number, number];
  counts?: CosmeticCounts;
  dq_present?: boolean;
  warning?: string | null;
  error?: string;
}

export interface CosmeticBatchResult {
  results: CosmeticBatchItem[];
  succeeded: number;
  failed: number;
  elapsed_ms: number;
}

export interface CosmeticOptions {
  masterDarkPath?: string | null;
  config: CosmeticConfig;
  defectListText?: string | null;
}
