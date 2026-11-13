import { describe, it, expect } from "vitest";
import {
  BATCH_SEMANTICS_TEXT,
  FALLBACK_APERTURE_RADIUS_PX,
  MAX_APERTURE_RADIUS_PX,
  MAX_SKY_OUTER_RADIUS_PX,
  MIN_APERTURE_RADIUS_PX,
  NO_DETECTED_STARS_HINT,
  PHOTOMETRY_COLUMN_KEYS,
  PHOTOMETRY_CORRECTION_NOTE,
  PHOTOMETRY_CSV_COLUMNS,
  PHOTOMETRY_ERROR_NOTE,
  PHOTOMETRY_LIMIT_NOTE,
  PHOTOMETRY_POSITIONS_NOTE,
  PHOTOMETRY_WEIGHTS_NOTE,
  abCellText,
  apertureRadiusInRange,
  fluxCellText,
  flagDuplicates,
  growthReferenceLines,
  medianSnr,
  parsePositions,
  photometryCsvFileName,
  photometryCsvProvenance,
  photometryCsvUnits,
  photometryTableCsv,
  plateauCaption,
  rowValue,
  sortRows,
  summedFluxUnit,
  tableUnitsCaption,
  type PhotometryCsvContext,
  type PhotometryTableRow,
} from "../photometryTable";
import { CSV_LINE_END } from "../catalogCsv";
import { exportEntries, provenanceLines, unitLines, withProvenance } from "../csvProvenance";
import { APP_VERSION } from "../constants";
import { FORCED_WEIGHTS_NOTE } from "../forcedPhotometry";
import { formatWithUncertainty } from "../formatUncertainty";
import type { FluxConvention, PhotCal, StarPhotometry } from "../../services/analysis";

function phot(overrides: Partial<StarPhotometry> = {}): StarPhotometry {
  return {
    x: 10,
    y: 20,
    peak: 900,
    net_flux: 1000,
    flux_err: 10,
    mag_inst: -7.5,
    snr: 100,
    fwhm: 3.2,
    aperture_radius: 5,
    aperture_pixels: 79,
    aperture_area: 78.5,
    bg_mean: 100,
    bg_sigma: 2,
    bg_pixels: 300,
    saturated: false,
    saturation_source: "none",
    n_masked: 0,
    n_nonfinite: 0,
    n_saturated: 0,
    err_used: false,
    aperture_correction: null,
    ee_in_aperture: null,
    flux_total: null,
    plateau_radius: null,
    flux_jy: null,
    flux_err_jy: null,
    mag_ab: null,
    mag_ab_err: null,
    mag_ab_total: null,
    st_mag: null,
    sky_inner: 10,
    sky_outer: 15,
    growth_curve: [],
    ee50_radius: null,
    ee80_radius: null,
    warnings: [],
    ...overrides,
  };
}

function row(index: number, p: Partial<StarPhotometry> | null, overrides: Partial<PhotometryTableRow> = {}): PhotometryTableRow {
  return { index, label: `s${index}`, photometry: p ? phot(p) : null, sky: null, error: p ? null : "failed", ...overrides };
}

function photcal(convention: FluxConvention, bunit: string | null): PhotCal {
  return { convention, bunit, notes: [], warnings: [], label: "calibration" };
}

const JWST_I2D = photcal({ kind: "jwst_mjy_sr", pixar_sr: 2.29e-14, pixar_a2: 0.000974, derived_pixar: false }, "MJy/sr");
const HST_RATE = photcal({ kind: "hst_counts", photflam: 1.5e-19, photplam: 6563, photzpt: -21.1, per_second: true, exptime: null }, "ELECTRONS/S");

