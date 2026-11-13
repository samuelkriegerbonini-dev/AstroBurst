import type { SeriesKey, TsoCurveMode, TsoTimeScale } from "./tso";

export type TsoLimbChoice = "fixed" | "free";

export interface TsoTransitFitRequest {
  period_days: number;
  t0: number;
  a_rs: number;
  b: number;
  rp_start: number;
  u1: number;
  u2: number;
  limb_darkening?: TsoLimbChoice;
  baseline_order?: 0 | 1 | 2;
  exclude_rows?: number[];
  scale_errors?: boolean;
}

export interface TsoTransitBin {
  band_um: [number, number];
  n_points: number;
  n_in_transit: number;
  weighted: boolean;
  rp: number | null;
  rp_err: number | null;
  depth: number | null;
  depth_err: number | null;
  u1: number | null;
  u1_err: number | null;
  u2: number | null;
  u2_err: number | null;
  chi2: number | null;
  dof: number;
  rms: number | null;
  grazing: boolean;
  npixels_ratio: number | null;
  pixel_loss: boolean;
  error: string | null;
  notes: string[];
  warnings: string[];
}

export interface TsoTransitSpectrum {
  set_id: string;
  label: string;
  series: SeriesKey;
  mode: TsoCurveMode;
  time_scale: TsoTimeScale;
  fit: Required<TsoTransitFitRequest>;
  model: string;
  n_integrations: number;
  n_excluded: number;
  n_in_transit: number;
  bins: TsoTransitBin[];
  notes: string[];
  warnings: string[];
}
