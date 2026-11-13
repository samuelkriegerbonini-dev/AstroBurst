import { describe, expect, it } from "vitest";
import type {
  CompareBandInfo,
  CompareColumn,
  ComparePreset,
  CompareProbe,
  CompareReport,
  CompareRequest,
  CompareRow,
} from "../../shared/types/catalogCompare";
import {
  apertureUsedText,
  compareCsvFileName,
  compareCsvHeaderLines,
  compareCsvText,
  compareHeadline,
  compareMarkRows,
  compareRequestOf,
  compareRowsCsv,
  CUSTOM_PRESET_KEY,
  emptyCompareForm,
  fluxCellUjy,
  formEdited,
  formFromPreset,
  initialCompareForm,
  microJy,
  orderedRows,
  presetKeyOf,
  presetLabel,
  ratioCell,
  rejectedText,
  signedFixed,
  type CompareCsvContext,
  type CompareForm,
  type CompareRequestResult,
} from "../catalogCompare";
import { exportEntries, provenanceLines, unitLines, type ProvenanceEntry } from "../csvProvenance";
import { formatWithUncertainty } from "../formatUncertainty";

const LEGACY_R: ComparePreset = {
  id: "legacy_dr10",
  label: "Legacy Surveys DR10",
  band: "r",
  mapping: {
    id: "ls_id",
    ra: "ra",
    dec: "dec",
    flux: "apflux_r_5",
    flux_unit: "nanomaggy",
    zero_point: null,
    flux_err: null,
    flux_ivar: "apflux_ivar_r_5",
    class: "type",
    missing_value: null,
  },
  aperture: { radius_arcsec: 2, radius_px: null, annulus_arcsec: null, annulus_px: null },
  cuts: [
    { column: "type", op: "==", value: "PSF" },
    { column: "maskbits", op: "==", value: 0 },
    { column: "fracflux_r", op: "<", value: 0.01 },
  ],
  min_snr: null,
  max_snr: null,
  min_neighbour_arcsec: null,
  notes: [],
};

const LEGACY_G: ComparePreset = {
  ...LEGACY_R,
  band: "g",
  mapping: { ...LEGACY_R.mapping, flux: "apflux_g_5", flux_ivar: "apflux_ivar_g_5" },
  cuts: [
    { column: "type", op: "==", value: "PSF" },
    { column: "maskbits", op: "==", value: 0 },
    { column: "fracflux_g", op: "<", value: 0.01 },
  ],
};

const DES_R: ComparePreset = {
  id: "des_dr2",
  label: "DES DR2",
  band: "r",
  mapping: {
    id: "coadd_object_id",
    ra: "alphawin_j2000",
    dec: "deltawin_j2000",
    flux: "flux_aper_8_r",
    flux_unit: "zero_point",
    zero_point: 30,
    flux_err: "fluxerr_aper_8_r",
    flux_ivar: null,
    class: "extended_class_coadd",
    missing_value: null,
  },
  aperture: { radius_arcsec: null, radius_px: 11.11, annulus_arcsec: null, annulus_px: null },
  cuts: [
    { column: "flags_r", op: "<", value: 4 },
    { column: "imaflags_iso_r", op: "==", value: 0 },
  ],
  min_snr: 50,
  max_snr: null,
  min_neighbour_arcsec: null,
  notes: [],
};

const SDSS_R: ComparePreset = {
  id: "sdss_dr17",
  label: "SDSS DR17",
  band: "r",
  mapping: {
    id: "objID",
    ra: "ra",
    dec: "dec",
    flux: "aperFlux7_r",
    flux_unit: "nanomaggy",
    zero_point: null,
    flux_err: null,
    flux_ivar: null,
    class: "type",
    missing_value: null,
  },
  aperture: { radius_arcsec: null, radius_px: 18.58, annulus_arcsec: null, annulus_px: [28.55, 44.21] },
  cuts: [
    { column: "type", op: "==", value: 6 },
    { column: "clean", op: "==", value: 1 },
    { column: "psfMag_r", op: ">=", value: 15 },
    { column: "psfMag_r", op: "<=", value: 18 },
  ],
  min_snr: null,
  max_snr: null,
  min_neighbour_arcsec: null,
  notes: [],
};

const PS1_R: ComparePreset = {
  id: "ps1_dr2",
  label: "Pan-STARRS1 DR2",
  band: "r",
  mapping: {
    id: "objid",
    ra: "ramean",
    dec: "decmean",
    flux: "rflxr5",
    flux_unit: "Jy",
    zero_point: null,
    flux_err: "rflxr5err",
    flux_ivar: null,
    class: null,
    missing_value: -999,
  },
  aperture: { radius_arcsec: 3, radius_px: null, annulus_arcsec: null, annulus_px: null },
  cuts: [{ column: "rflxr5fill", op: ">=", value: 0.97 }],
  min_snr: 50,
  max_snr: null,
  min_neighbour_arcsec: null,
  notes: [],
};

