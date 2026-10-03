import { describe, it, expect } from "vitest";
import {
  COMPARISON_PALETTE,
  MAX_COMPARISON_REGIONS,
  NORMALISE_MODES,
  NO_SURF_BRIGHT_REASON,
  PIXEL_SERIES_COLOR,
  X1D_NO_WINDOW_DATA_REASON,
  comparisonCsv,
  comparisonCsvFileName,
  comparisonSeries,
  entriesFrom,
  entryValues,
  finiteMedian,
  limitCandidates,
  normaliseFactor,
  offsetStepAuto,
  plottedTable,
  pruneEntries,
  type ComparisonCsvInput,
  type ComparisonEntry,
} from "../spectrumCompare";
import { comparisonAxis, comparisonCandidates, vacuumUmAxis } from "../spectrumExport";
import { APERTURE_NOTE, type TableEntryData } from "../x1dCompare";
import type { CubeSpectrum, RegionSpectrum } from "../../shared/types/cube";
import type { Region, RegionShape } from "../../shared/types/regions";
import type { SpectralAxisInfo, SpectralAxisKind } from "../../shared/types/spectral";

const NO_JY = "no Jy calibration (BUNIT is not MJy/sr)";

function axisOf(kind: SpectralAxisKind, values: number[], extra: Partial<SpectralAxisInfo> = {}): SpectralAxisInfo {
  return {
    kind,
    ctype: kind.toUpperCase(),
    unit: kind === "freq" ? "GHz" : kind === "wave" || kind === "awav" ? "um" : "km/s",
    header_unit: "",
    header_scale: 1,
    values,
    crval: values[0] ?? 0,
    cdelt: values.length > 1 ? values[1] - values[0] : 0,
    crpix: 1,
    rest_wavelength_um: null,
    rest_frequency_hz: null,
    specsys: null,
    velosys: null,
    notes: [],
    ...extra,
  };
}

function regionOf(id: string, shape: RegionShape, extra: Partial<Region["props"]> = {}, backgroundId: string | null = null): Region {
  return { id, shape, props: { color: null, width: null, text: null, dash: null, include: true, ...extra }, backgroundId };
}

const circle: RegionShape = { shape: "circle", x: 20, y: 20, r: 3 };
const box: RegionShape = { shape: "box", x: 5, y: 5, width: 4, height: 4, angle: 0 };
const annulus: RegionShape = { shape: "annulus", x: 20, y: 20, r_inner: 5, r_outer: 8 };

function regionSpectrum(sum: number[], extra: Partial<RegionSpectrum> = {}): RegionSpectrum {
  return {
    sum,
    mean: sum.map((v) => v / 10),
    npix: 10,
    n_bg: 0,
    bg_per_pixel: null,
    wavelengths: null,
    unit: "um",
    bg_subtracted: false,
    flux_jy: null,
    elapsed_ms: 1,
    ...extra,
  };
}

function pixelSpectrum(values: number[], fluxJy: number[] | null = null): CubeSpectrum {
  return { values, wavelengths: [], x: 3, y: 4, flux_jy: fluxJy };
}

function regionEntry(id: string, sum: number[], extra: Partial<ComparisonEntry> = {}, spectrum: Partial<RegionSpectrum> = {}): ComparisonEntry {
  return {
    id,
    kind: "region",
    label: id,
    color: "#111111",
    source: { kind: "region", shape: circle, background: null },
    regionText: null,
    backgroundId: null,
    region: regionSpectrum(sum, spectrum),
    pixel: null,
    table: null,
    error: null,
    ...extra,
  };
}

function pixelEntry(values: number[], fluxJy: number[] | null = null, extra: Partial<ComparisonEntry> = {}): ComparisonEntry {
  return {
    id: "pixel",
    kind: "pixel",
    label: "pixel (3, 4)",
    color: PIXEL_SERIES_COLOR,
    source: { kind: "pixel", x: 3, y: 4 },
    regionText: null,
    backgroundId: null,
    region: null,
    pixel: pixelSpectrum(values, fluxJy),
    table: null,
    error: null,
    ...extra,
  };
}

function fulfilled<T>(value: T): PromiseSettledResult<T> {
  return { status: "fulfilled", value };
}

function rejected<T>(reason: unknown): PromiseSettledResult<T> {
  return { status: "rejected", reason };
}