describe("summedFluxUnit (B2)", () => {
  it("names an aperture sum on an MJy/sr image MJy/sr x pix, the unit of a sum over pixels, not the surface brightness", () => {
    expect(summedFluxUnit(JWST_I2D)).toBe("MJy/sr x pix");
  });

  it("keeps BUNIT for count, rate and zero-point images, where the sum over pixels is a flux in the pixel unit", () => {
    expect(summedFluxUnit(HST_RATE)).toBe("ELECTRONS/S");
    expect(summedFluxUnit(photcal({ kind: "jwst_dn_per_sec", photmjsr: 0.5, pixar_sr: 2.1e-13 }, "DN/s"))).toBe("DN/s");
    expect(summedFluxUnit(photcal({ kind: "roman_dn_per_sec", mjy_per_dn_s: 0.3324, pixar_sr: 2.8e-13 }, "DN / s"))).toBe("DN / s");
    expect(summedFluxUnit(photcal({ kind: "generic_zero_point", zp: 25, keyword: "MAGZPT", per_second: false, exptime: 300 }, "ADU"))).toBe("ADU");
  });

  it("names no unit when the header has no BUNIT or no calibration was read", () => {
    expect(summedFluxUnit(photcal({ kind: "generic_zero_point", zp: 25, keyword: "MAGZPT", per_second: false, exptime: null }, null))).toBeNull();
    expect(summedFluxUnit(null)).toBeNull();
    expect(summedFluxUnit(undefined)).toBeNull();
  });

  it("takes the unit of an image calibrated by an assumed unit from the calibration", () => {
    const assumed: PhotCal = {
      convention: { kind: "flux_density_per_pixel", jy_per_unit: 3.630780547701014e-6, unit: "nanomaggy" },
      bunit: null,
      notes: [],
      warnings: [],
      label: "nanomaggy per pixel",
    };
    expect(summedFluxUnit(assumed)).toBe("nanomaggy");
    expect(summedFluxUnit({ ...assumed, bunit: "nanomaggy" })).toBe("nanomaggy");
  });
});

describe("parsePositions", () => {
  it("accepts space, comma and tab separated pairs and skips comments and blanks", () => {
    const { points, errors } = parsePositions("# header\n10 20\n\n10,20\n10.5\t20.25\n  30 , 40  \n");
    expect(points).toEqual([
      [10, 20],
      [10, 20],
      [10.5, 20.25],
      [30, 40],
    ]);
    expect(errors).toEqual([]);
  });

  it("reports the line number of a malformed entry and keeps the valid ones", () => {
    const { points, errors } = parsePositions("1 2\nabc\n3 4 5\n7 NaN\n\n8 9");
    expect(points).toEqual([
      [1, 2],
      [8, 9],
    ]);
    expect(errors).toHaveLength(3);
    expect(errors[0]).toContain("line 2");
    expect(errors[1]).toContain("line 3");
    expect(errors[2]).toContain("line 4");
  });

  it("returns nothing for an empty text", () => {
    expect(parsePositions("")).toEqual({ points: [], errors: [] });
  });
});

describe("flagDuplicates", () => {
  it("flags the second of two rows half a pixel apart and not rows two pixels apart", () => {
    const rows = [row(0, { x: 10, y: 10 }), row(1, { x: 10.3, y: 10.4 }), row(2, { x: 12, y: 10 }), row(3, null)];
    expect([...flagDuplicates(rows)]).toEqual([1]);
    expect(flagDuplicates(rows, 0.1).size).toBe(0);
    expect(flagDuplicates(rows, 3).size).toBe(2);
  });

  it("flags matches across grid cell boundaries", () => {
    const rows = [row(0, { x: 0.99, y: 0.99 }), row(1, { x: 1.01, y: 1.01 })];
    expect([...flagDuplicates(rows)]).toEqual([1]);
  });

  it("ignores an invalid tolerance", () => {
    const rows = [row(0, { x: 1, y: 1 }), row(1, { x: 1, y: 1 })];
    expect(flagDuplicates(rows, 0).size).toBe(0);
    expect(flagDuplicates(rows, Number.NaN).size).toBe(0);
  });
});

describe("sortRows", () => {
  const rows = [row(0, { snr: 5 }), row(1, null), row(2, { snr: 50 }), row(3, { snr: Number.NaN }), row(4, { snr: 20 })];

  it("sorts numerically with nulls last in both directions", () => {
    expect(sortRows(rows, "snr", "asc").map((r) => r.index)).toEqual([0, 4, 2, 1, 3]);
    expect(sortRows(rows, "snr", "desc").map((r) => r.index)).toEqual([2, 4, 0, 1, 3]);
  });

  it("sorts strings and booleans and keeps the input order for ties", () => {
    const mixed = [row(0, { saturated: true }), row(1, { saturated: false }), row(2, { saturated: true })];
    expect(sortRows(mixed, "saturated", "desc").map((r) => r.index)).toEqual([0, 2, 1]);
    expect(sortRows(mixed, "label", "desc").map((r) => r.index)).toEqual([2, 1, 0]);
    expect(sortRows(rows, "index", "asc")).not.toBe(rows);
  });
});