const EUCLID_VIS: ComparePreset = {
  id: "euclid_q1",
  label: "Euclid Q1",
  band: "VIS",
  mapping: {
    id: "object_id",
    ra: "ra",
    dec: "dec",
    flux: "flux_detection_total",
    flux_unit: "uJy",
    zero_point: null,
    flux_err: null,
    flux_ivar: null,
    class: "point_like_prob",
    missing_value: null,
  },
  aperture: { radius_arcsec: 1.25, radius_px: null, annulus_arcsec: null, annulus_px: null },
  cuts: [
    { column: "point_like_prob", op: ">", value: 0.7 },
    { column: "spurious_flag", op: "==", value: 0 },
  ],
  min_snr: null,
  max_snr: null,
  min_neighbour_arcsec: 2,
  notes: [],
};

const IMG = "C:/d/legacysurvey-0532m280-image-r.fits.fz#hdu=1";

function probeFor(preset: ComparePreset, textColumns: string[] = []): CompareProbe {
  const m = preset.mapping;
  const used = [m.id, m.ra, m.dec, m.flux, m.flux_err, m.flux_ivar, m.class, ...preset.cuts.map((c) => c.column)];
  const names = [...new Set(used.filter((name): name is string => name !== null))];
  return {
    table_path: "C:/cat/t.csv",
    file_name: "t.csv",
    format: "csv",
    n_rows: 3,
    columns: names.map((name) => ({ name, numeric: name !== m.id && !textColumns.includes(name), unit: null })),
    presets: [preset],
    notes: [],
  };
}

function numericColumn(name: string): CompareColumn {
  return { name, numeric: true, unit: null };
}

function textColumn(name: string): CompareColumn {
  return { name, numeric: false, unit: null };
}

function probeWith(columns: CompareColumn[]): CompareProbe {
  return { table_path: "C:/cat/u.csv", file_name: "u.csv", format: "csv", n_rows: 5, columns, presets: [], notes: [] };
}

function sent(result: CompareRequestResult): CompareRequest {
  if (result.request === null) throw new Error(result.error);
  return result.request;
}

const BAND: CompareBandInfo = {
  file_name: "legacysurvey-0532m280-image-r.fits.fz",
  label: "r",
  filter: { dataset: "DECAM", filter: "r", pivot_um: 0.6418, source: "header FILTER" },
  calibration_origin: "BUNIT nanomaggy, zero point 22.5",
  jy_per_native_unit: 3.6307805477010175e-6,
  bunit: "nanomaggy",
  pixel_scale_arcsec: 0.262,
  aperture_radius_px: 7.633587786259541,
  annulus_px: null,
  err_origin: {
    role: "sigma",
    source: "inverse variance from legacysurvey-0532m280-invvar-r.fits.fz",
    assumed: false,
    n_no_data: 12,
  },
  dq_source: null,
  warnings: [],
};

const ROWS: CompareRow[] = [
  {
    row: 1,
    id: "10995300315432397",
    ra_deg: 53.2001,
    dec_deg: -27.9002,
    x: null,
    y: null,
    class: "REX",
    flux_cat_jy: 1e-4,
    flux_err_cat_jy: null,
    snr_cat: null,
    flux_native: null,
    flux_err_native: null,
    flux_jy: null,
    flux_err_jy: null,
    ratio: null,
    masked_fraction: null,
    used: false,
    reason: "cut type == PSF",
  },
  {
    row: 2,
    id: "10995300315432398",
    ra_deg: 53.2101,
    dec_deg: -27.9102,
    x: 3333.8115,
    y: 625.4192,
    class: "PSF",
    flux_cat_jy: 2.871799e-4,
    flux_err_cat_jy: 1.0e-7,
    snr_cat: 2871.799,
    flux_native: 79.1,
    flux_err_native: 0.03,
    flux_jy: 2.872e-4,
    flux_err_jy: 1.1e-7,
    ratio: 1.00007,
    masked_fraction: 0,
    used: true,
    reason: null,
  },
  {
    row: 3,
    id: "row 3",
    ra_deg: 53.2201,
    dec_deg: -27.9202,
    x: 10.5,
    y: 20.25,
    class: null,
    flux_cat_jy: 5e-5,
    flux_err_cat_jy: 2e-7,
    snr_cat: 250,
    flux_native: 13.5,
    flux_err_native: 0.05,
    flux_jy: 4.9e-5,
    flux_err_jy: 1.8e-7,
    ratio: 0.98,
    masked_fraction: 0.25,
    used: false,
    reason: "aperture touches pixels without data",
  },
  {
    row: 4,
    id: "a,b",
    ra_deg: 53.2301,
    dec_deg: -27.9302,
    x: 100,
    y: 200,
    class: "PSF",
    flux_cat_jy: 1e-4,
    flux_err_cat_jy: 1e-7,
    snr_cat: 1000,
    flux_native: 27.5,
    flux_err_native: 0.03,
    flux_jy: 9.99e-5,
    flux_err_jy: 1.1e-7,
    ratio: 0.999,
    masked_fraction: 0,
    used: true,
    reason: null,
  },
];