const axis3 = [0, 1, 2];

describe("entry values and normalisation", () => {
  it("entryValues picks sum, mean or Jy for a region and values or Jy for a pixel, null without Jy", () => {
    const region = regionEntry("r1", [10, 20, 30], {}, { flux_jy: [1, 2, 3] });
    expect(entryValues(region, "sum")).toEqual([10, 20, 30]);
    expect(entryValues(region, "mean")).toEqual([1, 2, 3]);
    expect(entryValues(region, "jy")).toEqual([1, 2, 3]);
    expect(entryValues(regionEntry("r2", [1, 2, 3]), "jy")).toBeNull();
    const pixel = pixelEntry([4, 5, 6], [7, 8, 9]);
    expect(entryValues(pixel, "sum")).toEqual([4, 5, 6]);
    expect(entryValues(pixel, "mean")).toEqual([4, 5, 6]);
    expect(entryValues(pixel, "jy")).toEqual([7, 8, 9]);
    expect(entryValues(pixelEntry([4, 5, 6]), "jy")).toBeNull();
    expect(entryValues(regionEntry("r3", [], { region: null }), "sum")).toBeNull();
  });

  it("normaliseFactor peak divides by the largest finite value and refuses a non-positive peak", () => {
    expect(normaliseFactor([1, 5, NaN, 3], "peak", null)).toEqual({ factor: 5 });
    expect(normaliseFactor([-1, -2, -0.5], "peak", null)).toEqual({ reason: "peak is not positive" });
    expect(normaliseFactor([0, 0], "peak", null)).toEqual({ reason: "peak is not positive" });
    expect(normaliseFactor([1, Infinity], "peak", null)).toEqual({ factor: 1 });
  });

  it("normaliseFactor median ignores NaN and refuses a zero or negative median", () => {
    expect(normaliseFactor([1, NaN, 3], "median", null)).toEqual({ factor: 2 });
    expect(normaliseFactor([-3, -2, NaN, -1], "median", null)).toEqual({ reason: "median is not positive" });
    expect(normaliseFactor([0, 0, 0], "median", null)).toEqual({ reason: "median is not positive" });
  });

  it("normaliseFactor window uses the median over the union of both windows and refuses invalid windows", () => {
    const y = [0, 1, 2, 3, 4];
    expect(normaliseFactor(y, "window", [[0, 1], [4, 4]])).toEqual({ factor: 1 });
    expect(normaliseFactor(y, "window", null)).toEqual({ reason: "continuum windows are not set" });
    expect(normaliseFactor(y, "window", [[0, 1], [4, 5]])).toEqual({ reason: "continuum windows are not set" });
    expect(normaliseFactor(y, "none", null)).toEqual({ factor: 1 });
    expect(normaliseFactor(y, "offset", null)).toEqual({ factor: 1 });
  });

  it("normaliseFactor refuses an all-NaN series in peak, median and window with a reason", () => {
    const y = [NaN, NaN, NaN];
    expect(normaliseFactor(y, "peak", null)).toEqual({ reason: "peak is not positive" });
    expect(normaliseFactor(y, "median", null)).toEqual({ reason: "median is not positive" });
    expect(normaliseFactor(y, "window", [[0, 0], [2, 2]])).toEqual({ reason: "continuum median is not positive" });
  });

  it("normaliseFactor window refuses a zero continuum median", () => {
    expect(normaliseFactor([0, 0, 5, 5], "window", [[0, 1], [0, 1]])).toEqual({ reason: "continuum median is not positive" });
  });

  it("finiteMedian averages the two middle values of an even count and is NaN without finite values", () => {
    expect(finiteMedian([4, 1, NaN, 3, 2])).toBe(2.5);
    expect(finiteMedian([3, 1, 2])).toBe(2);
    expect(finiteMedian([NaN])).toBeNaN();
    expect(finiteMedian([])).toBeNaN();
  });

  it("offsetStepAuto returns 1 when every span is 0 or nothing is finite", () => {
    expect(offsetStepAuto([[2, 2], [5, 5, 5]])).toBe(1);
    expect(offsetStepAuto([[NaN, NaN]])).toBe(1);
    expect(offsetStepAuto([])).toBe(1);
    expect(offsetStepAuto([[1, 3], [10, 10.5]])).toBe(2);
    expect(offsetStepAuto([[1, NaN, 4]])).toBe(3);
  });
});