const CSV_HEADER_GOLDEN =
  "index,label,x,y,ra,dec,net_flux,flux_err,snr,mag_inst,mag_ab,mag_ab_err,flux_jy,flux_err_jy,fwhm,aperture_radius,sky_inner,sky_outer,bg_mean,bg_sigma,aperture_correction,flux_total,mag_ab_total,ee50_radius,ee80_radius,saturated,n_masked,err_used,error,n_nonfinite,ee_in_aperture,mag,mag_err,mag_system,upper_limit_jy,upper_limit_ab";

describe("photometryTableCsv", () => {
  const cellOf = (line: string, key: (typeof PHOTOMETRY_COLUMN_KEYS)[number]) => line.split(",")[PHOTOMETRY_COLUMN_KEYS.indexOf(key)];

  it("writes the columns in the specified order with blank nulls", () => {
    expect(PHOTOMETRY_CSV_COLUMNS.map((c) => c.header)).toEqual([...PHOTOMETRY_COLUMN_KEYS]);
    expect(PHOTOMETRY_COLUMN_KEYS[0]).toBe("index");
    const rows = [
      row(0, { x: 1.5, y: 2.5, mag_ab: 17.25, saturated: true, err_used: true }, { sky: { ra: 150.1, dec: 2.2 } }),
      row(1, null),
    ];
    const csv = photometryTableCsv(rows);
    const lines = csv.split(CSV_LINE_END);
    expect(lines[0]).toBe(CSV_HEADER_GOLDEN);
    const first = lines[1].split(",");
    expect(first.slice(0, 6)).toEqual(["0", "s0", "1.5", "2.5", "150.1", "2.2"]);
    expect(first[10]).toBe("17.25");
    expect(cellOf(lines[1], "saturated")).toBe("true");
    expect(cellOf(lines[1], "err_used")).toBe("true");
    expect(cellOf(lines[1], "error")).toBe("");
    const second = lines[2].split(",");
    expect(second[0]).toBe("1");
    expect(second[2]).toBe("");
    expect(cellOf(lines[2], "error")).toBe("failed");
    expect(second.length).toBe(PHOTOMETRY_COLUMN_KEYS.length);
  });

  it("appends the count of aperture pixels without data and the enclosed fraction after the error column", () => {
    const lines = photometryTableCsv([row(0, { n_nonfinite: 3, aperture_correction: 1 / 0.93, ee_in_aperture: 0.93 }), row(1, null)]).split(CSV_LINE_END);
    expect(lines[0].endsWith(",err_used,error,n_nonfinite,ee_in_aperture,mag,mag_err,mag_system,upper_limit_jy,upper_limit_ab")).toBe(true);
    expect(cellOf(lines[1], "n_nonfinite")).toBe("3");
    expect(cellOf(lines[1], "ee_in_aperture")).toBe("0.93");
    expect(Number(cellOf(lines[1], "aperture_correction"))).toBeCloseTo(1.0753, 4);
    expect(cellOf(lines[2], "n_nonfinite")).toBe("");
    expect(cellOf(lines[2], "ee_in_aperture")).toBe("");
  });

  it("leaves the FWHM cell empty when the FWHM was not measured", () => {
    const lines = photometryTableCsv([row(0, { fwhm: null })]).split(CSV_LINE_END);
    expect(cellOf(lines[1], "fwhm")).toBe("");
  });

  it("blanks non-finite numbers", () => {
    expect(rowValue(row(0, { mag_inst: Number.NaN }), "mag_inst")).toBeNull();
    expect(rowValue(row(0, null), "n_masked")).toBeNull();
  });

  it("appends the calibrated magnitude, its error and its system after the enclosed fraction", () => {
    expect(PHOTOMETRY_COLUMN_KEYS).toHaveLength(36);
    expect(PHOTOMETRY_COLUMN_KEYS.slice(30, 34)).toEqual(["ee_in_aperture", "mag", "mag_err", "mag_system"]);
    const vega = row(0, { mag_inst: -9.87, mag_ab: null }, { magnitude: { mag: 11.562, magErr: 0.00612, system: "vega" } });
    expect(rowValue(vega, "mag")).toBe(11.562);
    expect(rowValue(vega, "mag_err")).toBe(0.00612);
    expect(rowValue(vega, "mag_system")).toBe("vega");
    expect(rowValue(vega, "mag_ab")).toBeNull();
    const plain = row(1, { mag_ab: 17.5 });
    expect(rowValue(plain, "mag")).toBeNull();
    expect(rowValue(plain, "mag_err")).toBeNull();
    expect(rowValue(plain, "mag_system")).toBeNull();
    const lines = photometryTableCsv([vega, plain]).split(CSV_LINE_END);
    expect(lines[0].endsWith(",ee_in_aperture,mag,mag_err,mag_system,upper_limit_jy,upper_limit_ab")).toBe(true);
    expect(lines[1].endsWith(",11.562,0.00612,vega,,")).toBe(true);
    expect(lines[2].endsWith(",,,,,")).toBe(true);
  });

  it("appends the upper limit in Jy and in AB magnitude as the last two columns", () => {
    expect(PHOTOMETRY_COLUMN_KEYS.slice(-2)).toEqual(["upper_limit_jy", "upper_limit_ab"]);
    const limit = { sigma: 3, fCorr: null, isLimit: true, limitJy: 3.3e-7, limitAb: 25.103715150305284, limitNative: 36.900000000000006 };
    const faint = row(0, { net_flux: 20, flux_err: 12.3, flux_jy: 2.0e-7, flux_err_jy: 1.1e-7 }, { limit });
    expect(rowValue(faint, "upper_limit_jy")).toBe(3.3e-7);
    expect(rowValue(faint, "upper_limit_ab")).toBe(25.103715150305284);
    const bright = row(1, { net_flux: 50, flux_jy: 5.0e-7, flux_err_jy: 1.1e-7 }, {
      limit: { ...limit, isLimit: false, limitJy: null, limitAb: null, limitNative: null },
    });
    expect(rowValue(bright, "upper_limit_jy")).toBeNull();
    expect(rowValue(bright, "upper_limit_ab")).toBeNull();
    const plain = row(2, { flux_jy: 2.0e-7 });
    expect(rowValue(plain, "upper_limit_jy")).toBeNull();
    expect(rowValue(plain, "upper_limit_ab")).toBeNull();
    const lines = photometryTableCsv([faint, bright, plain]).split(CSV_LINE_END);
    expect(lines[0]).toBe(CSV_HEADER_GOLDEN);
    expect(cellOf(lines[1], "flux_jy")).toBe("2e-7");
    expect(lines[1].split(",").slice(-2).map(Number)).toEqual([3.3e-7, 25.103715150305284]);
    expect(lines[2].split(",").slice(-2)).toEqual(["", ""]);
    expect(lines[3].split(",").slice(-2)).toEqual(["", ""]);
  });
});

