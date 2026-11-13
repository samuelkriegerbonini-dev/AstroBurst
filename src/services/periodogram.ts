import { typedInvoke } from "../infrastructure/tauri";
import type { PeriodogramOptions, PeriodogramResult } from "../shared/types/periodogram";

export type PeriodogramUpperLimit = "given" | "cadence_nyquist" | "five_mean_nyquist";

export interface PeriodogramSearch extends PeriodogramResult {
  first_frequency: number;
  last_frequency: number;
  upper_limit: PeriodogramUpperLimit;
  cadence_days: number | null;
}

export function lightCurvePeriodogram(
  time: number[],
  value: number[],
  err: number[] | null,
  options: PeriodogramOptions = {},
): Promise<PeriodogramSearch> {
  return typedInvoke<PeriodogramSearch>("light_curve_periodogram_cmd", {
    time,
    value,
    err,
    minFrequency: options.minFrequency ?? null,
    maxFrequency: options.maxFrequency ?? null,
    samplesPerPeak: options.samplesPerPeak ?? null,
  });
}