const REPORT: CompareReport = {
  n_table: 69,
  n_on_image: 69,
  n_after_cuts: 21,
  n_measured: 21,
  n_used: 21,
  ratio_median: 1.00007,
  ratio_nmad: 0.00021,
  ratio_median_err: 0.00012,
  n_ratio: 21,
  ratio_offset_mag: -0.000076,
  err_ratio_median: 0.98434,
  err_ratio_nmad: 0.0054,
  n_err: 21,
  implied_zero_point_ab: 22.50008,
  implied_zero_point_nmad: 0.0012,
  n_zero_point: 21,
  rows: ROWS,
  rejected: [
    { reason: "cut type == PSF", count: 33 },
    { reason: "cut maskbits == 0", count: 14 },
    { reason: "cut fracflux_r < 0.01", count: 1 },
  ],
  aperture: { radius_px: 7.633587786259541, radius_arcsec: 2, annulus_px: null, pixel_scale_arcsec: 0.262, local_sky: false },
  band: BAND,
  calibration_origin: "BUNIT nanomaggy, zero point 22.5",
  preset: "legacy_dr10",
  catalogue_band: "r",
  table: {
    path: "C:/cat/C1b_ls_dr10_brick_0532m280.csv",
    file_name: "C1b_ls_dr10_brick_0532m280.csv",
    format: "csv",
    n_rows: 69,
  },
  notes: ["note one", "note two"],
  warnings: ["w1"],
  elapsed_ms: 12,
};

const REQUEST: CompareRequest = sent(compareRequestOf(formFromPreset(LEGACY_R), IMG, probeFor(LEGACY_R, ["type"])));

const CONTEXT: CompareCsvContext = { exportedUtc: "2026-11-12T12:00:00.000Z", request: REQUEST, presetEdited: false };

