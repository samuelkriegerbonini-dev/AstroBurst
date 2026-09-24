export interface ChannelStatistics {
  count: number;
  total: number;
  fraction: number;
  mean: number;
  median: number;
  avg_dev: number;
  mad: number;
  bwmv_sqrt: number;
  min: number;
  max: number;
  sum: number;
  variance: number;
  std_dev: number;
  nan_count: number;
  padding: number;
  excluded: number;
}

export interface NoiseEvaluation {
  sigma: number;
  fraction: number;
  iterations: number;
  method: string;
}

export interface ChannelStatisticsBody {
  statistics: ChannelStatistics;
  noise: NoiseEvaluation | null;
  noise_note: string | null;
  data_min: number | null;
  data_max: number | null;
}

export interface StatisticsResult extends ChannelStatisticsBody {
  unit: string | null;
  masked: boolean;
  dq_excluded: number | null;
  elapsed_ms: number;
}

export interface CompositeStatisticsResult {
  r: ChannelStatisticsBody;
  g: ChannelStatisticsBody;
  b: ChannelStatisticsBody;
  elapsed_ms: number;
}

export interface NoiseBatchEntry {
  path: string;
  sigma: number | null;
  fraction: number | null;
  error: string | null;
}

export interface NoiseBatchResult {
  results: NoiseBatchEntry[];
  elapsed_ms: number;
}

export interface SubframeNoiseMetrics {
  noise_sigma: number;
  noise_fraction: number;
}

export type StatisticsUnit = "raw" | "normalized" | "16bit";

export interface DataRange {
  min: number | null;
  max: number | null;
}