describe("comparisonSeries", () => {
  it("comparisonSeries shares one x reference across series and lifts the pen on NaN", () => {
    const entries = [regionEntry("r1", [1, NaN, 3]), regionEntry("r2", [2, 4, 6])];
    const result = comparisonSeries(entries, new Set(), axis3, "sum", "none", null, null, "Jy/beam");
    expect(result.series).toHaveLength(2);
    expect(result.series[0].x).toBe(axis3);
    expect(result.series[1].x).toBe(axis3);
    expect(result.series[0].y).toEqual([1, null, 3]);
    expect(result.series[1].y).toEqual([2, 4, 6]);
    expect(result.yLabel).toBe("Jy/beam x pix");
    expect(result.step).toBeNull();
    expect(result.plotted.map((p) => p.unit)).toEqual(["Jy/beam x pix", "Jy/beam x pix"]);
  });

  it("comparisonSeries lifts the pen on a null channel as sent by the backend", () => {
    const backendSum = [1, null, 3] as unknown as number[];
    const entries = [regionEntry("r1", backendSum)];
    for (const mode of ["none", "peak", "offset"] as const) {
      const result = comparisonSeries(entries, new Set(), axis3, "sum", mode, null, null, "Jy/beam");
      expect(result.series).toHaveLength(1);
      expect(result.series[0].y[1]).toBeNull();
      expect(Number.isFinite(result.series[0].y[0])).toBe(true);
    }
  });

  it("comparisonSeries offsets the k-th plotted series by k steps and takes the auto step from the largest span", () => {
    const entries = [regionEntry("r1", [1, 3]), regionEntry("r2", [10, 10.5])];
    const result = comparisonSeries(entries, new Set(), [0, 1], "sum", "offset", null, null, "Jy/beam");
    expect(result.step).toBe(2);
    expect(result.series[0].y).toEqual([1, 3]);
    expect(result.series[1].y).toEqual([12, 12.5]);
    expect(result.plotted.map((p) => p.offset)).toEqual([0, 2]);
    expect(result.yLabel).toBe("Jy/beam x pix + k × 2");
    const typed = comparisonSeries(entries, new Set(), [0, 1], "sum", "offset", null, 5, "Jy/beam");
    expect(typed.step).toBe(5);
    expect(typed.series[1].y).toEqual([15, 15.5]);
    expect(typed.plotted[1].unit).toBe("Jy/beam x pix + offset");
  });

  it("comparisonSeries omits a region without Jy in the Jy view with a reason and keeps the others", () => {
    const entries = [regionEntry("r1", [1, 2, 3], {}, { flux_jy: [0.1, 0.2, 0.3] }), regionEntry("r2", [4, 5, 6])];
    const result = comparisonSeries(entries, new Set(), axis3, "jy", "none", null, null, "MJy/sr");
    expect(result.series.map((s) => s.label)).toEqual(["r1"]);
    expect(result.series[0].y).toEqual([0.1, 0.2, 0.3]);
    expect(result.omitted).toEqual([{ id: "r2", label: "r2", reason: NO_JY }]);
    expect(result.yLabel).toBe("Jy");
  });

  it("comparisonSeries omits an errored entry with its error text and a hidden entry silently", () => {
    const entries = [
      regionEntry("r1", [1, 2, 3]),
      regionEntry("r2", [], { region: null, error: "circle region covers no image pixels" }),
      regionEntry("r3", [7, 8, 9]),
    ];
    const result = comparisonSeries(entries, new Set(["r3"]), axis3, "sum", "peak", null, null, null);
    expect(result.series.map((s) => s.label)).toEqual(["r1"]);
    expect(result.series[0].y).toEqual([1 / 3, 2 / 3, 1]);
    expect(result.omitted).toEqual([{ id: "r2", label: "r2", reason: "circle region covers no image pixels" }]);
    expect(result.plotted).toEqual([{ id: "r1", label: "r1", unit: "norm", factor: 3, offset: 0 }]);
    expect(result.yLabel).toBe("flux / peak");
    expect(NORMALISE_MODES).toEqual(["none", "peak", "median", "window", "offset"]);
  });
});

