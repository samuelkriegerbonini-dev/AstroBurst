import { describe, expect, it } from "vitest";
import { lineRedshiftSummary } from "../lineRedshiftSummary";
import { combineRedshifts } from "../lineRedshift";
import { formatQuantity } from "../lineMeasure";
import { formatWithUncertainty } from "../formatUncertainty";
import type { LogValue, MeasurementLogEntry } from "../measurementLog";

const SRC = '{"kind":"file","hdu":1}';

function logEntry(
  params: Record<string, LogValue>,
  values: Record<string, LogValue>,
  extra: Partial<MeasurementLogEntry> = {},
): MeasurementLogEntry {
  return {
    id: "line",
    timestamp_utc: "2026-11-13T00:00:00.000Z",
    kind: "line",
    file: "a.fits",
    image: "spectrum file",
    dq: "off",
    unit: "Jy x um",
    source: SRC,
    params: { rest_um: 0.6, ...params },
    values,
    notes: [],
    ...extra,
  };
}

function lineAt(
  lineId: string | null,
  lineLabel: string | null,
  z: number | null,
  zErr: number | null,
  extra: Partial<MeasurementLogEntry> = {},
): MeasurementLogEntry {
  return logEntry({ line_id: lineId, line_label: lineLabel }, { z_line: z, z_line_err: zErr }, extra);
}

const H_ALPHA = lineAt("h_alpha", "Hα", 0.5000891142048043, 4.5699592207305544e-5);
const OIII = lineAt("oiii_5007", "[OIII] 5007", 0.5000678881203775, 3.993418845742217e-5);
const H_BETA = lineAt("h_beta", "Hβ", 0.5000575814160093, 1.028239571594265e-4);
const THREE_LINES = [H_ALPHA, OIII, H_BETA];

describe("lineRedshiftSummary", () => {
  it("combines the latest measurement of each line of the file", () => {
    const summary = lineRedshiftSummary(THREE_LINES, "a.fits", SRC)!;
    expect(summary.n).toBe(3);
    expect(Math.abs(summary.z - 0.5000755423957206)).toBeLessThan(1e-12);
    expect(Math.abs(summary.zErr / 2.8861889458078432e-5 - 1)).toBeLessThan(1e-9);
    expect(Math.abs(summary.chi2 - 0.155446694624214)).toBeLessThan(1e-6);
    expect(summary.dof).toBe(2);
    expect(summary.labels).toEqual(["Hα", "[OIII] 5007", "Hβ"]);
    expect(summary.text).toBe(
      `Redshift from 3 lines: z = ${formatWithUncertainty(summary.z, summary.zErr)} (chi2 ${formatQuantity(summary.chi2)} for 2 dof)`,
    );
  });

  it("keeps the last of two measurements of one line", () => {
    const earlier = lineAt("h_alpha", "Hα", 0.6, 4.5699592207305544e-5);
    expect(lineRedshiftSummary([earlier, ...THREE_LINES], "a.fits", SRC)).toEqual(lineRedshiftSummary(THREE_LINES, "a.fits", SRC));
  });

  it("ignores other files, other kinds and entries without a redshift", () => {
    const noise = [
      lineAt("oiii_5007", "[OIII] 5007", 0.5000678881203775, 3.993418845742217e-5, { file: "b.fits" }),
      lineAt("h_beta", "Hβ", 0.5000575814160093, 1.028239571594265e-4, { kind: "line_fit" }),
      lineAt("pa_alpha", "Pa alpha", null, 4e-5),
      lineAt("br_alpha", "Br alpha", 0.50007, 0),
    ];
    expect(lineRedshiftSummary([H_ALPHA, ...noise], "a.fits", SRC)).toBeNull();
    expect(lineRedshiftSummary([...noise, ...THREE_LINES], "a.fits", SRC)!.n).toBe(3);
    expect(lineRedshiftSummary(THREE_LINES, null, SRC)).toBeNull();
    expect(lineRedshiftSummary(THREE_LINES, "a.fits", null)).toBeNull();
  });

  it("never combines lines measured on two sources of one file", () => {
    const first = '{"kind":"pixel","x":3,"y":4}';
    const second = '{"kind":"pixel","x":30,"y":12}';
    const alpha = lineAt("h_alpha", "Hα", 0.5000891142048043, 4.5699592207305544e-5, { source: first });
    const oiii = lineAt("oiii_5007", "[OIII] 5007", 0.5000678881203775, 3.993418845742217e-5, { source: second });
    expect(lineRedshiftSummary([alpha, oiii], "a.fits", first)).toBeNull();
    expect(lineRedshiftSummary([alpha, oiii], "a.fits", second)).toBeNull();
    const together = lineRedshiftSummary([alpha, { ...oiii, source: first }], "a.fits", first)!;
    expect(together.n).toBe(2);
    expect(together.dof).toBe(1);
    expect(together.labels).toHaveLength(2);
    const expected = combineRedshifts([
      { z: 0.5000891142048043, zErr: 4.5699592207305544e-5 },
      { z: 0.5000678881203775, zErr: 3.993418845742217e-5 },
    ])!;
    expect(Math.abs(together.z - expected.z)).toBeLessThanOrEqual(1e-15);
  });

  it("falls back to the rest wavelength when no line was picked", () => {
    const a = logEntry({ line_id: null, line_label: null, rest_um: 0.656461 }, { z_line: 0.5000891142048043, z_line_err: 4.5699592207305544e-5 });
    const b = logEntry({ line_id: null, line_label: null, rest_um: 0.486268 }, { z_line: 0.5000575814160093, z_line_err: 1.028239571594265e-4 });
    const summary = lineRedshiftSummary([a, b], "a.fits", SRC)!;
    expect(summary.n).toBe(2);
    expect(summary.labels).toEqual(["rest 0.656461 um", "rest 0.486268 um"]);
  });

  it("says when the scatter scaled the error", () => {
    const entries = [lineAt("a", "A", 0.5, 2e-4), lineAt("b", "B", 0.501, 2e-4), lineAt("c", "C", 0.5004, 2e-4)];
    const summary = lineRedshiftSummary(entries, "a.fits", SRC)!;
    expect(Math.abs(summary.zErr / 2.9059326290271356e-4 - 1)).toBeLessThan(1e-9);
    expect(summary.zErr).toBeGreaterThan(summary.zErrFormal);
    expect(summary.text.endsWith(", error scaled by the scatter of the lines")).toBe(true);
    expect(lineRedshiftSummary(THREE_LINES, "a.fits", SRC)!.text).not.toContain("error scaled");
  });
});
