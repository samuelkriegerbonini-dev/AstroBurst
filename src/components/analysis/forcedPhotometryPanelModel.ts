import type { ForcedPhotometryResult } from "../../shared/types/forcedPhotometry";

const counted = (count: number, word: string): string => `${count} ${word}${count === 1 ? "" : "s"}`;

export function runLabel(nPositions: number, nBands: number): string {
  return `Measure ${counted(nPositions, "position")} in ${counted(nBands, "band")}`;
}

export function runSummary(result: ForcedPhotometryResult): string {
  const seconds = (result.elapsed_ms / 1000).toFixed(1);
  return `${counted(result.sources.length, "position")} x ${counted(result.bands.length, "band")} in ${seconds} s, limits at ${String(result.limit_sigma)} sigma`;
}

export function progressText(progress: { current: number; total: number; stage: string }): string {
  const { current, total, stage } = progress;
  if (total <= 0 || stage === "" || stage === "complete") return "starting";
  return `${current} of ${counted(total, "band")} measured, last ${stage}`;
}