describe("entries", () => {
  it("entriesFrom assigns region colours, the palette by candidate position, labels from text or ordinal and the pixel colour", () => {
    const regions = [
      regionOf("bg1", annulus),
      regionOf("r1", circle, { text: "core", color: "#ff0000" }, "bg1"),
      regionOf("r2", box),
      regionOf("r3", { shape: "ellipse", x: 8, y: 8, rx: 2, ry: 1, angle: 0 }, { text: "  " }),
    ];
    const candidates = regions.slice(1);
    const settled = [
      fulfilled(regionSpectrum([1, 2, 3])),
      rejected<RegionSpectrum>(new Error("box region covers no image pixels")),
      rejected<RegionSpectrum>("plain string"),
    ];
    const entries = entriesFrom(candidates, regions, settled, { coord: { x: 3, y: 4 }, result: fulfilled(pixelSpectrum([4, 5, 6])) });
    expect(entries.map((e) => e.id)).toEqual(["r1", "r2", "r3", "pixel"]);
    expect(entries.map((e) => e.label)).toEqual(["core", "R3", "R4", "pixel (3, 4)"]);
    expect(entries.map((e) => e.color)).toEqual(["#ff0000", COMPARISON_PALETTE[1], COMPARISON_PALETTE[2], PIXEL_SERIES_COLOR]);
    expect(entries[0].backgroundId).toBe("bg1");
    expect(entries[0].source).toEqual({ kind: "region", shape: circle, background: annulus });
    expect(entries[0].region?.sum).toEqual([1, 2, 3]);
    expect(entries[0].error).toBeNull();
    expect(entries[1].error).toBe("box region covers no image pixels");
    expect(entries[1].region).toBeNull();
    expect(entries[2].error).toBe("plain string");
    expect(entries[3].kind).toBe("pixel");
    expect(entries[3].pixel?.values).toEqual([4, 5, 6]);
    expect(entries[3].source).toEqual({ kind: "pixel", x: 3, y: 4 });
    expect(COMPARISON_PALETTE).toHaveLength(8);
    expect(COMPARISON_PALETTE[0]).toBe("#7dd3fc");
    const failedPixel = entriesFrom([], regions, [], { coord: { x: 1, y: 1 }, result: rejected<CubeSpectrum>(new Error("cube closed")) });
    expect(failedPixel).toHaveLength(1);
    expect(failedPixel[0].error).toBe("cube closed");
    expect(entriesFrom(candidates, regions, settled, null)).toHaveLength(3);
  });

  it("pruneEntries drops entries of removed regions and of another channel count", () => {
    const regions = [regionOf("r1", circle), regionOf("r2", box)];
    const entries = [
      regionEntry("r1", [1, 2, 3]),
      regionEntry("r2", [1, 2]),
      regionEntry("gone", [1, 2, 3]),
      pixelEntry([1, 2, 3]),
      pixelEntry([1, 2, 3, 4], null, { id: "pixel-long" }),
      regionEntry("r2", [], { id: "r2-error", region: null, error: "failed" }),
    ];
    const kept = pruneEntries(entries, regions, 3);
    expect(kept.map((e) => e.id)).toEqual(["r1", "pixel"]);
  });

  it("limitCandidates compares only the first MAX_COMPARISON_REGIONS candidates and lists the rest as omitted with the cap reason", () => {
    expect(MAX_COMPARISON_REGIONS).toBe(16);
    const many = Array.from({ length: MAX_COMPARISON_REGIONS + 2 }, (_, i) =>
      regionOf(`r${i + 1}`, { shape: "circle", x: i, y: i, r: 1 }, i === MAX_COMPARISON_REGIONS + 1 ? { text: "last" } : {}),
    );
    const regions = [regionOf("bg1", annulus), ...many];
    const candidates = comparisonCandidates(regions);
    expect(candidates).toHaveLength(MAX_COMPARISON_REGIONS + 2);
    const limited = limitCandidates(candidates, regions);
    expect(limited.compared).toHaveLength(MAX_COMPARISON_REGIONS);
    expect(limited.compared.map((r) => r.id)).toEqual(candidates.slice(0, MAX_COMPARISON_REGIONS).map((r) => r.id));
    expect(limited.omitted).toEqual([
      { id: "r17", label: "R18", reason: "more than 16 regions: not compared" },
      { id: "r18", label: "last", reason: "more than 16 regions: not compared" },
    ]);
    const few = limitCandidates(candidates.slice(0, 3), regions);
    expect(few.compared.map((r) => r.id)).toEqual(["r1", "r2", "r3"]);
    expect(few.omitted).toEqual([]);
  });

  it("comparisonSeries appends the skipped candidates to omitted after the entry omissions and comparisonCsv names them", () => {
    const skipped = [{ id: "r17", label: "R18", reason: "more than 16 regions: not compared" }];
    const entries = [regionEntry("r1", [1, 2, 3]), regionEntry("r2", [], { region: null, error: "failed" })];
    const result = comparisonSeries(entries, new Set(), axis3, "sum", "none", null, null, "Jy/beam", skipped);
    expect(result.series.map((s) => s.label)).toEqual(["r1"]);
    expect(result.omitted).toEqual([{ id: "r2", label: "r2", reason: "failed" }, ...skipped]);
    expect(comparisonSeries(entries, new Set(), axis3, "sum", "none", null, null, "Jy/beam").omitted).toEqual([
      { id: "r2", label: "r2", reason: "failed" },
    ]);
    const csv = comparisonCsv({
      fileName: "cube.fits",
      view: "sum",
      mode: "none",
      windows: null,
      axis: { values: null, label: "Channel", unit: "ch", header: "channel" },
      axisKnown: false,
      vacuum: vacuumUmAxis(null, null, "optical", 3),
      channelCount: 3,
      entries,
      result,
      specsys: null,
      axisMode: "wavelength_vac",
      correction: "none",
      correctionResult: null,
      exportedAtUtc: "2026-09-25T12:00:00.000Z",
    });
    expect(csv.split("\r\n")).toContain("# omitted: R18; more than 16 regions: not compared");
    expect(csv.split("\r\n")).toContain("# omitted: r2; failed");
  });
});

