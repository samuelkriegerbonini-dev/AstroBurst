import { typedInvoke } from "../infrastructure/tauri";
import type { RegionShape } from "../shared/types/regions";
import type {
  StatisticsResult,
  CompositeStatisticsResult,
  NoiseBatchResult,
} from "../shared/types/statistics";

export interface StatisticsOptions {
  excludeDq?: boolean;
  region?: RegionShape | null;
  noise?: boolean;
}

export function computeStatistics(path: string, opts: StatisticsOptions = {}): Promise<StatisticsResult> {
  return typedInvoke<StatisticsResult>("compute_statistics_cmd", {
    path,
    excludeDq: opts.excludeDq ?? false,
    region: opts.region ?? null,
    noise: opts.noise ?? false,
  });
}

export function computeStatisticsComposite(noise = false, rgbPath: string | null = null): Promise<CompositeStatisticsResult> {
  return typedInvoke<CompositeStatisticsResult>("compute_statistics_composite_cmd", { noise, path: rgbPath });
}

export function evaluateNoiseBatch(paths: string[]): Promise<NoiseBatchResult> {
  return typedInvoke<NoiseBatchResult>("evaluate_noise_batch_cmd", { paths });
}