describe("catalogue comparison form and request", () => {
  it("names a preset by its survey and its band", () => {
    expect(presetKeyOf(LEGACY_R)).toBe("legacy_dr10:r");
    expect(presetLabel(LEGACY_R)).toBe("Legacy Surveys DR10, r");
    expect(presetKeyOf(EUCLID_VIS)).toBe("euclid_q1:VIS");
    expect(presetLabel(EUCLID_VIS)).toBe("Euclid Q1, VIS");
  });

  it("fills the form from a preset, every value as text", () => {
    expect(formFromPreset(SDSS_R)).toEqual({
      presetKey: "sdss_dr17:r",
      catalogueBand: "r",
      idColumn: "objID",
      raColumn: "ra",
      decColumn: "dec",
      fluxColumn: "aperFlux7_r",
      fluxUnit: "nanomaggy",
      zeroPointText: "",
      errorKind: "none",
      errorColumn: "",
      classColumn: "type",
      missingValueText: "",
      radiusText: "18.58",
      radiusUnit: "px",
      localSky: true,
      annulusInnerText: "28.55",
      annulusOuterText: "44.21",
      cuts: [
        { column: "type", op: "==", value: "6" },
        { column: "clean", op: "==", value: "1" },
        { column: "psfMag_r", op: ">=", value: "15" },
        { column: "psfMag_r", op: "<=", value: "18" },
      ],
      minSnrText: "",
      maxSnrText: "",
      minNeighbourText: "",
      excludeDq: false,
    });
    expect(formFromPreset(DES_R)).toMatchObject({
      fluxUnit: "zero_point",
      zeroPointText: "30",
      errorKind: "flux_err",
      errorColumn: "fluxerr_aper_8_r",
      radiusText: "11.11",
      radiusUnit: "px",
      localSky: false,
      minSnrText: "50",
    });
    expect(formFromPreset(PS1_R)).toMatchObject({ missingValueText: "-999", classColumn: "", radiusText: "3", radiusUnit: "arcsec" });
    const legacy = formFromPreset(LEGACY_R);
    expect(legacy.errorKind).toBe("flux_ivar");
    expect(legacy.errorColumn).toBe("apflux_ivar_r_5");
    expect(legacy.cuts[0]).toEqual({ column: "type", op: "==", value: "PSF" });
    expect(formFromPreset(EUCLID_VIS)).toMatchObject({ minNeighbourText: "2", radiusText: "1.25" });
  });

  it("a preset survives the form unchanged", () => {
    const cases: [ComparePreset, string[]][] = [
      [LEGACY_R, ["type"]],
      [DES_R, []],
      [SDSS_R, []],
      [PS1_R, []],
      [EUCLID_VIS, []],
    ];
    for (const [preset, textColumns] of cases) {
      const probe = probeFor(preset, textColumns);
      const form = formFromPreset(preset);
      const result = compareRequestOf(form, IMG, probe);
      expect(result.error).toBeNull();
      const request = sent(result);
      expect(request.mapping).toEqual(preset.mapping);
      expect(request.aperture).toEqual(preset.aperture);
      expect(request.cuts).toEqual(preset.cuts);
      expect(request.min_snr).toBe(preset.min_snr);
      expect(request.max_snr).toBe(preset.max_snr);
      expect(request.min_neighbour_arcsec).toBe(preset.min_neighbour_arcsec);
      expect(request.preset).toBe(preset.id);
      expect(request.catalogue_band).toBe(preset.band);
      expect(request.band).toEqual({ path: IMG, label: null, dataset: null });
      expect(request.table_path).toBe("C:/cat/t.csv");
      expect(request.exclude_dq).toBe(false);
      expect(formEdited(form, probe)).toBe(false);
    }
  });

  it("an edited preset is marked and a custom form never is", () => {
    const probe = probeFor(LEGACY_R, ["type"]);
    const form = formFromPreset(LEGACY_R);
    const widened = { ...form, radiusText: "3" };
    expect(formEdited(widened, probe)).toBe(true);
    expect(sent(compareRequestOf(widened, IMG, probe)).preset).toBe("legacy_dr10");
    expect(formEdited({ ...form, radiusText: "2.0" }, probe)).toBe(false);
    expect(formEdited({ ...form, excludeDq: true }, probe)).toBe(false);
    expect(formEdited({ ...form, catalogueBand: "g" }, probe)).toBe(true);
    const custom = { ...form, presetKey: CUSTOM_PRESET_KEY };
    expect(formEdited(custom, probe)).toBe(false);
    expect(sent(compareRequestOf(custom, IMG, probe)).preset).toBeNull();
    expect(formEdited({ ...form, radiusText: "" }, probe)).toBe(true);
    expect(formEdited(form, null)).toBe(false);
  });

  it("starts from the first preset, or from guessed position columns", () => {
    const probe = probeFor(LEGACY_R, ["type"]);
    expect(initialCompareForm({ ...probe, presets: [LEGACY_G, LEGACY_R] })).toEqual(formFromPreset(LEGACY_G));
    expect(initialCompareForm(probeWith([numericColumn("RA"), numericColumn("DEC"), numericColumn("f")]))).toEqual({
      presetKey: "custom",
      catalogueBand: "",
      idColumn: "",
      raColumn: "RA",
      decColumn: "DEC",
      fluxColumn: "",
      fluxUnit: "uJy",
      zeroPointText: "",
      errorKind: "none",
      errorColumn: "",
      classColumn: "",
      missingValueText: "",
      radiusText: "",
      radiusUnit: "arcsec",
      localSky: false,
      annulusInnerText: "",
      annulusOuterText: "",
      cuts: [],
      minSnrText: "",
      maxSnrText: "",
      minNeighbourText: "",
      excludeDq: false,
    });
    const des = initialCompareForm(probeWith([numericColumn("alphawin_j2000"), numericColumn("deltawin_j2000")]));
    expect([des.raColumn, des.decColumn]).toEqual(["alphawin_j2000", "deltawin_j2000"]);
    const ps1 = initialCompareForm(probeWith([numericColumn("raMean"), numericColumn("decMean")]));
    expect([ps1.raColumn, ps1.decColumn]).toEqual(["raMean", "decMean"]);
    const unknown = initialCompareForm(probeWith([numericColumn("a"), numericColumn("b")]));
    expect([unknown.raColumn, unknown.decColumn]).toEqual(["", ""]);
    const none = emptyCompareForm(null);
    expect([none.raColumn, none.decColumn]).toEqual(["", ""]);
  });

  it("starts from the preset of the band the image names, else from the first", () => {
    const probe = { ...probeFor(LEGACY_R, ["type"]), presets: [LEGACY_G, LEGACY_R] };
    expect(initialCompareForm(probe, "r")).toEqual(formFromPreset(LEGACY_R));
    expect(initialCompareForm(probe, "R")).toEqual(formFromPreset(LEGACY_R));
    expect(initialCompareForm(probe, "g")).toEqual(formFromPreset(LEGACY_G));
    expect(initialCompareForm(probe, "F200W")).toEqual(formFromPreset(LEGACY_G));
    expect(initialCompareForm(probe, null)).toEqual(formFromPreset(LEGACY_G));
  });

  it("refuses a form that cannot be sent, first problem first", () => {
    const probe = probeWith([numericColumn("ra"), numericColumn("dec"), numericColumn("f"), numericColumn("e")]);
    const empty = emptyCompareForm(probe);
    const refusal = (form: CompareForm) => compareRequestOf(form, IMG, probe);
    expect(refusal(empty)).toEqual({ request: null, error: "Choose the flux column" });
    expect(refusal({ ...empty, raColumn: "" })).toEqual({ request: null, error: "Choose the RA column" });
    expect(refusal({ ...empty, decColumn: "" })).toEqual({ request: null, error: "Choose the Dec column" });
    expect(refusal({ ...empty, fluxColumn: "f", errorKind: "flux_err" })).toEqual({
      request: null,
      error: "Choose the error column",
    });
    const base: CompareForm = { ...empty, fluxColumn: "f", errorKind: "none", radiusText: "2" };
    expect(refusal(base).error).toBeNull();
    const cases: [Partial<CompareForm>, string][] = [
      [{ radiusText: "" }, "Aperture radius: Enter a number"],
      [{ radiusText: "0" }, "Aperture radius: Use a value of at least 0.001"],
      [{ radiusText: "2,5" }, "Aperture radius: Use a dot (.) as the decimal separator"],
      [{ fluxUnit: "zero_point", zeroPointText: "" }, "Zero point: Enter a number"],
      [{ minSnrText: "x" }, "Minimum S/N: Not a number"],
      [
        { localSky: true, annulusInnerText: "5", annulusOuterText: "5" },
        "Sky annulus: the outer radius must be larger than the inner radius",
      ],
      [{ cuts: [{ column: "", op: "==", value: "1" }] }, "Cut 1: choose a column"],
      [
        {
          cuts: [
            { column: "f", op: ">", value: "1" },
            { column: "f", op: "<", value: " " },
          ],
        },
        "Cut 2: type a value",
      ],
    ];
    for (const [change, error] of cases) {
      expect(refusal({ ...base, ...change })).toEqual({ request: null, error });
    }
  });

  it("types a cut value by its column", () => {
    const probe = probeWith([numericColumn("ra"), numericColumn("dec"), numericColumn("f"), numericColumn("n"), textColumn("t")]);
    const base: CompareForm = { ...emptyCompareForm(probe), fluxColumn: "f", radiusText: "2" };
    const cutValue = (column: string, value: string) =>
      sent(compareRequestOf({ ...base, cuts: [{ column, op: "==", value }] }, IMG, probe)).cuts[0].value;
    expect(cutValue("n", "6")).toBe(6);
    expect(cutValue("n", "abc")).toBe("abc");
    expect(cutValue("t", "6")).toBe("6");
    expect(cutValue("t", " PSF ")).toBe("PSF");
    expect(cutValue("n", "1e3")).toBe(1000);
  });

  it("sends the radius and the annulus in the unit chosen", () => {
    const probe = probeWith([numericColumn("ra"), numericColumn("dec"), numericColumn("f")]);
    const base: CompareForm = {
      ...emptyCompareForm(probe),
      fluxColumn: "f",
      radiusText: "3",
      radiusUnit: "arcsec",
      localSky: true,
      annulusInnerText: "5",
      annulusOuterText: "7.5",
    };
    expect(sent(compareRequestOf(base, IMG, probe)).aperture).toEqual({
      radius_arcsec: 3,
      radius_px: null,
      annulus_arcsec: [5, 7.5],
      annulus_px: null,
    });
    expect(sent(compareRequestOf({ ...base, radiusUnit: "px" }, IMG, probe)).aperture).toEqual({
      radius_arcsec: null,
      radius_px: 3,
      annulus_arcsec: null,
      annulus_px: [5, 7.5],
    });
    for (const radiusUnit of ["arcsec", "px"] as const) {
      const aperture = sent(compareRequestOf({ ...base, radiusUnit, localSky: false }, IMG, probe)).aperture;
      expect(aperture.annulus_arcsec).toBeNull();
      expect(aperture.annulus_px).toBeNull();
    }
  });

  it("maps the error column by its kind and sends a zero point only with its unit", () => {
    const probe = probeWith([numericColumn("ra"), numericColumn("dec"), numericColumn("f"), numericColumn("e")]);
    const base: CompareForm = { ...emptyCompareForm(probe), fluxColumn: "f", radiusText: "2" };
    const mappingOf = (change: Partial<CompareForm>) => sent(compareRequestOf({ ...base, ...change }, IMG, probe)).mapping;
    expect(mappingOf({ errorKind: "flux_err", errorColumn: "e" })).toMatchObject({ flux_err: "e", flux_ivar: null });
    expect(mappingOf({ errorKind: "flux_ivar", errorColumn: "e" })).toMatchObject({ flux_err: null, flux_ivar: "e" });
    expect(mappingOf({ errorKind: "none", errorColumn: "e" })).toMatchObject({ flux_err: null, flux_ivar: null });
    expect(mappingOf({ fluxUnit: "uJy", zeroPointText: "30" }).zero_point).toBeNull();
    expect(mappingOf({ fluxUnit: "zero_point", zeroPointText: "30" }).zero_point).toBe(30);
    expect(mappingOf({ idColumn: "", classColumn: "" })).toMatchObject({ id: null, class: null });
    expect(mappingOf({ missingValueText: "-999" }).missing_value).toBe(-999);
    expect(sent(compareRequestOf({ ...base, catalogueBand: "  " }, IMG, probe)).catalogue_band).toBeNull();
  });
});

