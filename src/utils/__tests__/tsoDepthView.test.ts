import { describe, expect, it } from "vitest";
import { bandText, type DepthRow, type DepthSpectrum } from "../tsoLightCurves";
import {
  DEPTH_BAND_COLOR,
  DEPTH_COLOR,
  bandBins,
  depthAxis,
  depthBandMarks,
  depthPlot,
  depthPlotBands,
  depthTableRows,
} from "../tsoDepthView";

function ROW(k: number, centre: number, depth: number | null, err: number | null, extra: Partial<DepthRow> = {}): DepthRow {
  const band: [number, number] = [centre - 0.25, centre + 0.25];
  return {
    docId: `bin:${k}`,
    label: bandText(band),
    band,
    centre,
    halfWidth: 0.25,
    depth,
    errScatter: err,
    errFormal: null,
    err,
    nIn: 6,
    nOut: 210,
    pixelRatio: null,
    pixelLoss: false,
    ...extra,
  };
}

function SPEC(rows: DepthRow[], white: DepthRow | null = null): DepthSpectrum {
  return { white, rows, window: { start: 1, end: 2 }, marginDays: 0, notes: [], warnings: [] };
}

const WHITE = { ...ROW(9, 3, 0.9504, 0.0063), docId: "white" };

describe("tsoDepthView", () => {
  it("draws the depths against the bin centres in wavelength order with the white light as a reference", () => {
    const rows = [ROW(1, 3.5, 0.9695, 0.003), ROW(0, 1.5, 0.9833, 0.0048)];

    expect(depthPlot(SPEC(rows, WHITE))).toEqual({
      series: [{ x: [1.5, 3.5], y: [0.9833, 0.9695], yErr: [0.0048, 0.003], color: "#fbbf24", label: "depth", mode: "points" }],
      referenceLines: [{ axis: "y", value: 0.9504, label: "white light", dashed: true }],
    });
    expect(DEPTH_COLOR).toBe("#fbbf24");
    expect(depthPlot(SPEC(rows))?.referenceLines).toEqual([]);
    expect(depthPlot(SPEC(rows, { ...WHITE, depth: null }))?.referenceLines).toEqual([]);
    expect(depthPlot(SPEC([ROW(0, 1.5, null, null), ROW(1, 3.5, null, null)], WHITE))).toBeNull();
    expect(depthPlot(SPEC([], WHITE))).toBeNull();

    const gapped = depthPlot(SPEC([ROW(2, 4.65, 0.8731, 0.0081), ROW(0, 0.8, 0.9674, null), ROW(1, 2.5, null, null)]));
    expect(gapped?.series).toHaveLength(1);
    expect(gapped?.series[0].x).toEqual([0.8, 2.5, 4.65]);
    expect(gapped?.series[0].y).toEqual([0.9674, null, 0.8731]);
    expect(gapped?.series[0].yErr).toEqual([null, null, 0.0081]);
    expect(rows.map((row) => row.centre)).toEqual([3.5, 1.5]);
  });

  it("words the table rows and marks a bin that lost pixels", () => {
    const spectrum = SPEC([ROW(1, 3.5, 0.9695, 0.003, { pixelRatio: 0.4386, pixelLoss: true }), ROW(0, 1.5, 0.9833, 0.0048)], WHITE);

    const rows = depthTableRows(spectrum);

    expect(rows.map((row) => row.band)).toEqual(["white light", "1.25 to 1.75 um", "3.25 to 3.75 um"]);
    expect(rows[0]).toEqual({ key: "white", band: "white light", depth: "0.9504", error: "0.0063", counts: "6 / 210", pixels: "--", pixelLoss: false });
    expect(rows[1]).toEqual({ key: "bin:0", band: "1.25 to 1.75 um", depth: "0.9833", error: "0.0048", counts: "6 / 210", pixels: "--", pixelLoss: false });
    expect(rows[2]).toMatchObject({ key: "bin:1", band: "3.25 to 3.75 um", depth: "0.9695", error: "0.003", pixels: "0.439", pixelLoss: true });

    const empty = depthTableRows(SPEC([ROW(2, 2.5, null, null, { nIn: 0, nOut: 12 })]));
    expect(empty).toEqual([{ key: "bin:2", band: "2.25 to 2.75 um", depth: "--", error: "--", counts: "0 / 12", pixels: "--", pixelLoss: false }]);
    expect(depthTableRows(SPEC([ROW(3, 3, 0.95, null)]))[0].error).toBe("--");
    expect(depthTableRows(SPEC([]))).toEqual([]);
  });

  it("builds a vacuum wavelength axis and asks the band table for the species shown", () => {
    expect(depthAxis(0.6, 5.3)).toEqual({
      kind: "wave",
      ctype: "WAVE",
      unit: "um",
      header_unit: "um",
      header_scale: 1,
      values: [0.6, 5.3],
      crval: 0.6,
      cdelt: 5.3 - 0.6,
      crpix: 1,
      rest_wavelength_um: null,
      rest_frequency_hz: null,
      specsys: null,
      velosys: null,
      notes: [],
      tabulated: true,
    });

    expect(depthBandMarks(["CO2"], 0.6, 5.3)).toEqual([{ id: "co2_4p3", label: "CO2 4.3", species: "CO2", kind: "molecular_band", lo: 4.15, hi: 4.5 }]);
    expect(depthBandMarks(["H2O"], 0.6, 5.3).map((mark) => [mark.id, mark.lo, mark.hi])).toEqual([["h2o_2p7", 2.5, 3]]);
    expect(depthBandMarks(["CO"], 0.6, 4.4).map((mark) => [mark.id, mark.lo, mark.hi])).toEqual([["co_4p7", 4.3, 4.4]]);
    expect(depthBandMarks([], 0.6, 5.3)).toEqual([]);
    expect(depthBandMarks(["CO2"], null, 5.3)).toEqual([]);
    expect(depthBandMarks(["CO2"], 0.6, null)).toEqual([]);
    expect(depthBandMarks(["CO2"], 5, 4)).toEqual([]);
    expect(depthBandMarks(["CO2"], 4.3, 4.3)).toEqual([]);
    expect(depthBandMarks(["CO2"], 0.6, Infinity)).toEqual([]);
    expect(depthBandMarks(["CO2"], Number.NaN, 5.3)).toEqual([]);
  });

  it("turns marks into shaded spans and molecular bands into bins", () => {
    const marks = depthBandMarks(["H2O", "CO2", "Na", "G395H gap"], 0.5, 5.3);

    expect(depthPlotBands(marks)).toEqual([
      { x0: 2.5, x1: 3, color: "rgba(103,232,249,0.10)", label: "H2O 2.7" },
      { x0: 4.15, x1: 4.5, color: "rgba(103,232,249,0.10)", label: "CO2 4.3" },
      { x0: 0.5891583264, x1: 0.5897558147, color: "rgba(103,232,249,0.10)", label: "Na I D" },
      { x0: 3.72, x1: 3.82, color: "rgba(103,232,249,0.10)", label: "G395H gap" },
    ]);
    expect(DEPTH_BAND_COLOR).toBe("rgba(103,232,249,0.10)");
    expect(bandBins(marks)).toEqual([
      [2.5, 3],
      [4.15, 4.5],
    ]);
    expect(bandBins(depthBandMarks(["CO2"], 0.6, 4.15))).toEqual([]);
    expect(depthPlotBands([])).toEqual([]);
    expect(bandBins([])).toEqual([]);
  });
});
