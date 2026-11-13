import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { LogValue, MeasurementLogEntry } from "../../../utils/measurementLog";
import { formatWithUncertainty } from "../../../utils/formatUncertainty";
import { formatQuantity } from "../../../utils/lineMeasure";
import { lineRedshiftSummary } from "../../../utils/lineRedshiftSummary";

const log = vi.hoisted(() => ({ entries: [] as readonly unknown[] }));

vi.mock("../../../hooks/useMeasurementLog", () => ({ useMeasurementLog: () => log.entries }));

import LineRedshiftSummary from "../LineRedshiftSummary";

const SRC = '{"kind":"file","hdu":1}';

function lineAt(lineId: string, lineLabel: string, z: number, zErr: number): MeasurementLogEntry {
  const params: Record<string, LogValue> = { line_id: lineId, line_label: lineLabel, rest_um: 0.6 };
  return {
    id: lineId,
    timestamp_utc: "2026-11-13T00:00:00.000Z",
    kind: "line",
    file: "a.fits",
    image: "spectrum file",
    dq: "off",
    unit: "Jy x um",
    source: SRC,
    params,
    values: { z_line: z, z_line_err: zErr },
    notes: [],
  };
}

const THREE_LINES = [
  lineAt("h_alpha", "Hα", 0.5000891142048043, 4.5699592207305544e-5),
  lineAt("oiii_5007", "[OIII] 5007", 0.5000678881203775, 3.993418845742217e-5),
  lineAt("h_beta", "Hβ", 0.5000575814160093, 1.028239571594265e-4),
];

function render(entries: readonly MeasurementLogEntry[], fileName: string | null, source: string | null): string {
  log.entries = entries;
  return renderToStaticMarkup(createElement(LineRedshiftSummary, { fileName, source }));
}

describe("LineRedshiftSummary", () => {
  it("renders the combined redshift and the lines", () => {
    const html = render(THREE_LINES, "a.fits", SRC);
    const summary = lineRedshiftSummary(THREE_LINES, "a.fits", SRC)!;
    expect(summary.text).toBe(
      `Redshift from 3 lines: z = ${formatWithUncertainty(summary.z, summary.zErr)} (chi2 ${formatQuantity(summary.chi2)} for 2 dof)`,
    );
    expect(html).toContain(summary.text);
    expect(html).toContain("lines: Hα, [OIII] 5007, Hβ");
  });

  it("renders nothing with fewer than two lines, without a file or for another source", () => {
    expect(render(THREE_LINES.slice(0, 1), "a.fits", SRC)).toBe("");
    expect(render(THREE_LINES, null, SRC)).toBe("");
    expect(render(THREE_LINES, "a.fits", null)).toBe("");
    expect(render(THREE_LINES, "a.fits", '{"kind":"file","hdu":2}')).toBe("");
  });
});
