import type { StatisticsUnit } from "../shared/types/statistics";
import { exportEntries, provenanceLines, withProvenance, type ProvenanceEntry } from "./csvProvenance";

export interface StatisticsCsvInput {
  table: string;
  file: string | null;
  region: string;
  bunit: string | null;
  displayUnit: StatisticsUnit;
  dqExcluded: number | null;
  dqGap: string | null;
  noiseMethod: string | null;
  exportedUtc: string;
}

const STATISTICS_LINE_END = "\n";

export function statisticsClipboardCsv(input: StatisticsCsvInput): string {
  const entries: ProvenanceEntry[] = [...exportEntries(input.file, input.exportedUtc), ["region", input.region]];
  if (input.dqExcluded !== null) entries.push(["dq_excluded", input.dqExcluded]);
  if (input.dqGap !== null) entries.push(["dq_exclusion", `requested but not applied: ${input.dqGap}`]);
  if (input.noiseMethod !== null) entries.push(["noise_method", input.noiseMethod]);
  entries.push(["bunit", input.bunit], ["display_unit", input.displayUnit]);
  return withProvenance(provenanceLines(entries), input.table, STATISTICS_LINE_END);
}
