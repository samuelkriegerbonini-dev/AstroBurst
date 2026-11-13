export interface PeriodogramResult {
  frequency: number[];
  power: number[];
  best_frequency: number;
  best_period: number;
  best_power: number;
  n_points: number;
  n_dropped: number;
  n_frequencies: number;
  baseline_days: number;
  frequency_step: number;
  normalization: "standard";
  weighted: boolean;
  false_alarm_probability: number | null;
  fap_method: "baluev";
  notes: string[];
  elapsed_ms: number;
}

export interface PeriodogramOptions {
  minFrequency?: number | null;
  maxFrequency?: number | null;
  samplesPerPeak?: number | null;
}
