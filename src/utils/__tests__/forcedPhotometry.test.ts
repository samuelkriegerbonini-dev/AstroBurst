import { describe, it, expect } from "vitest";
import {
  FORCED_ERROR_NOTE,
  FORCED_WEIGHTS_NOTE,
  adjacentPairs,
  bandOrder,
  bandSlugs,
  cellFlags,
  cellMagnitude,
  colourCellText,
  fluxCellText,
  forcedColour,
  forcedCsvFileName,
  forcedLongCsv,
  forcedSedPoints,
  forcedWideCsv,
  magCellText,
  toUjy,
} from "../forcedPhotometry";
import { CSV_LINE_END } from "../catalogCsv";
import { provenanceLines, unitLines } from "../csvProvenance";
import { formatWithUncertainty } from "../formatUncertainty";
import type {
  ForcedApertureCorrection,
  ForcedBand,
  ForcedCell,
  ForcedPhotometryResult,
} from "../../shared/types/forcedPhotometry";

const EXPORTED_UTC = "2026-11-13T10:00:00.000Z";
const APP_VERSION_UNDER_TEST = "v0.8.1";
const NO_WCS_ERROR = "x.fits has no celestial WCS: sky positions cannot be placed on it";

function cell(overrides: Partial<ForcedCell> = {}): ForcedCell {
  return {
    x: 10.5,
    y: 20.25,
    on_image: true,
    error: null,
    flux_native: 0.550813,
    flux_err_native: 0.0275406,
    flux_jy: 2e-6,
    flux_err_jy: 1e-7,
    flux_err_corr_jy: null,
    flux_total_jy: null,
    mag_ab: 23.147425,
    mag_ab_err: 0.054287,
    upper_limit_jy: null,
    upper_limit_ab: null,
    snr: 20,
    masked_fraction: 0,
    n_nonfinite: 0,
    n_masked: 0,
    flags_or: 0,
    saturated: false,
    fwhm_px: null,
    ...overrides,
  };
}

function band(index: number, label: string, pivot: number | null, overrides: Partial<ForcedBand> = {}): ForcedBand {
  return {
    index,
    path: `C:/data/${label}.fits`,
    file_name: `${label}.fits`,
    label,
    error: null,
    filter: pivot === null ? null : { dataset: "DECAM", filter: label, pivot_um: pivot, source: "header" },
    photcal: null,
    calibration_origin: "BUNIT nanomaggy, zero point 22.5",
    jy_per_native_unit: 3.631e-6,
    bunit: "nanomaggy",
    pixel_scale_arcsec: 0.262,
    aperture_radius_px: 7.6,
    annulus_px: null,
    aperture_correction: null,
    err_origin: null,
    dq_source: null,
    f_corr: null,
    n_measured: 1,
    n_failed: 0,
    warnings: [],
    ...overrides,
  };
}

function correction(factor: number): ForcedApertureCorrection {
  return { radius_px: 7.6, reference_radius_px: 13.4, factor, scatter: 0.01, n_stars: 12, n_candidates: 20 };
}

function failedBand(index: number, label: string, error: string): ForcedBand {
  return band(index, label, null, {
    error,
    calibration_origin: null,
    jy_per_native_unit: null,
    bunit: null,
    pixel_scale_arcsec: null,
    aperture_radius_px: null,
    n_measured: 0,
  });
}

function run(bands: ForcedBand[], cellsBySource: (ForcedCell | null)[][]): ForcedPhotometryResult {
  return {
    bands,
    sources: cellsBySource.map((cells, i) => ({ id: `s${i}`, ra_deg: 150.1 + i * 0.001, dec_deg: 2.2, cells })),
    aperture: {
      radius_arcsec: 2,
      radius_px: null,
      annulus_arcsec: null,
      annulus_px: null,
      correction: "none",
      reference_radius_arcsec: null,
    },
    limit_sigma: 3,
    elapsed_ms: 12,
    warnings: [],
  };
}

function csvInput(result: ForcedPhotometryResult) {
  return { result, exportedUtc: EXPORTED_UTC, appVersion: APP_VERSION_UNDER_TEST };
}

function splitCsv(text: string): { comments: string[]; header: string; rows: string[]; tail: string } {
  const lines = text.split(CSV_LINE_END);
  const first = lines.findIndex((line) => !line.startsWith("#"));
  return {
    comments: lines.slice(0, first),
    header: lines[first],
    rows: lines.slice(first + 1, lines.length - 1),
    tail: lines[lines.length - 1],
  };
}

