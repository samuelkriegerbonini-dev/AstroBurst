import type { DqFlagTable } from "../shared/types/dq";

export const STATISTICS_DESCRIPTION =
  "Exact per-pixel statistics (no histogram approximation). Negative values are included; exact zeros are counted as padding (0) and non-finite pixels as NaN, and both are left out of every statistic. avgDev, MAD and sqrt(BWMV) are robust scale estimates; the noise row uses the k-sigma multiresolution estimator.";

export const DQ_NOT_APPLIED_PREFIX = "DQ exclusion requested but not applied: ";

export function dqExclusionGap(input: {
  planeReason: string | null | undefined;
  flagTable: (Pick<DqFlagTable, "flags" | "exclusion_mask" | "label"> & { dq_unavailable?: string | null }) | null;
}): string {
  const { planeReason, flagTable } = input;
  if (planeReason) return planeReason;
  if (flagTable?.dq_unavailable) return flagTable.dq_unavailable;
  if (!flagTable) return "this image has no DQ plane";
  if (flagTable.exclusion_mask >>> 0 === 0) {
    return flagTable.flags.length === 0
      ? `the flag layer has no usable bit names (${flagTable.label}), so nothing is excluded until bits are chosen`
      : `no flag of this layer is selected for exclusion (${flagTable.label})`;
  }
  return "no flagged pixel could be excluded";
}
