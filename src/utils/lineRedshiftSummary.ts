import { formatWithUncertainty } from "./formatUncertainty";
import { formatQuantity } from "./lineMeasure";
import { combineRedshifts } from "./lineRedshift";
import type { LogValue, MeasurementLogEntry } from "./measurementLog";

export interface LineRedshiftSummary {
  n: number;
  z: number;
  zErr: number;
  zErrFormal: number;
  chi2: number;
  dof: number;
  labels: string[];
  text: string;
}

const SCATTER_SCALED_NOTE = ", error scaled by the scatter of the lines";

function finiteNumber(value: LogValue | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function nonEmptyText(value: LogValue | undefined): string | null {
  return typeof value === "string" && value !== "" ? value : null;
}

function hasUsableRedshift(entry: MeasurementLogEntry): boolean {
  const zErr = finiteNumber(entry.values.z_line_err);
  return finiteNumber(entry.values.z_line) !== null && zErr !== null && zErr > 0;
}

function lineKey(entry: MeasurementLogEntry): string {
  return nonEmptyText(entry.params.line_id) ?? `rest:${entry.params.rest_um}`;
}

function lineLabel(entry: MeasurementLogEntry): string {
  return nonEmptyText(entry.params.line_label) ?? nonEmptyText(entry.params.line_id) ?? `rest ${entry.params.rest_um} um`;
}

export function lineRedshiftSummary(
  entries: readonly MeasurementLogEntry[],
  fileName: string | null,
  source: string | null,
): LineRedshiftSummary | null {
  if (fileName === null || source === null) return null;
  const latestByLine = new Map<string, MeasurementLogEntry>();
  for (const entry of entries) {
    if (entry.kind !== "line" || entry.file !== fileName || entry.source !== source || !hasUsableRedshift(entry)) continue;
    const key = lineKey(entry);
    latestByLine.delete(key);
    latestByLine.set(key, entry);
  }
  const kept = [...latestByLine.values()];
  const combined = combineRedshifts(kept.map((entry) => ({ z: finiteNumber(entry.values.z_line), zErr: finiteNumber(entry.values.z_line_err) })));
  if (!combined || combined.n < 2) return null;
  const { n, z, zErr, zErrFormal, chi2, dof } = combined;
  const scaled = zErr > zErrFormal ? SCATTER_SCALED_NOTE : "";
  return {
    n,
    z,
    zErr,
    zErrFormal,
    chi2,
    dof,
    labels: kept.map(lineLabel),
    text: `Redshift from ${n} lines: z = ${formatWithUncertainty(z, zErr)} (chi2 ${formatQuantity(chi2)} for ${dof} dof)${scaled}`,
  };
}