describe("catalogue comparison report numbers", () => {
  it("converts Jy to uJy without float dust", () => {
    expect(microJy(2.871799e-4)).toBe(287.1799);
    expect(microJy(3.630780547701009e-6)).toBe(3.6307805477);
    expect(microJy(9.99e-5)).toBe(99.9);
    expect(microJy(null)).toBeNull();
    expect(microJy(undefined)).toBeNull();
    expect(microJy(Number.NaN)).toBeNull();
    expect(microJy(Number.POSITIVE_INFINITY)).toBeNull();
  });

  it("the headline counts every stage and gives the ratio with its error, scatter and N", () => {
    expect(compareHeadline(REPORT)).toEqual({
      counts: "69 rows in the table, 69 on the image, 21 after the cuts, 21 measured, 21 used",
      ratio: `${formatWithUncertainty(1.00007, 0.00012)} (scatter ${formatWithUncertainty(0.00021, null)}, N = 21)`,
      offset: "+0.007 percent, -0.0001 mag",
      errRatio: "0.984 (scatter 0.005, N = 21)",
      zeroPoint: "22.500 AB (scatter 0.001 mag, N = 21)",
    });
  });

  it("a report without a calibrated ratio says so", () => {
    const uncalibrated = compareHeadline({
      ...REPORT,
      ratio_median: null,
      ratio_nmad: null,
      ratio_median_err: null,
      n_ratio: 0,
      ratio_offset_mag: null,
    });
    expect(uncalibrated.ratio).toBe("not available");
    expect(uncalibrated.offset).toBeNull();
    expect(uncalibrated.zeroPoint).toBe("22.500 AB (scatter 0.001 mag, N = 21)");
    expect(compareHeadline({ ...REPORT, err_ratio_median: null }).errRatio).toBeNull();
    expect(compareHeadline({ ...REPORT, implied_zero_point_ab: null }).zeroPoint).toBeNull();
    expect(compareHeadline({ ...REPORT, err_ratio_nmad: null }).errRatio).toBe("0.984 (scatter --, N = 21)");
  });

  it("signed numbers never print a negative zero", () => {
    expect(signedFixed(0.0001, 3)).toBe("0.000");
    expect(signedFixed(-0.0000011, 4)).toBe("0.0000");
    expect(signedFixed(-0.36, 3)).toBe("-0.360");
    expect(signedFixed(0.003915702838827801, 4)).toBe("+0.0039");
    const fainter = { ...REPORT, ratio_median: 0.9964, ratio_offset_mag: 0.003915702838827801 };
    expect(compareHeadline(fainter).offset).toBe("-0.360 percent, +0.0039 mag");
    expect(compareHeadline({ ...fainter, ratio_offset_mag: null }).offset).toBe("-0.360 percent");
  });

  it("states the aperture that was measured", () => {
    expect(apertureUsedText(REPORT.aperture)).toBe("r = 7.63 px (2.00 arcsec at 0.2620 arcsec per px), no local sky");
    const sdss = { radius_px: 18.58, radius_arcsec: 7.35768, annulus_px: [28.55, 44.21] as [number, number], pixel_scale_arcsec: 0.396, local_sky: true };
    expect(apertureUsedText(sdss)).toBe("r = 18.58 px (7.36 arcsec at 0.3960 arcsec per px), local sky from 28.55 to 44.21 px");
    expect(apertureUsedText({ ...sdss, annulus_px: null })).toBe("r = 18.58 px (7.36 arcsec at 0.3960 arcsec per px), no local sky");
  });

  it("cells and rejected reasons", () => {
    expect(fluxCellUjy(2.871799e-4, 1.0e-7)).toBe(formatWithUncertainty(287.1799, 0.1));
    expect(fluxCellUjy(null, null)).toBe("--");
    expect(fluxCellUjy(1e-4, null)).toBe(formatWithUncertainty(100, null));
    expect(ratioCell(1.00007)).toBe("1.0001");
    expect(ratioCell(null)).toBe("--");
    expect(rejectedText({ reason: "cut type == PSF", count: 33 })).toBe("33 x cut type == PSF");
  });

  it("orders the used rows first and keeps the table order inside each group", () => {
    const before = ROWS.map((r) => r.row);
    expect(orderedRows(ROWS).map((r) => r.row)).toEqual([2, 4, 1, 3]);
    expect(ROWS.map((r) => r.row)).toEqual(before);
    expect(orderedRows([])).toEqual([]);
  });
});