describe("medianSnr", () => {
  it("takes the median over measured rows only", () => {
    expect(medianSnr([row(0, { snr: 1 }), row(1, null), row(2, { snr: 9 }), row(3, { snr: 4 })])).toBe(4);
    expect(medianSnr([row(0, { snr: 1 }), row(1, { snr: 3 })])).toBe(2);
    expect(medianSnr([row(0, null)])).toBeNull();
  });
});

describe("photometryCsvFileName", () => {
  it("derives the name from the image stem", () => {
    expect(photometryCsvFileName("/data/ngc1234_cal.fits")).toBe("ngc1234_cal_photometry.csv");
    expect(photometryCsvFileName("C:\\img\\m31.fit.gz")).toBe("m31_photometry.csv");
  });

  it("drops a #hdu= or #array= plane fragment but keeps '#' that belongs to the path", () => {
    expect(photometryCsvFileName("C:\\data\\jw02733_cal.fits#hdu=1")).toBe("jw02733_cal_photometry.csv");
    expect(photometryCsvFileName("/data/r0000.asdf#array=roman.dq")).toBe("r0000_photometry.csv");
    expect(photometryCsvFileName("/data/run#2/m51.fits")).toBe("m51_photometry.csv");
    expect(photometryCsvFileName("/data/obs#1.fits")).toBe("obs#1_photometry.csv");
  });

  it("drops the compression suffix of tile-compressed and bzip2 files from the saved name", () => {
    expect(photometryCsvFileName("C:/d/legacysurvey-0532m280-image-r.fits.fz")).toBe("legacysurvey-0532m280-image-r_photometry.csv");
    expect(photometryCsvFileName("C:/d/frame-r-001000-1-0027.fits.bz2")).toBe("frame-r-001000-1-0027_photometry.csv");
  });
});