describe("comparisonCsv", () => {
  function csvInput(extra: Partial<ComparisonCsvInput> = {}): ComparisonCsvInput {
    const axis = axisOf("wave", [1.0, 1.001, 1.002], { specsys: "BARYCENT" });
    const entries = [
      regionEntry("r1", [1, 2, 3], { label: "R1", regionText: null }, { flux_jy: [0.1, 0.2, 0.3] }),
      regionEntry("r2", [4, 5, 6], { label: "R2", backgroundId: "bg1", source: { kind: "region", shape: box, background: annulus } }, { bg_subtracted: true, n_bg: 12 }),
      pixelEntry([7, 8, 9]),
    ];
    const axisColumn = comparisonAxis(axis, "wavelength_vac", null, "optical", "none", null, 3);
    const mode = extra.mode ?? "none";
    const result = comparisonSeries(entries, new Set(), axisColumn.values ?? axis3, extra.view ?? "sum", mode, null, null, "Jy/beam");
    return {
      fileName: "cube.fits",
      view: "sum",
      mode,
      windows: null,
      axis: axisColumn,
      axisKnown: true,
      vacuum: vacuumUmAxis(axis, null, "optical", 3),
      channelCount: 3,
      entries,
      result,
      specsys: "BARYCENT",
      axisMode: "wavelength_vac",
      correction: "none",
      correctionResult: null,
      exportedAtUtc: "2026-09-25T12:00:00.000Z",
      ...extra,
    };
  }

  it("comparisonCsv writes one column per plotted series on the common axis and names omitted entries in the header", () => {
    const csv = comparisonCsv(csvInput());
    const lines = csv.split("\r\n");
    const header = lines.find((line) => !line.startsWith("#")) ?? "";
    expect(header).toBe('channel,wavelength_vacuum_um,vacuum_wavelength_um,R1 [Jy/beam x pix],R2 [Jy/beam x pix],"pixel (3, 4) [Jy/beam]"');
    expect(lines[0]).toBe("# file: cube.fits");
    expect(lines).toContain("# view: sum");
    expect(lines).toContain("# normalise: none");
    expect(lines).toContain("# stored_frame: BARYCENT");
    expect(lines.some((line) => line.startsWith("# velocity_frame: as stored; shift_kms: 0; method: -"))).toBe(true);
    expect(lines).toContain("# exported_utc: 2026-09-25T12:00:00.000Z");
    expect(lines.some((line) => line.startsWith("# series: R2; region; box (5.0, 5.0) 4.0×4.0 θ=0.0°; background: annulus bg1; npix: 10; n_bg: 12; bg_subtracted: true; unit: Jy/beam x pix; factor: 1; offset: 0"))).toBe(true);
    expect(lines.some((line) => line.startsWith("# series: pixel (3, 4); pixel; pixel (3, 4); background: none;"))).toBe(true);
    const rows = lines.filter((line) => line !== "" && !line.startsWith("#")).slice(1);
    expect(rows).toEqual(["0,1,1,1,4,7", "1,1.001,1.001,2,5,8", "2,1.002,1.002,3,6,9"]);
    expect(rows[0].split(",")).toHaveLength(3 + 3);

    const jy = comparisonCsv(csvInput({ view: "jy", mode: "peak" }));
    const jyLines = jy.split("\r\n");
    const jyHeader = jyLines.find((line) => !line.startsWith("#")) ?? "";
    expect(jyHeader).toBe("channel,wavelength_vacuum_um,vacuum_wavelength_um,R1 [norm]");
    expect(jyLines).toContain("# omitted: R2; no Jy calibration (BUNIT is not MJy/sr)");
    expect(jyLines).toContain("# omitted: pixel (3, 4); no Jy calibration (BUNIT is not MJy/sr)");
    expect(jyLines).toContain("# normalise: peak");
    const jyRows = jyLines.filter((line) => line !== "" && !line.startsWith("#")).slice(1);
    expect(jyRows[2]).toBe("2,1.002,1.002,1");

    const unknownAxis = comparisonCsv(csvInput({ axisKnown: false, axis: { values: null, label: "Channel", unit: "ch", header: "channel" }, vacuum: vacuumUmAxis(null, null, "optical", 3) }));
    const unknownHeader = unknownAxis.split("\r\n").find((line) => !line.startsWith("#")) ?? "";
    expect(unknownHeader).toBe('channel,R1 [Jy/beam x pix],R2 [Jy/beam x pix],"pixel (3, 4) [Jy/beam]"');
    expect(unknownAxis.split("\r\n")).toContain("# wavelength_note: no spectral axis");
  });

  it("comparisonCsvFileName strips the plane fragment", () => {
    expect(comparisonCsvFileName("C:/data/cube_s3d.fits#hdu=1")).toBe("cube_s3d_spectra.csv");
    expect(comparisonCsvFileName("C:\\data\\cube.fits")).toBe("cube_spectra.csv");
    expect(comparisonCsvFileName(null)).toBe("spectra.csv");
  });
});

