import type { CompareRow } from "../../shared/types/catalogCompare";

export function reasonText(row: Pick<CompareRow, "reason" | "masked_fraction">): string {
  if (row.reason === null) return "";
  if (typeof row.masked_fraction === "number" && row.masked_fraction > 0) {
    return `${row.reason} (${(100 * row.masked_fraction).toFixed(1)} percent of the aperture)`;
  }
  return row.reason;
}