const CONTEXT: PhotometryCsvContext = {
  exportedUtc: "2026-11-12T18:00:00.000Z",
  fileName: "brick-r.fits.fz",
  image: "original",
  positions: "sky",
  bunit: "nanomaggy",
  summedUnit: "nanomaggy",
  calibration: "BUNIT nanomaggy, zero point 22.5",
  calibrationNotes: [],
  magSystem: "ab",
  zeroPoint: null,
  filter: { dataset: "DECAM", filter: "r", pivot_um: 0.64176, source: "header FILTER" },
  pixelScaleArcsec: 0.262,
  radiusUnit: "arcsec",
  typedRadii: { aperture: 2 },
  gain: null,
  excludeDq: true,
  masked: true,
  errLayer: "file name -invvar-, brick-invvar-r.fits.fz: 1 / square root",
  dqLayer: "file name -maskbits, brick-maskbits.fits.fz, MBIT_ cards (Legacy maskbits) flags",
  warnings: [],
  limitSigma: 3,
  fCorr: 1.647,
  fCorrRadius: "r = 1 arcsec",
};

describe("photometry table provenance (C-CSVPROV)", () => {
  const entry = (context: Partial<PhotometryCsvContext>, key: string) =>
    photometryCsvProvenance({ ...CONTEXT, ...context }, 1).find(([k]) => k === key)?.[1];
  const tableRows = () => [row(0, { x: 1.5, y: 2.5, mag_ab: 17.25 }, { sky: { ra: 150.1, dec: 2.2 } }), row(1, null)];

  it("leaves the table alone when no context is given", () => {
    const lines = photometryTableCsv(tableRows()).split(CSV_LINE_END);
    expect(lines.some((line) => line.startsWith("#"))).toBe(false);
    expect(lines[0]).toBe(CSV_HEADER_GOLDEN);
    expect(lines[0].split(",")).toHaveLength(36);
  });

  it("lists the provenance of a photometry table in the order of the contract", () => {
    expect(photometryCsvProvenance(CONTEXT, 69)).toEqual([
      ...exportEntries("brick-r.fits.fz", "2026-11-12T18:00:00.000Z"),
      ["image", "original"],
      ["rows", 69],
      ["position_source", "sky positions (ICRS) placed with the WCS of the image"],
      ["bunit", "nanomaggy"],
      ["calibration", "BUNIT nanomaggy, zero point 22.5"],
      ["mag_system", "ab"],
      ["zero_point_typed", "none"],
      ["filter", "DECAM r, pivot 0.64176 um, from the header FILTER"],
      ["pixel_scale_arcsec", 0.262],
      ["aperture_radius", "2 arcsec (7.63359 px)"],
      ["sky_annulus", "automatic: 2 to 3 x the aperture radius"],
      ["gain", "none"],
      ["dq_exclusion", "applied"],
      ["error_layer", CONTEXT.errLayer],
      ["mask_layer", CONTEXT.dqLayer],
      ["aperture_weights", PHOTOMETRY_WEIGHTS_NOTE],
      ["flux_error", PHOTOMETRY_ERROR_NOTE],
      ["limit_sigma", 3],
      ["f_corr", "1.647 from the Depth section (r = 1 arcsec), applied on request to rows measured with an error layer"],
      ["upper_limit", PHOTOMETRY_LIMIT_NOTE],
      ["aperture_correction", PHOTOMETRY_CORRECTION_NOTE],
      ["positions", PHOTOMETRY_POSITIONS_NOTE],
    ]);
    expect(entry({ fCorr: null, fCorrRadius: null }, "f_corr")).toBe("not applied");
    expect(entry({ fCorr: 1.65, fCorrRadius: null }, "f_corr")).toBe("1.65 from the Depth section, applied on request to rows measured with an error layer");
    expect(PHOTOMETRY_LIMIT_NOTE).toBe(
      "a row is a limit when flux_jy < limit_sigma x flux_err_jy x f_corr; then upper_limit_jy = limit_sigma x flux_err_jy x f_corr and upper_limit_ab = -2.5 log10(upper_limit_jy) + 8.90; f_corr is 1 unless the line f_corr names a factor; the measured flux and error are kept",
    );
  });

  it("words every variant of the request and of the calibration", () => {
    expect(entry({ typedRadii: {} }, "aperture_radius")).toBe("automatic: 1.5 x FWHM of each star, 6 px where the FWHM is not measured");
    const typedPx: Partial<PhotometryCsvContext> = { radiusUnit: "px", typedRadii: { aperture: 5, skyInner: 10, skyOuter: 15 } };
    expect(entry(typedPx, "aperture_radius")).toBe("5 px (1.31 arcsec)");
    expect(entry(typedPx, "sky_annulus")).toBe("10 to 15 px");
    expect(entry({ ...typedPx, pixelScaleArcsec: null }, "aperture_radius")).toBe("5 px");
    expect(entry({ positions: "stars" }, "position_source")).toBe("detected stars");
    expect(entry({ positions: "regions" }, "position_source")).toBe("point regions");
    expect(entry({ positions: "pasted" }, "position_source")).toBe("pasted pixel positions");
    expect(entry({ calibration: null, magSystem: null }, "calibration")).toBe("none");
    expect(entry({ calibration: null, magSystem: null }, "mag_system")).toBe("none");
    expect(entry({ magSystem: "unknown" }, "mag_system")).toBe("unknown (AB assumed)");
    expect(entry({ zeroPoint: { zp: 21.432, system: "vega", per_second: true } }, "zero_point_typed")).toBe("21.432 Vega, per second");
    expect(entry({ zeroPoint: { zp: 25, system: "ab", per_second: false } }, "zero_point_typed")).toBe("25 AB");
    expect(entry({ filter: null }, "filter")).toBe("not identified");
    expect(entry({ gain: 2.5 }, "gain")).toBe(2.5);
    expect(entry({ excludeDq: false }, "dq_exclusion")).toBe("off");
    expect(entry({ excludeDq: true, masked: false }, "dq_exclusion")).toBe("requested, not applied");
    expect(entry({ errLayer: null }, "error_layer")).toBe("none: errors from the sky scatter (column err_used is false)");
    expect(entry({ dqLayer: null }, "mask_layer")).toBe("none");
  });

  it("ends the provenance with the calibration notes and the warnings", () => {
    const entries = photometryCsvProvenance(
      { ...CONTEXT, calibrationNotes: ["zero point user: mag = 25.0 - 2.5 log10(sum)"], warnings: ["w", "w", "v"] },
      1,
    );
    expect(entries.slice(-4)).toEqual([
      ["positions", PHOTOMETRY_POSITIONS_NOTE],
      ["calibration_note", "zero point user: mag = 25.0 - 2.5 log10(sum)"],
      ["warning", "w"],
      ["warning", "v"],
    ]);
  });

  it("names the unit of every numeric column", () => {
    const units = photometryCsvUnits(CONTEXT);
    for (const key of ["net_flux", "flux_err", "flux_total", "bg_mean", "bg_sigma"]) expect(units[key], key).toBe("nanomaggy");
    expect(units.x).toBe("pixel (0-based)");
    expect(units.ra).toBe("deg (ICRS)");
    expect(units.mag_ab).toBe("mag (AB)");
    expect(units.mag_ab_total).toBe("mag (AB)");
    expect(units.flux_jy).toBe("Jy");
    expect(units.fwhm).toBe("pixel");
    expect(units.mag).toBe("mag (system of the column mag_system)");
    expect(Object.keys(units)).toEqual([
      "x", "y", "ra", "dec", "net_flux", "flux_err", "mag_inst", "mag_ab", "mag_ab_err", "flux_jy", "flux_err_jy",
      "fwhm", "aperture_radius", "sky_inner", "sky_outer", "bg_mean", "bg_sigma", "flux_total", "mag_ab_total",
      "ee50_radius", "ee80_radius", "mag", "mag_err", "upper_limit_jy", "upper_limit_ab",
    ]);
    expect(units.upper_limit_jy).toBe("Jy");
    expect(units.upper_limit_ab).toBe("mag (AB)");
    const columns: readonly string[] = PHOTOMETRY_COLUMN_KEYS;
    expect(Object.keys(units).every((key) => columns.includes(key))).toBe(true);
    expect(Object.keys(units)).toEqual(columns.filter((key) => key in units));
    const surface = photometryCsvUnits({ ...CONTEXT, bunit: "MJy/sr", summedUnit: "MJy/sr x pix" });
    expect(surface.bg_mean).toBe("MJy/sr");
    expect(surface.net_flux).toBe("MJy/sr x pix");
    const unknown = photometryCsvUnits({ ...CONTEXT, bunit: null, summedUnit: null });
    expect(unknown.bg_mean).toBe("image unit");
    expect(unknown.net_flux).toBe("image unit, summed over the aperture pixels");
  });

  it("puts the provenance and the unit lines before the table", () => {
    const rows = tableRows();
    const csv = photometryTableCsv(rows, CONTEXT);
    expect(csv).toBe(
      withProvenance(
        [...provenanceLines(photometryCsvProvenance(CONTEXT, rows.length)), ...unitLines(photometryCsvUnits(CONTEXT))],
        photometryTableCsv(rows),
        CSV_LINE_END,
      ),
    );
    const lines = csv.split(CSV_LINE_END);
    expect(lines.slice(0, 3)).toEqual([`# software: AstroBurst ${APP_VERSION}`, "# exported_utc: 2026-11-12T18:00:00.000Z", "# file: brick-r.fits.fz"]);
    expect(lines).toContain("# calibration: BUNIT nanomaggy, zero point 22.5");
    expect(lines).toContain("# filter: DECAM r, pivot 0.64176 um, from the header FILTER");
    expect(lines).toContain(`# flux_error: ${PHOTOMETRY_ERROR_NOTE}`);
    expect(lines).toContain("# unit net_flux: nanomaggy");
    expect(lines.find((line) => !line.startsWith("#"))).toBe(CSV_HEADER_GOLDEN);
  });

  it("states the error model as the backend computes it", () => {
    expect(PHOTOMETRY_ERROR_NOTE).toContain("sqrt(sum (w sigma)^2)");
    expect(PHOTOMETRY_ERROR_NOTE).toContain("n_ap^2 sigma_sky^2 / n_sky");
    expect(PHOTOMETRY_WEIGHTS_NOTE).toBe("5 x 5 subsampled overlap of the aperture with boundary pixels");
    expect(PHOTOMETRY_WEIGHTS_NOTE).toBe(FORCED_WEIGHTS_NOTE);
  });
});