describe("table entries", () => {
  const X1D_PATH = "C:/d/jw01_nrs1_x1d.fits";
  const TABLE_LABEL = "x1d (nrs1, G235H/F170LP)";

  function tableData(flux: number[], surfBright: number[] | null): TableEntryData {
    return {
      path: X1D_PATH,
      hdu: 1,
      nRows: 5,
      fluxUnit: "Jy",
      sbUnit: "MJy/sr",
      resampled: {
        flux,
        fluxErr: flux.map((v) => (Number.isFinite(v) ? v / 10 : null)),
        surfBright,
        rowsUsed: 4,
        droppedDq: 1,
        overlap: [1, 1.002],
      },
    };
  }

  function tableEntryOf(flux: number[], surfBright: number[] | null = null, extra: Partial<ComparisonEntry> = {}): ComparisonEntry {
    return {
      id: "x1d",
      kind: "table",
      label: TABLE_LABEL,
      color: "#f0abfc",
      source: { kind: "table", path: X1D_PATH, hdu: 1 },
      regionText: null,
      backgroundId: null,
      region: null,
      pixel: null,
      table: tableData(flux, surfBright),
      error: null,
      ...extra,
    };
  }

  it("entryValues gives the resampled flux in the sum and Jy views and the surface brightness in the mean view", () => {
    const entry = tableEntryOf([10, NaN, 30], [1, 2, 3]);
    expect(entryValues(entry, "sum")).toEqual([10, NaN, 30]);
    expect(entryValues(entry, "jy")).toEqual([10, NaN, 30]);
    expect(entryValues(entry, "mean")).toEqual([1, 2, 3]);
    expect(entryValues(tableEntryOf([10, 20, 30]), "mean")).toBeNull();
    expect(entryValues(tableEntryOf([10, 20, 30], null, { table: null, error: "no vacuum axis" }), "jy")).toBeNull();
  });

  it("comparisonSeries labels a region sum next to an x1d as mixed units and a Jy pair as Jy", () => {
    const region = regionEntry("r1", [1, 2, 3], {}, { flux_jy: [0.1, 0.2, 0.3] });
    const table = tableEntryOf([10, NaN, 30], [1, 2, 3]);
    const sum = comparisonSeries([region, table], new Set(), axis3, "sum", "none", null, null, "MJy/sr");
    expect(sum.yLabel).toBe("flux (mixed units)");
    expect(sum.plotted.map((p) => p.unit)).toEqual(["MJy/sr x pix", "Jy"]);
    expect(sum.series[1].y).toEqual([10, null, 30]);
    const jy = comparisonSeries([region, table], new Set(), axis3, "jy", "none", null, null, "MJy/sr");
    expect(jy.yLabel).toBe("Jy");
    expect(jy.series.map((s) => s.label)).toEqual(["r1", TABLE_LABEL]);
    const mean = comparisonSeries([region, table], new Set(), axis3, "mean", "none", null, null, "MJy/sr");
    expect(mean.yLabel).toBe("MJy/sr");
  });

  it("comparisonSeries omits an x1d without SURF_BRIGHT in the mean view with its own reason", () => {
    const result = comparisonSeries([tableEntryOf([10, 20, 30])], new Set(), axis3, "mean", "none", null, null, "MJy/sr");
    expect(result.series).toEqual([]);
    expect(result.omitted).toEqual([{ id: "x1d", label: TABLE_LABEL, reason: NO_SURF_BRIGHT_REASON }]);
    expect(NO_SURF_BRIGHT_REASON).toBe("x1d has no SURF_BRIGHT column");
    const regionless = comparisonSeries([regionEntry("r1", [], { region: null })], new Set(), axis3, "mean", "none", null, null, "MJy/sr");
    expect(regionless.omitted[0].reason).toBe("no result for this entry");
  });

  it("comparisonSeries says the x1d has no data in the continuum windows instead of a non-positive median", () => {
    const outside = comparisonSeries([tableEntryOf([NaN, NaN, 30])], new Set(), axis3, "sum", "window", [[0, 0], [1, 1]], null, "MJy/sr");
    expect(outside.series).toEqual([]);
    expect(outside.omitted).toEqual([{ id: "x1d", label: TABLE_LABEL, reason: X1D_NO_WINDOW_DATA_REASON }]);
    expect(X1D_NO_WINDOW_DATA_REASON).toBe("x1d has no data in the continuum windows");
    const negative = comparisonSeries([tableEntryOf([-1, -2, 30])], new Set(), axis3, "sum", "window", [[0, 0], [1, 1]], null, "MJy/sr");
    expect(negative.omitted[0].reason).toBe("continuum median is not positive");
    const inside = comparisonSeries([tableEntryOf([NaN, NaN, 30])], new Set(), axis3, "sum", "window", [[0, 0], [2, 2]], null, "MJy/sr");
    expect(inside.plotted.map((p) => p.factor)).toEqual([30]);
    const region = comparisonSeries([regionEntry("r1", [NaN, NaN, 3])], new Set(), axis3, "sum", "window", [[0, 0], [1, 1]], null, "MJy/sr");
    expect(region.omitted[0].reason).toBe("continuum median is not positive");
  });

  it("pruneEntries keeps a table entry on the cube channel grid and one still waiting for an axis", () => {
    const regions = [regionOf("r1", circle)];
    const kept = pruneEntries([regionEntry("r1", [1, 2, 3]), tableEntryOf([1, 2, 3])], regions, 3);
    expect(kept.map((e) => e.kind)).toEqual(["region", "table"]);
    expect(pruneEntries([tableEntryOf([1, 2])], regions, 3)).toEqual([]);
    const waiting = tableEntryOf([], null, { table: null, error: "no spectral axis" });
    expect(pruneEntries([waiting], regions, 3)).toEqual([waiting]);
  });

  function tableCsvInput(hidden: ReadonlySet<string>, mode: ComparisonCsvInput["mode"] = "none"): ComparisonCsvInput {
    const axis = axisOf("wave", [1.0, 1.001, 1.002]);
    const entries = [regionEntry("r1", [1, 2, 3], { label: "R1" }), tableEntryOf([10, NaN, 30], [1, 2, 3])];
    const axisColumn = comparisonAxis(axis, "wavelength_vac", null, "optical", "none", null, 3);
    const result = comparisonSeries(entries, hidden, axisColumn.values ?? axis3, "sum", mode, null, null, "MJy/sr");
    return {
      fileName: "cube_s3d.fits",
      view: "sum",
      mode,
      windows: null,
      axis: axisColumn,
      axisKnown: true,
      vacuum: vacuumUmAxis(axis, null, "optical", 3),
      channelCount: 3,
      entries,
      result,
      specsys: null,
      axisMode: "wavelength_vac",
      correction: "none",
      correctionResult: null,
      exportedAtUtc: "2026-10-02T12:00:00.000Z",
    };
  }

  it("plottedTable returns the table data only while the table entry is plotted", () => {
    const shown = tableCsvInput(new Set());
    expect(plottedTable(shown.entries, shown.result)?.path).toBe(X1D_PATH);
    const hidden = tableCsvInput(new Set(["x1d"]));
    expect(plottedTable(hidden.entries, hidden.result)).toBeNull();
  });

  it("comparisonCsv adds the x1d provenance line and unnormalised x1d flux and error columns", () => {
    const csv = comparisonCsv(tableCsvInput(new Set(), "peak"));
    const lines = csv.split("\r\n");
    expect(lines).toContain(
      `# x1d: ${X1D_PATH}; hdu: 1; rows: 5; dq_rows_dropped: 1; resampling: linear onto the cube vacuum axis, never across a dropped row; x1d_flux_err_jy: FLUX_ERROR combined in quadrature between the bracketing rows; ${APERTURE_NOTE}`,
    );
    expect(lines).toContain(
      `# series: ${TABLE_LABEL}; table; x1d jw01_nrs1_x1d.fits HDU 1; background: pipeline; npix: -; n_bg: -; bg_subtracted: -; unit: norm; factor: 30; offset: 0`,
    );
    const header = lines.find((line) => !line.startsWith("#")) ?? "";
    expect(header).toBe(`channel,wavelength_vacuum_um,vacuum_wavelength_um,R1 [norm],"${TABLE_LABEL} [norm]",x1d_flux_jy,x1d_flux_err_jy`);
    const rows = lines.filter((line) => line !== "" && !line.startsWith("#")).slice(1);
    expect(rows[0]).toBe(`0,1,1,${1 / 3},${1 / 3},10,1`);
    expect(rows[1]).toBe(`1,1.001,1.001,${2 / 3},,,`);
    expect(rows[2]).toBe("2,1.002,1.002,1,1,30,3");
  });

  it("comparisonCsv leaves the x1d columns out while the table entry is hidden", () => {
    const csv = comparisonCsv(tableCsvInput(new Set(["x1d"])));
    expect(csv).not.toContain("x1d_flux_jy");
    expect(csv.split("\r\n").some((line) => line.startsWith("# x1d:"))).toBe(false);
  });

  it("comparisonCsv writes the table it is given, the same one the measurement log reads", () => {
    const given = { ...tableData([7, 8, 9], null), path: "C:/d/other_x1d.fits", hdu: 4 };
    const lines = comparisonCsv({ ...tableCsvInput(new Set()), table: given }).split("\r\n");
    expect(lines.some((line) => line.startsWith("# x1d: C:/d/other_x1d.fits; hdu: 4;"))).toBe(true);
    const rows = lines.filter((line) => line !== "" && !line.startsWith("#")).slice(1);
    expect(rows.map((row) => row.split(",").slice(-2).join(","))).toEqual(["7,0.7", "8,0.8", "9,0.9"]);
    const none = comparisonCsv({ ...tableCsvInput(new Set()), table: null });
    expect(none).not.toContain("x1d_flux_jy");
    expect(none.split("\r\n").some((line) => line.startsWith("# x1d:"))).toBe(false);
  });
});