function bandLine(b: ForcedBand): string {
  const prefix = "# band_1: ";
  const line = splitCsv(forcedLongCsv(csvInput(run([b], [])))).comments.find((l) => l.startsWith(prefix));
  return (line ?? "").slice(prefix.length);
}

const LONG_HEADER = [
  "id",
  "ra_deg",
  "dec_deg",
  "band",
  "file",
  "filter",
  "pivot_um",
  "x",
  "y",
  "on_image",
  "flux_native",
  "flux_err_native",
  "flux_ujy",
  "flux_err_ujy",
  "flux_err_corr_ujy",
  "flux_total_ujy",
  "mag_ab",
  "mag_ab_err",
  "upper_limit_ujy",
  "upper_limit_ab",
  "snr",
  "masked_fraction",
  "n_masked",
  "n_nonfinite",
  "flags_or",
  "saturated",
  "fwhm_px",
  "error",
];

const LONG_UNITS = {
  ra_deg: "deg",
  dec_deg: "deg",
  pivot_um: "um",
  x: "px (0-based)",
  y: "px (0-based)",
  flux_native: "image unit (see the band lines)",
  flux_err_native: "image unit (see the band lines)",
  flux_ujy: "uJy",
  flux_err_ujy: "uJy",
  flux_err_corr_ujy: "uJy",
  flux_total_ujy: "uJy",
  mag_ab: "mag (AB)",
  mag_ab_err: "mag (AB)",
  upper_limit_ujy: "uJy",
  upper_limit_ab: "mag (AB)",
  fwhm_px: "px",
};