describe("plateauCaption", () => {
  it("shows the growth-curve plateau radius at the precision of the reference line", () => {
    expect(plateauCaption(11.5)).toBe(" (growth curve plateau at r=11.50 px)");
    expect(plateauCaption((21.3 / 40) * 50)).toBe(" (growth curve plateau at r=26.63 px)");
    expect(plateauCaption(12)).toBe(" (growth curve plateau at r=12.00 px)");
    expect(plateauCaption(null)).toBe("");
    expect(plateauCaption(Number.NaN)).toBe("");
    const p = phot({ aperture_radius: 10.6, sky_inner: 21.2, sky_outer: 31.8, plateau_radius: 11.5, ee50_radius: 3.53 });
    const line = growthReferenceLines(p).find((l) => l.label === "plateau");
    expect(plateauCaption(p.plateau_radius)).toContain(`r=${line!.value.toFixed(2)} px`);
  });
});

describe("apertureRadiusInRange", () => {
  it("accepts a blank radius and the inclusive 2 to 60 px range the measurement uses without clamping", () => {
    expect(apertureRadiusInRange(undefined)).toBe(true);
    expect(apertureRadiusInRange(MIN_APERTURE_RADIUS_PX)).toBe(true);
    expect(apertureRadiusInRange(MAX_APERTURE_RADIUS_PX)).toBe(true);
    expect(apertureRadiusInRange(5)).toBe(true);
  });

  it("refuses the radii the core would silently clamp", () => {
    expect(apertureRadiusInRange(1.5)).toBe(false);
    expect(apertureRadiusInRange(70)).toBe(false);
    expect([MIN_APERTURE_RADIUS_PX, MAX_APERTURE_RADIUS_PX]).toEqual([2, 60]);
  });

  it("caps the sky annulus outer radius at the 512 px the commands accept", () => {
    expect(MAX_SKY_OUTER_RADIUS_PX).toBe(512);
  });
});

