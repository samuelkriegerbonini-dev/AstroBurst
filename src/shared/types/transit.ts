export type TransitKind = "transit" | "eclipse";
export type TransitParameterName = "t0" | "rp" | "a_rs" | "b" | "u1" | "u2" | "flux_ratio";
export interface TransitParameterInput { value: number; free: boolean }
export interface TransitFitRequest {
  kind?: TransitKind;
  time: number[];
  flux: number[];
  err: number[] | null;
  period: number;
  t0: TransitParameterInput;
  rp: TransitParameterInput;
  a_rs: TransitParameterInput;
  b: TransitParameterInput;
  u1: TransitParameterInput;
  u2: TransitParameterInput;
  flux_ratio?: TransitParameterInput | null;
  baseline_order: 0 | 1 | 2;
  scale_errors: boolean;
}
export interface TransitFittedParameter { name: TransitParameterName; value: number; err: number | null; free: boolean; at_bound: boolean }
export interface TransitFittedBaseline { order: number; reference_time: number; coefficients: number[]; errors: (number | null)[] }
export interface TransitDerived {
  inclination_deg: number | null; inclination_err: number | null;
  duration_t14_days: number | null; duration_t14_err: number | null;
  duration_t23_days: number | null; duration_t23_err: number | null;
  stellar_density_cgs: number | null; stellar_density_err: number | null;
}
export interface TransitTimeAveraging { cadence_days: number; bin_points: number[]; n_bins: number[]; rms_ppm: number[]; expected_ppm: number[]; beta: number }
export interface TransitFitResult {
  model: string;
  period: number;
  parameters: TransitFittedParameter[];
  baseline: TransitFittedBaseline;
  depth: number;
  depth_err: number | null;
  chi2: number;
  dof: number;
  reduced_chi2: number;
  weighted: boolean;
  error_scale: number;
  rms: number;
  rms_ppm: number;
  n_points: number;
  n_in_transit: number;
  iterations: number;
  converged: boolean;
  free_names: string[];
  time: number[];
  model_flux: number[];
  baseline_flux: number[];
  residual: number[];
  notes: string[];
  warnings: string[];
  correlation: number[][] | null;
  derived: TransitDerived;
  time_averaging: TransitTimeAveraging;
  curve_time: number[];
  curve_flux: number[];
  curve_baseline: number[];
  kind: TransitKind;
  elapsed_ms: number;
}
export interface TransitFitOptions { includeSeries?: boolean }
