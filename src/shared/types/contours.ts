export type ContourMode = "list" | "linear" | "log" | "sqrt" | "sigma";

export const CONTOUR_MODES: readonly ContourMode[] = ["list", "linear", "log", "sqrt", "sigma"];

export interface ContourLevel {
  value: number;
  polylines: [number, number][][];
  closed: boolean[];
  n_points: number;
}

export interface ContourResult {
  levels: ContourLevel[];
  bin: number;
  n_points: number;
  background_median: number;
  background_sigma: number;
  notes: string[];
  masked: boolean;
  elapsed_ms: number;
}

export interface ContourOptions {
  mode: ContourMode;
  levels?: number[] | null;
  nLevels?: number | null;
  lo?: number | null;
  hi?: number | null;
  sigmaMultiples?: number[] | null;
  smoothSigma?: number | null;
  bin?: number | null;
  excludeDq?: boolean;
}