describe("BATCH_SEMANTICS_TEXT", () => {
  it("states the square peak-search box and that recentring can reach beyond 8 px", () => {
    expect(BATCH_SEMANTICS_TEXT).toContain("17 x 17 px box");
    expect(BATCH_SEMANTICS_TEXT).toMatch(/beyond 8 px/);
    expect(BATCH_SEMANTICS_TEXT).not.toMatch(/within 8 px/);
  });

  it("states the 6 px radius a blank aperture takes when the FWHM of a star is not measured (1.5 x the 4 px default)", () => {
    expect(FALLBACK_APERTURE_RADIUS_PX).toBe(6);
    expect(BATCH_SEMANTICS_TEXT).toContain("(leave it blank for 1.5 x FWHM per star, or 6 px where the FWHM is not measured)");
  });
});

describe("growthReferenceLines", () => {
  it("draws the aperture, both sky radii, the plateau and EE50 as vertical lines in that order", () => {
    const lines = growthReferenceLines(phot({ aperture_radius: 5, sky_inner: 10, sky_outer: 15, plateau_radius: 12, ee50_radius: 2.5 }));
    expect(lines.map((l) => l.label)).toEqual(["r_ap", "sky in", "sky out", "plateau", "EE50"]);
    expect(lines.map((l) => l.value)).toEqual([5, 10, 15, 12, 2.5]);
    expect(lines.every((l) => l.axis === "x")).toBe(true);
    expect(lines.map((l) => l.dashed)).toEqual([false, false, false, true, true]);
    expect(lines[0].color).not.toBe(lines[1].color);
    expect(lines[1].color).toBe(lines[2].color);
    expect(lines[3].color).toBe(lines[4].color);
  });

  it("omits null and non-finite radii without disturbing the order of the rest", () => {
    const lines = growthReferenceLines(phot({ aperture_radius: Number.NaN, sky_inner: 10, sky_outer: Number.POSITIVE_INFINITY, plateau_radius: null, ee50_radius: 3 }));
    expect(lines.map((l) => l.label)).toEqual(["sky in", "EE50"]);
  });

  it("returns nothing when no radius is usable", () => {
    expect(growthReferenceLines(phot({ aperture_radius: Number.NaN, sky_inner: Number.NaN, sky_outer: Number.NaN }))).toEqual([]);
  });
});