describe("catalogue comparison CSV and marks", () => {
  it("the CSV has one line per row, fluxes in uJy and empty cells for what was not measured", () => {
    expect(compareRowsCsv(ROWS).split("\r\n")).toEqual([
      "row,id,ra_deg,dec_deg,x,y,class,flux_cat_ujy,flux_err_cat_ujy,snr_cat,flux_native,flux_err_native,flux_ujy,flux_err_ujy,ratio,masked_fraction,used,reason",
      "1,10995300315432397,53.2001,-27.9002,,,REX,100,,,,,,,,,false,cut type == PSF",
      "2,10995300315432398,53.2101,-27.9102,3333.8115,625.4192,PSF,287.1799,0.1,2871.799,79.1,0.03,287.2,0.11,1.00007,0,true,",
      "3,row 3,53.2201,-27.9202,10.5,20.25,,50,0.2,250,13.5,0.05,49,0.18,0.98,0.25,false,aperture touches pixels without data",
      '4,"a,b",53.2301,-27.9302,100,200,PSF,100,0.1,1000,27.5,0.03,99.9,0.11,0.999,0,true,',
      "",
    ]);
    expect(compareRowsCsv([])).toBe(
      "row,id,ra_deg,dec_deg,x,y,class,flux_cat_ujy,flux_err_cat_ujy,snr_cat,flux_native,flux_err_native,flux_ujy,flux_err_ujy,ratio,masked_fraction,used,reason\r\n",
    );
  });

  it("heads the CSV with what was compared and how", () => {
    const entries: ProvenanceEntry[] = [
      ...exportEntries("legacysurvey-0532m280-image-r.fits.fz", "2026-11-12T12:00:00.000Z"),
      ["catalogue_table", "C1b_ls_dr10_brick_0532m280.csv"],
      ["catalogue_format", "csv"],
      ["catalogue_rows", 69],
      ["preset", "legacy_dr10"],
      ["catalogue_band", "r"],
      ["image_filter", "DECAM r"],
      ["image_unit", "nanomaggy"],
      ["calibration", "BUNIT nanomaggy, zero point 22.5"],
      ["jy_per_image_unit", 3.6307805477010175e-6],
      ["error_layer", "inverse variance from legacysurvey-0532m280-invvar-r.fits.fz (sigma)"],
      ["mask", "none"],
      ["flagged_pixels", "kept"],
      ["position_columns", "ra, dec"],
      ["id_column", "ls_id"],
      ["flux_column", "apflux_r_5"],
      ["flux_unit", "nanomaggy"],
      ["zero_point", null],
      ["error_column", "apflux_ivar_r_5 (inverse variance)"],
      ["class_column", "type"],
      ["missing_value", null],
      ["aperture", "r = 7.63 px (2.00 arcsec at 0.2620 arcsec per px), no local sky"],
      ["cuts", "type == PSF; maskbits == 0; fracflux_r < 0.01"],
      ["min_snr", null],
      ["max_snr", null],
      ["min_neighbour_arcsec", null],
      ["n_table", 69],
      ["n_on_image", 69],
      ["n_after_cuts", 21],
      ["n_measured", 21],
      ["n_used", 21],
      ["ratio_median", 1.00007],
      ["ratio_median_err", 0.00012],
      ["ratio_nmad", 0.00021],
      ["n_ratio", 21],
      ["ratio_offset_mag", -0.000076],
      ["err_ratio_median", 0.98434],
      ["err_ratio_nmad", 0.0054],
      ["n_err", 21],
      ["implied_zero_point_ab", 22.50008],
      ["implied_zero_point_nmad", 0.0012],
      ["n_zero_point", 21],
      ["rejected", "33 x cut type == PSF; 14 x cut maskbits == 0; 1 x cut fracflux_r < 0.01"],
      ["note", "note one"],
      ["note", "note two"],
      ["warning", "w1"],
    ];
    const units = {
      ra_deg: "deg (ICRS)",
      dec_deg: "deg (ICRS)",
      x: "px (0-based)",
      y: "px (0-based)",
      flux_cat_ujy: "uJy",
      flux_err_cat_ujy: "uJy",
      snr_cat: "catalogue flux over catalogue error",
      flux_native: "nanomaggy",
      flux_err_native: "nanomaggy",
      flux_ujy: "uJy",
      flux_err_ujy: "uJy",
      ratio: "flux_ujy / flux_cat_ujy",
      masked_fraction: "fraction of the aperture area without data",
    };
    const lines = compareCsvHeaderLines(REPORT, CONTEXT);
    expect(lines).toEqual([...provenanceLines(entries), ...unitLines(units)]);
    expect(lines[0].startsWith("# software: AstroBurst ")).toBe(true);
    expect(lines[2]).toBe("# file: legacysurvey-0532m280-image-r.fits.fz");
    const surfaceBrightness = compareCsvHeaderLines({ ...REPORT, band: { ...BAND, bunit: "MJy/sr" } }, CONTEXT);
    expect(surfaceBrightness).toContain("# unit flux_native: MJy/sr x pix");
    expect(surfaceBrightness).toContain("# unit flux_err_native: MJy/sr x pix");
  });

  it("says when the preset was edited, when nothing was cut and when the image has no unit or layers", () => {
    const lineOf = (lines: string[], key: string) => lines.find((line) => line.startsWith(`# ${key}:`));
    const expected = (key: string, value: string | null) => provenanceLines([[key, value]])[0];
    expect(lineOf(compareCsvHeaderLines(REPORT, { ...CONTEXT, presetEdited: true }), "preset")).toBe(
      expected("preset", "legacy_dr10 (edited)"),
    );
    expect(lineOf(compareCsvHeaderLines({ ...REPORT, preset: null }, CONTEXT), "preset")).toBe(expected("preset", "none"));
    const uncut = { ...CONTEXT, request: { ...REQUEST, cuts: [] } };
    expect(lineOf(compareCsvHeaderLines(REPORT, uncut), "cuts")).toBe(expected("cuts", "none"));
    const masked = { ...CONTEXT, request: { ...REQUEST, exclude_dq: true } };
    expect(lineOf(compareCsvHeaderLines(REPORT, masked), "flagged_pixels")).toBe(expected("flagged_pixels", "excluded"));
    const bare = compareCsvHeaderLines(
      { ...REPORT, band: { ...BAND, bunit: null, err_origin: null, filter: null, dq_source: "DQ layer of the file" } },
      CONTEXT,
    );
    expect(lineOf(bare, "image_unit")).toBe(expected("image_unit", null));
    expect(lineOf(bare, "error_layer")).toBe(expected("error_layer", "none"));
    expect(lineOf(bare, "image_filter")).toBe(expected("image_filter", null));
    expect(lineOf(bare, "mask")).toBe(expected("mask", "DQ layer of the file"));
    expect(bare.some((line) => line.startsWith("# unit flux_native:"))).toBe(false);
    expect(bare.some((line) => line.startsWith("# unit flux_err_native:"))).toBe(false);
    const assumed = compareCsvHeaderLines({ ...REPORT, band: { ...BAND, err_origin: { ...BAND.err_origin!, assumed: true } } }, CONTEXT);
    expect(lineOf(assumed, "error_layer")?.endsWith("(sigma, assumed)")).toBe(true);
    const withError = { ...CONTEXT, request: { ...REQUEST, mapping: { ...REQUEST.mapping, flux_err: "e", flux_ivar: null } } };
    expect(lineOf(compareCsvHeaderLines(REPORT, withError), "error_column")).toBe(expected("error_column", "e (error)"));
    expect(lineOf(compareCsvHeaderLines({ ...REPORT, rejected: [] }, CONTEXT), "rejected")).toBe(expected("rejected", "none"));
  });

  it("every header line starts with # and holds no line break", () => {
    const lines = compareCsvHeaderLines({ ...REPORT, notes: ["a\nb"], calibration_origin: "x\r\ny" }, CONTEXT);
    for (const line of lines) expect(line).toMatch(/^#[^\r\n]*$/);
  });

  it("the text is the header lines and then the table, each line ended by CRLF", () => {
    expect(compareCsvText(REPORT, ["# a: 1", "# unit x: px"])).toBe("# a: 1\r\n# unit x: px\r\n" + compareRowsCsv(ROWS));
    expect(compareCsvText(REPORT, [])).toBe(compareRowsCsv(ROWS));
  });

  it("names the file after the image", () => {
    expect(compareCsvFileName(IMG)).toBe("legacysurvey-0532m280-image-r_catalog_compare.csv");
    expect(compareCsvFileName("C:/d/frame-r-001462-4-0165.fits.bz2")).toBe("frame-r-001462-4-0165_catalog_compare.csv");
    expect(compareCsvFileName(null)).toBe("image_catalog_compare.csv");
  });

  it("marks the measured rows and tells the used ones apart", () => {
    const { placed, usedIds } = compareMarkRows(ROWS);
    expect(placed.map((p) => p.id)).toEqual(["10995300315432398", "row 3", "a,b"]);
    expect(placed[0]).toMatchObject({ x: 3333.8115, y: 625.4192, ra: 53.2101, dec: -27.9102, g: null, on_image: true });
    expect([...usedIds]).toEqual(["10995300315432398", "a,b"]);
    expect(compareMarkRows([{ ...ROWS[1], x: Number.NaN }]).placed).toEqual([]);
  });
});