describe("forcedPhotometry", () => {
  it("converts Jy to uJy without float dust and keeps missing values missing", () => {
    expect(toUjy(2.871799e-4)).toBe(287.1799);
    expect(toUjy(3.630780547701009e-6)).toBe(3.6307805477);
    expect(toUjy(null)).toBeNull();
    expect(toUjy(undefined)).toBeNull();
    expect(toUjy(Number.NaN)).toBeNull();
    expect(toUjy(Number.POSITIVE_INFINITY)).toBeNull();
  });

  it("orders bands by pivot wavelength with unidentified bands last", () => {
    const bands = [band(0, "z", 0.9168), band(1, "x", null), band(2, "g", 0.4808), band(3, "r", 0.6418)];
    expect(bandOrder(bands)).toEqual([2, 3, 0, 1]);
    const ties = [band(0, "a", null), band(1, "b", 0.5), band(2, "c", null), band(3, "d", 0.5)];
    expect(bandOrder(ties)).toEqual([1, 3, 0, 2]);
  });

  it("a colour is the difference of AB magnitudes with errors in quadrature", () => {
    const result = run(
      [band(0, "g", 0.4808), band(1, "r", 0.6418)],
      [[cell({ mag_ab: 20.0, mag_ab_err: 0.03 }), cell({ mag_ab: 19.5, mag_ab_err: 0.04 })]],
    );
    const colour = forcedColour(result, 0, 0, 1);
    expect(Math.abs((colour.value ?? Number.NaN) - 0.5)).toBeLessThan(1e-12);
    expect(Math.abs((colour.err ?? Number.NaN) - 0.05)).toBeLessThan(1e-12);
    expect(colour.limit).toBe("none");
    expect(colour.basis).toBe("aperture");
  });

  it("with both aperture corrections the colour is between total magnitudes", () => {
    const cells = [cell({ mag_ab: 20.0, mag_ab_err: 0.03 }), cell({ mag_ab: 19.5, mag_ab_err: 0.04 })];
    const both = run(
      [
        band(0, "g", 0.4808, { aperture_correction: correction(1.1) }),
        band(1, "r", 0.6418, { aperture_correction: correction(1.25) }),
      ],
      [cells],
    );
    const total = forcedColour(both, 0, 0, 1);
    expect(total.basis).toBe("total");
    expect(Math.abs((total.value ?? Number.NaN) - 0.6387933196245783)).toBeLessThan(1e-9);
    expect(Math.abs((total.err ?? Number.NaN) - 0.05)).toBeLessThan(1e-12);
    const g = cellMagnitude(cells[0], both.bands[0], "total");
    expect(Math.abs((g.mag ?? Number.NaN) - (20 - 2.5 * Math.log10(1.1)))).toBeLessThan(1e-12);
    const one = run([band(0, "g", 0.4808, { aperture_correction: correction(1.1) }), band(1, "r", 0.6418)], [cells]);
    const aperture = forcedColour(one, 0, 0, 1);
    expect(aperture.basis).toBe("aperture");
    expect(Math.abs((aperture.value ?? Number.NaN) - 0.5)).toBeLessThan(1e-12);
  });

  it("a limit on one side makes the colour a bound", () => {
    const limitCell = cell({ mag_ab: 24.8, mag_ab_err: 0.4, upper_limit_jy: 9.12e-7, upper_limit_ab: 24.0 });
    const detected = cell({ mag_ab: 22.0, mag_ab_err: 0.05 });
    const result = run([band(0, "g", 0.4808), band(1, "r", 0.6418)], [[limitCell, detected], [limitCell, limitCell]]);
    expect(cellMagnitude(limitCell, result.bands[0], "aperture")).toEqual({ mag: 24.0, err: null, limit: true });
    const lower = forcedColour(result, 0, 0, 1);
    expect(lower.value).toBe(2.0);
    expect(lower.err).toBeNull();
    expect(lower.limit).toBe("lower");
    const upper = forcedColour(result, 0, 1, 0);
    expect(upper.value).toBe(-2.0);
    expect(upper.err).toBeNull();
    expect(upper.limit).toBe("upper");
    const both = forcedColour(result, 1, 0, 1);
    expect(both.value).toBeNull();
    expect(both.err).toBeNull();
    expect(both.limit).toBe("both");
    const corrected = run(
      [band(0, "g", 0.4808, { aperture_correction: correction(1.25) }), band(1, "r", 0.6418, { aperture_correction: correction(1.25) })],
      [[limitCell, detected]],
    );
    expect(cellMagnitude(limitCell, corrected.bands[0], "total").mag).toBeCloseTo(24.0 - 2.5 * Math.log10(1.25), 12);
  });

  it("missing, failed and off-image cells give no magnitude and no colour", () => {
    const detected = cell({ mag_ab: 22.0, mag_ab_err: 0.05 });
    const bands = [band(0, "g", 0.4808), band(1, "r", 0.6418)];
    const cases: (ForcedCell | null)[] = [
      null,
      cell({ error: "the aperture has no finite pixel", mag_ab: 20.0 }),
      cell({ on_image: false, mag_ab: 20.0 }),
    ];
    for (const broken of cases) {
      const colour = forcedColour(run(bands, [[broken, detected]]), 0, 0, 1);
      expect({ value: colour.value, err: colour.err, limit: colour.limit }).toEqual({ value: null, err: null, limit: "none" });
      expect(cellMagnitude(broken, bands[0], "aperture")).toEqual({ mag: null, err: null, limit: false });
    }
    expect(cellMagnitude(cell({ mag_ab: null, mag_ab_err: null }), bands[0], "aperture")).toEqual({
      mag: null,
      err: null,
      limit: false,
    });
  });

  it("adjacent pairs skip bands without calibration, filter or data", () => {
    const bands = [
      band(0, "g", 0.4808),
      band(1, "r", 0.6418),
      band(2, "i", 0.7835, { jy_per_native_unit: null }),
      band(3, "z", 0.9168),
    ];
    expect(adjacentPairs(bands)).toEqual([
      [0, 1],
      [1, 3],
    ]);
    const shuffled = [band(0, "z", 0.9168), failedBand(1, "y", "broken"), band(2, "g", 0.4808), band(3, "w", null)];
    expect(adjacentPairs(shuffled)).toEqual([[2, 0]]);
  });

  it("flux and magnitude texts go through the shared formatter", () => {
    const g = band(0, "g", 0.4808);
    expect(fluxCellText(cell(), g)).toBe(formatWithUncertainty(2, 0.1, { unit: "uJy" }));
    expect(fluxCellText(cell({ flux_err_corr_jy: 1.6e-7 }), g)).toBe(formatWithUncertainty(2, 0.16, { unit: "uJy" }));
    const limitCell = cell({ upper_limit_jy: 3e-7, upper_limit_ab: 25.207 });
    expect(fluxCellText(limitCell, g)).toBe("< " + formatWithUncertainty(0.3, null, { unit: "uJy" }));
    expect(magCellText(limitCell)).toBe("> 25.21");
    expect(magCellText(cell())).toBe(formatWithUncertainty(23.147425, 0.054287));
    const raw = band(1, "raw", 0.6418, { jy_per_native_unit: null, bunit: "counts" });
    const uncalibrated = cell({ flux_jy: null, flux_err_jy: null, mag_ab: null, mag_ab_err: null, flux_native: 12.5, flux_err_native: 0.5 });
    expect(fluxCellText(uncalibrated, raw)).toBe(formatWithUncertainty(12.5, 0.5, { unit: "counts" }));
    expect(fluxCellText(uncalibrated, band(1, "raw", 0.6418, { jy_per_native_unit: null, bunit: null }))).toBe(
      formatWithUncertainty(12.5, 0.5),
    );
    expect(magCellText(uncalibrated)).toBe("--");
    expect(fluxCellText(cell({ on_image: false }), g)).toBe("off image");
    expect(fluxCellText(cell({ error: "the aperture has no finite pixel" }), g)).toBe("failed");
    expect(fluxCellText(null, g)).toBe("--");
    expect(fluxCellText(cell(), failedBand(0, "g", "broken"))).toBe("--");
    expect(magCellText(null)).toBe("--");
    expect(colourCellText({ value: null, err: null, limit: "both", basis: "aperture" })).toBe("--");
    expect(colourCellText({ value: 2, err: null, limit: "lower", basis: "aperture" })).toBe("> 2.00");
    expect(colourCellText({ value: -2, err: null, limit: "upper", basis: "aperture" })).toBe("< -2.00");
    expect(colourCellText({ value: 0.5, err: 0.05, limit: "none", basis: "total" })).toBe(formatWithUncertainty(0.5, 0.05));
  });

  it("flags name saturation, missing data and the bits that are set", () => {
    const flagged = cell({ saturated: true, masked_fraction: 0.034, flags_or: 5 });
    const named = band(0, "g", 0.4808, {
      flag_names: [
        { value: 1, name: "NPRIMARY" },
        { value: 4, name: "SATUR_G" },
        { value: 2, name: "BRIGHT" },
      ],
    });
    expect(cellFlags(flagged, named)).toEqual(["saturated", "3.4% no data", "NPRIMARY", "SATUR_G"]);
    expect(cellFlags(flagged, band(0, "g", 0.4808))).toEqual(["saturated", "3.4% no data", "flags 0x5"]);
    expect(cellFlags(flagged, band(0, "g", 0.4808, { flag_names: [] }))).toEqual(["saturated", "3.4% no data", "flags 0x5"]);
    expect(cellFlags(cell({ masked_fraction: 0.7 }), band(0, "g", 0.4808))).toEqual(["70% no data"]);
    expect(cellFlags(cell({ flags_or: 26 }), band(0, "g", 0.4808))).toEqual(["flags 0x1a"]);
    expect(cellFlags(cell(), named)).toEqual([]);
    expect(cellFlags(null, named)).toEqual([]);
  });

  it("sed points are in uJy at the pivot of each band, with limits marked", () => {
    const limitCell = cell({ upper_limit_jy: 3e-7, upper_limit_ab: 25.207 });
    const plain = run([band(0, "g", 0.4808), band(1, "r", 0.6418), band(2, "z", null)], [[cell(), limitCell, cell()]]);
    expect(forcedSedPoints(plain, 0)).toEqual({
      points: [
        { index: 0, filter: "g", pivotUm: 0.4808, value: 2, err: 0.1, limit: false },
        { index: 1, filter: "r", pivotUm: 0.6418, value: 0.3, err: null, limit: true },
      ],
      basis: "aperture",
    });
    const corrected = run(
      [
        band(0, "g", 0.4808, { aperture_correction: correction(1.25) }),
        band(1, "r", 0.6418, { aperture_correction: correction(1.25) }),
        band(2, "z", null),
      ],
      [[cell(), limitCell, cell()]],
    );
    expect(forcedSedPoints(corrected, 0)).toEqual({
      points: [
        { index: 0, filter: "g", pivotUm: 0.4808, value: 2.5, err: 0.125, limit: false },
        { index: 1, filter: "r", pivotUm: 0.6418, value: 0.375, err: null, limit: true },
      ],
      basis: "total",
    });
    const noisy = run([band(0, "g", 0.4808), band(1, "r", 0.6418)], [[cell({ flux_err_corr_jy: 1.6e-7 }), null]]);
    expect(forcedSedPoints(noisy, 0)).toEqual({
      points: [{ index: 0, filter: "g", pivotUm: 0.4808, value: 2, err: 0.16, limit: false }],
      basis: "aperture",
    });
    const reversed = run([band(0, "r", 0.6418), band(1, "g", 0.4808)], [[cell({ flux_jy: -1e-7 }), cell({ flux_err_jy: null })]]);
    expect(forcedSedPoints(reversed, 0).points).toEqual([
      { index: 1, filter: "g", pivotUm: 0.4808, value: 2, err: null, limit: false },
      { index: 0, filter: "r", pivotUm: 0.6418, value: -0.1, err: 0.1, limit: false },
    ]);
  });

  it("slugs are unique identifiers", () => {
    const labels = ["g", "r band", "g", ""];
    expect(bandSlugs(labels.map((label, i) => band(i, label, null)))).toEqual(["g", "r_band", "g_2", "band"]);
    const crowded = ["g", "g_2", "g", "g"];
    expect(bandSlugs(crowded.map((label, i) => band(i, label, null)))).toEqual(["g", "g_2", "g_3", "g_4"]);
  });

  it("the long csv has one row per source and band, the provenance first and the pinned columns", () => {
    const gError = "g.fits has no celestial WCS: sky positions cannot be placed on it";
    const result = run(
      [failedBand(0, "g", gError), band(1, "r", 0.6418)],
      [
        [null, cell()],
        [null, cell({ upper_limit_jy: 3e-7, upper_limit_ab: 25.207, flux_jy: -1e-7 })],
      ],
    );
    const csv = splitCsv(forcedLongCsv(csvInput(result)));
    const expectedEntries: [string, string | number][] = [
      ["software", "AstroBurst v0.8.1"],
      ["exported_utc", EXPORTED_UTC],
      ["file", "g.fits; r.fits"],
      ["positions", 2],
      ["bands", 2],
      ["aperture_radius_arcsec", 2],
      ["sky_annulus", "none"],
      ["aperture_correction", "none"],
      ["limit_sigma", 3],
      ["aperture_weights", FORCED_WEIGHTS_NOTE],
      ["flux_error", FORCED_ERROR_NOTE],
      ["band_1", `g; file g.fits; not measured: ${gError}`],
      [
        "band_2",
        "r; file r.fits; filter DECAM r, pivot 0.6418 um; pixel scale 0.262 arcsec/px; radius 7.6 px; calibration BUNIT nanomaggy, zero point 22.5; error layer none; f_corr not given; aperture correction none",
      ],
    ];
    expect(csv.comments).toEqual([...provenanceLines(expectedEntries), ...unitLines(LONG_UNITS)]);
    expect(csv.comments.slice(0, 3).map((line) => line.slice(2, line.indexOf(":")))).toEqual(["software", "exported_utc", "file"]);
    expect(csv.comments).toContain(`# aperture_weights: ${FORCED_WEIGHTS_NOTE}`);
    expect(csv.comments).toContain(`# flux_error: ${FORCED_ERROR_NOTE}`);
    expect(csv.header).toBe(LONG_HEADER.join(","));
    expect(LONG_HEADER).toHaveLength(28);
    expect(csv.rows).toHaveLength(4);
    expect(csv.tail).toBe("");
    const column = (name: string) => LONG_HEADER.indexOf(name);
    const cells = csv.rows.map((row) => row.split(","));
    expect(cells[1][column("id")]).toBe("s0");
    expect(cells[1][column("band")]).toBe("r");
    expect(cells[1][column("file")]).toBe("r.fits");
    expect(cells[1][column("filter")]).toBe("DECAM r");
    expect(cells[1][column("pivot_um")]).toBe("0.6418");
    expect(cells[1][column("flux_ujy")]).toBe("2");
    expect(cells[1][column("flux_err_ujy")]).toBe("0.1");
    expect(cells[1][column("flux_err_corr_ujy")]).toBe("");
    expect(cells[1][column("mag_ab")]).toBe("23.147425");
    expect(cells[1][column("on_image")]).toBe("true");
    expect(cells[1][column("error")]).toBe("");
    expect(cells[0][column("id")]).toBe("s0");
    expect(cells[0][column("band")]).toBe("g");
    expect(cells[0][column("file")]).toBe("g.fits");
    expect(cells[0][column("flux_ujy")]).toBe("");
    expect(cells[0][column("mag_ab")]).toBe("");
    expect(cells[0][column("error")]).toBe(gError);
    expect(cells[3][column("id")]).toBe("s1");
    expect(cells[3][column("flux_ujy")]).toBe("-0.1");
    expect(cells[3][column("upper_limit_ujy")]).toBe("0.3");
    expect(cells[3][column("upper_limit_ab")]).toBe("25.207");

    const empty = splitCsv(forcedLongCsv(csvInput(run([band(0, "r", 0.6418)], []))));
    expect(empty.header).toBe(LONG_HEADER.join(","));
    expect(empty.rows).toEqual([]);
    expect(empty.comments).toContain("# positions: 0");

    const allMissing = run([failedBand(0, "g", "first"), failedBand(1, "r", "second")], [[null, null]]);
    const missingRows = splitCsv(forcedLongCsv(csvInput(allMissing))).rows.map((row) => row.split(","));
    expect(missingRows.map((row) => [row[column("band")], row[column("file")], row[column("error")]])).toEqual([
      ["g", "g.fits", "first"],
      ["r", "r.fits", "second"],
    ]);

    const withSky = run([band(0, "r", 0.6418)], []);
    withSky.aperture = {
      radius_arcsec: null,
      radius_px: 5,
      annulus_arcsec: null,
      annulus_px: [8, 12.5],
      correction: "growth_curve",
      reference_radius_arcsec: 3.5,
    };
    const skyComments = splitCsv(forcedLongCsv(csvInput(withSky))).comments;
    expect(skyComments.slice(5, 9)).toEqual([
      "# aperture_radius_px: 5",
      "# sky_annulus: 8 to 12.5 px",
      "# aperture_correction: growth curve of the stars of each band",
      "# reference_radius_arcsec: 3.5",
    ]);
    const arcsecSky = run([band(0, "r", 0.6418)], []);
    arcsecSky.aperture = { ...arcsecSky.aperture, annulus_arcsec: [4, 6] };
    expect(splitCsv(forcedLongCsv(csvInput(arcsecSky))).comments).toContain("# sky_annulus: 4 to 6 arcsec");
  });

  it("the wide csv has one row per source with a block per band and the adjacent colours", () => {
    const result = run(
      [band(0, "r", 0.6418), band(1, "g", 0.4808), failedBand(2, "z", "broken")],
      [
        [cell({ mag_ab: 19.5, mag_ab_err: 0.04 }), cell({ mag_ab: 20.0, mag_ab_err: 0.03, masked_fraction: 0.25 }), null],
        [cell({ mag_ab: 22.0, mag_ab_err: 0.05 }), cell({ upper_limit_jy: 9.12e-7, upper_limit_ab: 24.0 }), null],
      ],
    );
    const csv = splitCsv(forcedWideCsv(csvInput(result)));
    expect(csv.header).toBe(
      "id,ra_deg,dec_deg,g_flux_ujy,g_flux_err_ujy,g_mag_ab,g_mag_ab_err,g_upper_limit_ab,g_masked_fraction,r_flux_ujy,r_flux_err_ujy,r_mag_ab,r_mag_ab_err,r_upper_limit_ab,r_masked_fraction,g-r,g-r_err,g-r_limit",
    );
    expect(csv.comments).toContain("# colours: g-r (aperture)");
    const colours = csv.comments.indexOf("# colours: g-r (aperture)");
    expect(csv.comments[colours - 1]).toBe(`# flux_error: ${FORCED_ERROR_NOTE}`);
    expect(csv.comments[colours + 1].startsWith("# band_1: r; file r.fits;")).toBe(true);
    expect(csv.comments[colours + 3]).toBe("# band_3: z; file z.fits; not measured: broken");
    expect(csv.comments).toContain("# unit g_flux_ujy: uJy");
    expect(csv.comments).toContain("# unit r_flux_err_ujy: uJy");
    expect(csv.comments).toContain("# unit g_mag_ab: mag (AB)");
    expect(csv.comments).toContain("# unit g_mag_ab_err: mag (AB)");
    expect(csv.comments).toContain("# unit r_upper_limit_ab: mag (AB)");
    expect(csv.comments).toContain("# unit g-r: mag (AB)");
    expect(csv.comments).toContain("# unit g-r_err: mag (AB)");
    expect(csv.rows).toHaveLength(2);
    const header = csv.header.split(",");
    const first = csv.rows[0].split(",");
    const second = csv.rows[1].split(",");
    expect(first[header.indexOf("id")]).toBe("s0");
    expect(first[header.indexOf("g_masked_fraction")]).toBe("0.25");
    expect(first[header.indexOf("g_flux_ujy")]).toBe("2");
    expect(Math.abs(Number(first[header.indexOf("g-r")]) - 0.5)).toBeLessThan(1e-12);
    expect(Math.abs(Number(first[header.indexOf("g-r_err")]) - 0.05)).toBeLessThan(1e-12);
    expect(first[header.indexOf("g-r_limit")]).toBe("none");
    expect(second[header.indexOf("g_upper_limit_ab")]).toBe("24");
    expect(second[header.indexOf("g-r")]).toBe("2");
    expect(second[header.indexOf("g-r_err")]).toBe("");
    expect(second[header.indexOf("g-r_limit")]).toBe("lower");

    const oneBand = splitCsv(forcedWideCsv(csvInput(run([band(0, "r", 0.6418)], []))));
    expect(oneBand.comments).toContain("# colours: none");
    expect(oneBand.header).toBe(
      "id,ra_deg,dec_deg,r_flux_ujy,r_flux_err_ujy,r_mag_ab,r_mag_ab_err,r_upper_limit_ab,r_masked_fraction",
    );
    expect(oneBand.rows).toEqual([]);
    expect(oneBand.tail).toBe("");

    const totals = run(
      [
        band(0, "g", 0.4808, { aperture_correction: correction(1.1) }),
        band(1, "r", 0.6418, { aperture_correction: correction(1.25) }),
      ],
      [[cell(), cell()]],
    );
    expect(splitCsv(forcedWideCsv(csvInput(totals))).comments).toContain("# colours: g-r (total)");
  });

  it("band lines state filter, scale, radius, calibration, error layer, noise factor and correction", () => {
    const full = band(0, "r", 0.64176, {
      file_name: "brick-r.fits",
      pixel_scale_arcsec: 0.262,
      aperture_radius_px: 2 / 0.262,
      calibration_origin: "BUNIT nanomaggy, zero point 22.5",
      err_origin: { role: "inverse_variance", source: "brick-invvar-r.fits", assumed: false, n_no_data: 0 },
      f_corr: 1.647,
      aperture_correction: {
        radius_px: 2 / 0.262,
        reference_radius_px: 3.5 / 0.262,
        factor: 1.0427,
        scatter: 0.0068,
        n_stars: 16,
        n_candidates: 23,
      },
    });
    expect(bandLine(full)).toBe(
      "r; file brick-r.fits; filter DECAM r, pivot 0.64176 um; pixel scale 0.262 arcsec/px; radius 7.63359 px; calibration BUNIT nanomaggy, zero point 22.5; error layer inverse_variance from brick-invvar-r.fits; f_corr 1.647; aperture correction 1.0427 (scatter 0.0068, 16 stars, reference 13.3588 px)",
    );
    const bare = band(0, "x", null, {
      pixel_scale_arcsec: 0.2,
      aperture_radius_px: 10,
      calibration_origin: null,
      jy_per_native_unit: null,
    });
    expect(bandLine(bare)).toBe(
      "x; file x.fits; filter not identified; pixel scale 0.2 arcsec/px; radius 10 px; calibration none; error layer none; f_corr not given; aperture correction none",
    );
    expect(bandLine(failedBand(0, "x", NO_WCS_ERROR))).toBe(`x; file x.fits; not measured: ${NO_WCS_ERROR}`);
    const assumed = band(0, "x", null, {
      pixel_scale_arcsec: 0.2,
      aperture_radius_px: 10,
      calibration_origin: null,
      err_origin: { role: "sigma", source: "sky noise of the image", assumed: true, n_no_data: 0 },
    });
    expect(bandLine(assumed)).toContain("; error layer sigma from sky noise of the image (assumed); ");
  });

  it("file names", () => {
    expect(forcedCsvFileName("long")).toBe("forced_photometry_long.csv");
    expect(forcedCsvFileName("wide")).toBe("forced_photometry_wide.csv");
  });
});