describe("fluxCellText (UX-07)", () => {
  it("prints a faint calibrated flux to four significant figures, never as 0.0", () => {
    expect(fluxCellText(0.0312)).toBe("0.0312");
    expect(fluxCellText(0.0312)).not.toBe("0.0");
    expect(fluxCellText(0.004)).toBe("0.004");
    expect(fluxCellText(-0.00012346)).toBe("-0.0001235");
  });

  it("rounds every magnitude of flux to four significant figures", () => {
    for (const v of [0.0312, 0.123456, 12.3456, 1234.5, 430123.4, 1234567.8, -98765.4]) {
      expect(Number(fluxCellText(v))).toBe(Number(v.toPrecision(4)));
    }
    expect(fluxCellText(430123.4)).toBe("430100");
  });

  it("switches to exponent form from 10^6, as the other table columns do, so no padded zeros read as measured", () => {
    expect(fluxCellText(23456789)).toBe("2.346e+7");
    expect(fluxCellText(-6554321)).toBe("-6.554e+6");
    expect(fluxCellText(1234567.8)).toBe("1.235e+6");
    expect(fluxCellText(999949)).toBe("999900");
  });

  it("keeps the placeholder for missing values", () => {
    expect(fluxCellText(null)).toBe("--");
    expect(fluxCellText(undefined)).toBe("--");
    expect(fluxCellText(Number.NaN)).toBe("--");
    expect(fluxCellText(0)).toBe("0");
  });
});

describe("abCellText (UX-07)", () => {
  it("prints the AB magnitude with its error to the millimag, as the single-star panel does", () => {
    expect(abCellText(24.512, 0.031)).toContain("±");
    expect(abCellText(24.512, 0.031)).toBe("24.512 ± 0.031");
  });

  it("prints the magnitude alone when the error is missing and the placeholder when the magnitude is", () => {
    expect(abCellText(24.512, null)).toBe("24.512");
    expect(abCellText(24.512, Number.NaN)).toBe("24.512");
    expect(abCellText(null, 0.031)).toBe("--");
    expect(abCellText(undefined, undefined)).toBe("--");
  });

  it("rounds the magnitude to the two leading digits of its error", () => {
    expect(abCellText(24.51234, 0.2512)).toBe(formatWithUncertainty(24.51234, 0.2512));
    expect(abCellText(24.51234, 0.2512)).not.toBe("24.512 ± 0.251");
    expect(abCellText(24.51234, 0.2512)).toBe("24.51 ± 0.25");
  });
});

describe("photometry table captions", () => {
  it("names the units of the table columns", () => {
    expect(tableUnitsCaption("MJy/sr x pix")).toBe("units: flux MJy/sr x pix; AB, inst mag; FWHM, EE50 px");
    const notStated = "units: flux in the pixel unit of the image (not stated in the header); AB, inst mag; FWHM, EE50 px";
    expect(tableUnitsCaption(null)).toBe(notStated);
    expect(tableUnitsCaption("")).toBe(notStated);
  });

  it("says where detected stars come from", () => {
    expect(NO_DETECTED_STARS_HINT).toContain("Detect Stars");
    expect(NO_DETECTED_STARS_HINT).toContain("Astrometry");
  });
});
