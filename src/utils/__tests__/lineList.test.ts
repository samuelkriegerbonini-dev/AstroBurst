import { describe, it, expect } from "vitest";
import {
  BAND_SPECIES,
  DEFAULT_BAND_SPECIES,
  LINE_FAMILIES,
  LINE_PICK_TOLERANCE_PX,
  MAX_REDSHIFT,
  REST_BANDS,
  REST_LINES,
  bandById,
  bandMarks,
  conventionForKind,
  displayConvention,
  familyLabel,
  formatRedshift,
  formatSystemicKms,
  lineById,
  lineClickHint,
  lineClickSetsRest,
  lineDisplayValue,
  lineListAvailability,
  lineMarks,
  nearestBandMark,
  nearestLineMark,
  observedVacuumUm,
  parseRedshiftInput,
  parseSystemicInput,
  pickedBandLabel,
  pickedLineLabel,
  placeLineMarks,
  redshiftFrameLabel,
  redshiftFromVelocityKms,
  resyncRedshiftText,
  resyncSystemicText,
  storedVacuumUm,
  toggleBandSpecies,
  velocityKmsFromRedshift,
  type LineFamily,
  type LineMark,
  type LineMarksInput,
  type RestBandKind,
} from "../lineList";
import {
  SPEED_OF_LIGHT_KMS,
  formatAxis,
  frequencyGhzFromWavelengthUm,
  vacuumToAirUm,
  velocityKms,
} from "../spectralAxis";
import { axisValueToPixel, nearestChannel, type PlotMapping } from "../spectrumRange";
import type { SpectralAxisInfo, SpectralAxisKind, SpectralAxisMode, VelocityConvention } from "../../shared/types/spectral";

const HA = 0.656461;
const CO21_UM = 1300.403655796;
const c = SPEED_OF_LIGHT_KMS;
const CONVENTIONS: readonly VelocityConvention[] = ["optical", "radio", "relativistic"];
const MODES: readonly SpectralAxisMode[] = ["wavelength_vac", "wavelength_air", "frequency", "velocity"];

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

function mapping(xMin: number, xMax: number, xValues: number[] | null, n: number): PlotMapping {
  return { width: 400, padLeft: 50, padRight: 12, xMin, xMax, xValues, n };
}

function expectNear(actual: number | null, expected: number, tolerance: number) {
  expect(actual).not.toBeNull();
  expect(Math.abs((actual as number) - expected)).toBeLessThanOrEqual(tolerance);
}

function linearAxis(start: number, end: number, count: number): number[] {
  return Array.from({ length: count }, (_, i) => start + ((end - start) * i) / (count - 1));
}

function optical(id: string): number {
  return lineById(id)?.vacuumUm ?? NaN;
}

describe("rest-frame line table", () => {
  it("the table holds the 47 lines of the brief once each with the family of their storage unit", () => {
    expect(REST_LINES).toHaveLength(47);
    expect(new Set(REST_LINES.map((l) => l.id)).size).toBe(47);
    expect(LINE_FAMILIES).toEqual(["optical", "nir", "mir", "radio"]);
    const count: Record<LineFamily, number> = { optical: 0, nir: 0, mir: 0, radio: 0 };
    for (const line of REST_LINES) {
      count[line.family]++;
      expect(line.vacuumUm).toBeGreaterThan(0);
      if (line.family === "radio") expect(line.restGhz).toBeGreaterThan(0);
      else expect(line.restGhz).toBeNull();
    }
    expect(count).toEqual({ optical: 14, nir: 10, mir: 16, radio: 7 });
  });

  it("optical lines match NIST ASD vacuum wavelengths", () => {
    const nist: Record<string, number> = {
      h_alpha: 0.656461,
      h_beta: 0.486268,
      oiii_4959: 0.49603,
      oiii_5007: 0.500824,
      nii_6548: 0.654986,
      nii_6583: 0.658527,
      sii_6716: 0.671829,
      sii_6731: 0.673267,
      oii_3727: 0.372709,
      oii_3729: 0.372988,
    };
    for (const [id, um] of Object.entries(nist)) {
      const line = lineById(id);
      expect(line?.family).toBe("optical");
      expectNear(line?.vacuumUm ?? null, um, 2e-6);
    }
    const alkali: Record<string, [string, number]> = {
      nai_5890: ["Na I 5890", 0.5891583264],
      nai_5896: ["Na I 5896", 0.5897558147],
      ki_7665: ["K I 7665", 0.7667008906],
      ki_7699: ["K I 7699", 0.7701083536],
    };
    for (const [id, [label, um]] of Object.entries(alkali)) {
      const line = lineById(id);
      expect(line?.label).toBe(label);
      expect(line?.family).toBe("optical");
      expect(line?.restGhz).toBeNull();
      expectNear(line?.vacuumUm ?? null, um, 1e-7);
    }
  });

  it("near-infrared lines match NIST ASD vacuum wavelengths and the H2 1-0 S(1) literature value", () => {
    const values: Record<string, number> = {
      pa_alpha: 1.875613,
      pa_beta: 1.282159,
      br_gamma: 2.16612,
      hei_1083: 1.083331,
      h2_2122: 2.121834,
      feii_1644: 1.644,
      br_alpha: 4.052262,
      br_beta: 2.625872,
      pf_beta: 4.653778,
      hei_2058: 2.05869,
    };
    for (const [id, um] of Object.entries(values)) {
      const line = lineById(id);
      expect(line?.family).toBe("nir");
      expectNear(line?.vacuumUm ?? null, um, 1e-5);
    }
  });

  it("Brα, Brβ and Pfβ follow the Rydberg values of Storey & Hummer (1995) / NIST ASD and He I 2.058 the NIST ASD value (Drake 2006), in vacuum", () => {
    const expected: Record<string, [string, number]> = {
      br_alpha: ["Brα", 4.052262],
      br_beta: ["Brβ", 2.625872],
      pf_beta: ["Pfβ", 4.653778],
      hei_2058: ["He I 2.058", 2.05869],
    };
    for (const [id, [label, um]] of Object.entries(expected)) {
      const line = lineById(id);
      expect(line?.vacuumUm).toBe(um);
      expect(line?.label).toBe(label);
      expect(line?.family).toBe("nir");
      expect(line?.restGhz).toBeNull();
    }
    const families = REST_LINES.map((l) => l.family);
    const firstIndex = (f: LineFamily) => families.indexOf(f);
    const lastIndex = (f: LineFamily) => families.lastIndexOf(f);
    expect(lastIndex("optical")).toBeLessThan(firstIndex("nir"));
    expect(lastIndex("nir")).toBeLessThan(firstIndex("mir"));
    expect(lastIndex("mir")).toBeLessThan(firstIndex("radio"));
    expect(new Set(REST_LINES.map((l) => l.id)).size).toBe(REST_LINES.length);
    expect(new Set(REST_LINES.map((l) => l.label)).size).toBe(REST_LINES.length);
  });

  it("mid-infrared lines match the ISO (Lutz) fine-structure list and Black's H2 rotational list in vacuum", () => {
    const iso: Record<string, [string, number]> = {
      arii_6985: ["[ArII] 6.985", 6.985274],
      ariii_8991: ["[ArIII] 8.991", 8.99138],
      h2_s3_9665: ["H2 0-0 S(3)", 9.66492],
      siv_10511: ["[SIV] 10.511", 10.5105],
      h2_s2_12279: ["H2 0-0 S(2)", 12.27861],
      neii_12814: ["[NeII] 12.814", 12.81355],
      nev_14322: ["[NeV] 14.322", 14.3217],
      neiii_15555: ["[NeIII] 15.555", 15.5551],
      h2_s1_17035: ["H2 0-0 S(1)", 17.03484],
      siii_18713: ["[SIII] 18.713", 18.713],
      nev_24318: ["[NeV] 24.318", 24.3175],
      oiv_25890: ["[OIV] 25.890", 25.8903],
      feii_25988: ["[FeII] 25.988", 25.98829],
      h2_s0_28219: ["H2 0-0 S(0)", 28.21883],
      siii_33481: ["[SIII] 33.481", 33.481],
      si_ii_34815: ["[SiII] 34.815", 34.8152],
    };
    for (const [id, [label, um]] of Object.entries(iso)) {
      const line = lineById(id);
      expect(line?.label).toBe(label);
      expect(line?.family).toBe("mir");
      expect(line?.restGhz).toBeNull();
      expectNear(line?.vacuumUm ?? null, um, 1e-6);
    }
    const axis = axisOf("wave", linearAxis(5, 28, 231));
    const base = { axis, mode: "wavelength_vac" as const, restUm: null, convention: "optical" as const, shiftKms: null, redshift: 0, xMin: 5, xMax: 28 };
    const marks = lineMarks({ ...base, families: ["mir"] });
    expect(marks.map((m) => m.id)).toContain("neii_12814");
    expect(marks.every((m) => m.family === "mir")).toBe(true);
    expect(lineMarks({ ...base, families: ["nir", "radio"] })).toEqual([]);
  });

  it("radio lines match Splatalogue rest frequencies", () => {
    const ghz: Record<string, number> = {
      co_1_0: 115.2712018,
      co_2_1: 230.538,
      co_3_2: 345.7959899,
      hcn_1_0: 88.6316023,
      hcop_1_0: 89.1885247,
      cii_158: 1900.5369,
      hi_21cm: 1.420405751768,
    };
    for (const [id, f] of Object.entries(ghz)) {
      const line = lineById(id);
      expect(line?.family).toBe("radio");
      expectNear(line?.restGhz ?? null, f, 1e-5);
    }
  });

  it("radio lines carry the vacuum wavelength of their rest frequency, CO 2-1 giving the repository golden 1300.403655796 um", () => {
    expectNear(lineById("co_2_1")?.vacuumUm ?? null, CO21_UM, 1e-6);
    expectNear(lineById("cii_158")?.vacuumUm ?? null, 157.7409, 1e-3);
    expectNear(lineById("hi_21cm")?.vacuumUm ?? null, 211061.14, 1e-2);
  });
});

describe("redshift and systemic velocity", () => {
  it("conventionForKind mirrors the Rust VelocityConvention parser", () => {
    expect(conventionForKind("vrad")).toBe("radio");
    expect(conventionForKind("vopt")).toBe("optical");
    expect(conventionForKind("velo")).toBe("relativistic");
    for (const kind of ["wave", "awav", "freq", "zopt", "unknown"] as const) expect(conventionForKind(kind)).toBeNull();
    expect(displayConvention(axisOf("vrad", [-100, -98]), "optical")).toBe("radio");
    expect(displayConvention(axisOf("wave", [0.6, 0.7]), "radio")).toBe("radio");
    expect(displayConvention(axisOf("zopt", [0, 0.001]), "relativistic")).toBe("relativistic");
    expect(displayConvention(null, "radio")).toBe("radio");
  });

  it("redshift and systemic velocity invert each other in every convention", () => {
    for (const z of [0, 0.001, 0.1, 2]) {
      for (const conv of CONVENTIONS) {
        expectNear(redshiftFromVelocityKms(velocityKmsFromRedshift(z, conv), conv), z, 1e-12);
      }
    }
    expectNear(velocityKmsFromRedshift(0.001, "optical"), 299.7925, 1e-3);
    expectNear(velocityKmsFromRedshift(0.001, "radio"), 299.493, 1e-3);
    expectNear(velocityKmsFromRedshift(0.001, "relativistic"), 299.6426, 1e-3);
  });

  it("parseRedshiftInput accepts finite z above -1 up to MAX_REDSHIFT and rejects the rest", () => {
    expect(MAX_REDSHIFT).toBe(100);
    expect(parseRedshiftInput("0")).toBe(0);
    expect(parseRedshiftInput("0.5")).toBe(0.5);
    expect(parseRedshiftInput("-0.5")).toBe(-0.5);
    expect(parseRedshiftInput("100")).toBe(100);
    for (const text of ["", " ", "abc", "-1", "-2", "100.5", "Infinity"]) expect(parseRedshiftInput(text)).toBeNull();
  });

  it("parseSystemicInput derives z by convention and rejects superluminal radio and relativistic velocities", () => {
    expectNear(parseSystemicInput("1000", "optical"), 0.003335641, 1e-9);
    expectNear(parseSystemicInput("1000", "radio"), 0.003346805, 1e-9);
    expectNear(parseSystemicInput("1000", "relativistic"), 0.003341223, 1e-9);
    expectNear(parseSystemicInput("300000", "optical"), 1.00069, 1e-5);
    expect(parseSystemicInput("300000", "radio")).toBeNull();
    expect(parseSystemicInput("300000", "relativistic")).toBeNull();
    expect(parseSystemicInput("299792.458", "radio")).toBeNull();
    expect(parseSystemicInput("", "optical")).toBeNull();
    expect(parseSystemicInput("abc", "optical")).toBeNull();
  });

  it("formatRedshift and formatSystemicKms round for display", () => {
    expect(formatRedshift(0.001234567)).toBe("0.001235");
    expect(formatRedshift(2)).toBe("2");
    expect(formatRedshift(0)).toBe("0");
    expect(formatSystemicKms(299.792458)).toBe("299.8");
  });
});

describe("line placement", () => {
  it("observedVacuumUm applies the redshift and storedVacuumUm removes the frame shift", () => {
    expectNear(observedVacuumUm(HA, 0.01), 0.66302561, 1e-8);
    expectNear(storedVacuumUm(0.66302561, 30), 0.66295927, 1e-8);
    expect(storedVacuumUm(0.66302561, null)).toBe(0.66302561);
    expectNear(storedVacuumUm(HA, 30), 0.65639532, 1e-8);
  });

  it("on a wavelength axis the mark follows the display mode applied to the stored wavelength", () => {
    const axis = axisOf("wave", linearAxis(0.6, 0.7, 101));
    const obs = 0.66302561;
    const st = storedVacuumUm(obs, 30);
    expectNear(st, 0.66295927, 1e-8);
    expectNear(lineDisplayValue(obs, axis, "wavelength_vac", null, "optical", 30), st, 1e-9);
    const air = lineDisplayValue(obs, axis, "wavelength_air", null, "optical", 30);
    expectNear(air, vacuumToAirUm(st), 1e-9);
    expect(air as number).toBeLessThan(st);
    const freq = lineDisplayValue(obs, axis, "frequency", null, "optical", 30);
    expectNear(freq, frequencyGhzFromWavelengthUm(st), 1e-9);
    expectNear(freq, 452203.4345, 1e-3);
    const velo = lineDisplayValue(obs, axis, "velocity", HA, "optical", 30);
    expectNear(velo, velocityKms(st, HA, "optical") + 30, 1e-9);
    expectNear(velo, 2997.6276, 1e-3);
    const vsys = velocityKmsFromRedshift(0.01, "optical");
    expectNear(vsys, 2997.9246, 1e-3);
    const beta = 30 / c;
    expectNear((velo as number) - vsys, (30 * (beta - 0.01)) / (1 + beta), 1e-6);
    expect(lineDisplayValue(obs, axis, "velocity", null, "optical", 30)).toBeNull();
    expect(lineDisplayValue(obs, axis, "velocity", 0, "optical", 30)).toBeNull();
  });

  it("a line at z = 0 reads 0.0 km/s on the corrected velocity axis and moves in wavelength by the correction", () => {
    const axis = axisOf("wave", linearAxis(0.6, 0.7, 101));
    const velo = lineDisplayValue(HA, axis, "velocity", HA, "optical", 30);
    expectNear(velo, (30 * 30) / (c * (1 + 30 / c)), 1e-6);
    expect((velo as number).toFixed(1)).toBe("0.0");
    const vac = lineDisplayValue(HA, axis, "wavelength_vac", HA, "optical", 30);
    expectNear(vac, HA / (1 + 30 / c), 1e-9);
    expectNear(vac, 0.65639532, 1e-8);
    expect(vac as number).toBeLessThan(HA);
  });

  it("on a VRAD axis the mark uses the header rest and the radio convention whatever the panel rest and convention", () => {
    const obs = CO21_UM * 1.01;
    const vrad = axisOf("vrad", [-100, -98], { rest_wavelength_um: CO21_UM, specsys: "LSRK" });
    expectNear(lineDisplayValue(obs, vrad, "wavelength_vac", 0.5, "optical", null), 2968.2422, 1e-3);
    const vopt = axisOf("vopt", [-100, -98], { rest_wavelength_um: CO21_UM });
    expectNear(lineDisplayValue(obs, vopt, "wavelength_vac", 0.5, "radio", null), 2997.9246, 1e-3);
    const velo = axisOf("velo", [-100, -98], { rest_wavelength_um: CO21_UM });
    expectNear(lineDisplayValue(obs, velo, "wavelength_vac", 0.5, "optical", null), 2982.9357, 1e-3);
    expectNear(
      lineDisplayValue(obs, vrad, "velocity", 0.5, "optical", 30),
      velocityKms(storedVacuumUm(obs, 30), CO21_UM, "radio") + 30,
      1e-9,
    );
  });

  it("on a velocity or ZOPT axis without a header rest the mark is null and the availability names RESTFRQ", () => {
    const vrad = axisOf("vrad", [-100, -98]);
    const zopt = axisOf("zopt", [0, 0.001]);
    expect(lineDisplayValue(HA, vrad, "velocity", HA, "optical", null)).toBeNull();
    expect(lineDisplayValue(HA, zopt, "velocity", HA, "optical", null)).toBeNull();
    for (const axis of [vrad, zopt]) {
      const availability = lineListAvailability(axis, "velocity", null, formatAxis(axis, "velocity", null, "optical"), 2);
      expect(availability.ok).toBe(false);
      if (!availability.ok) expect(availability.reason).toContain("RESTFRQ/RESTWAV");
    }
  });

  it("on a ZOPT axis the mark uses the header rest and the panel convention", () => {
    const zopt = axisOf("zopt", [0, 0.001], { rest_wavelength_um: HA });
    const obs = HA * 1.01;
    expectNear(lineDisplayValue(obs, zopt, "velocity", null, "optical", null), 2997.9246, 1e-3);
    expectNear(lineDisplayValue(obs, zopt, "velocity", null, "radio", null), 2968.2422, 1e-3);
    expectNear(
      lineDisplayValue(obs, zopt, "velocity", null, "optical", 30),
      velocityKms(storedVacuumUm(obs, 30) / HA, 1, "optical") + 30,
      1e-9,
    );
  });

  it("the mark lands on the same channel in every display mode with the correction applied, at z 0.01 and at z 5", () => {
    const channelFor = (axis: SpectralAxisInfo, mode: SpectralAxisMode, obs: number): number | null => {
      const f = formatAxis(axis, mode, HA, "optical");
      if (!f) return null;
      const values = mode === "velocity" ? f.values.map((v) => v + 30) : f.values;
      const m = mapping(Math.min(...values), Math.max(...values), values, values.length);
      const value = lineDisplayValue(obs, axis, mode, HA, "optical", 30);
      return value === null ? null : nearestChannel(value, m);
    };
    const axisA = axisOf("wave", linearAxis(0.6, 0.7, 101));
    for (const mode of MODES) expect(channelFor(axisA, mode, observedVacuumUm(HA, 0.01))).toBe(63);

    const axisB = axisOf("wave", linearAxis(3.9, 4.0, 1001));
    const obsB = observedVacuumUm(HA, 5);
    expectNear(obsB, 3.938766, 1e-6);
    expectNear(storedVacuumUm(obsB, 30), 3.938372, 1e-6);
    for (const mode of MODES) expect(channelFor(axisB, mode, obsB)).toBe(384);
    const velocityValues = formatAxis(axisB, "velocity", HA, "optical")!.values.map((v) => v + 30);
    const velocityMapping = mapping(Math.min(...velocityValues), Math.max(...velocityValues), velocityValues, velocityValues.length);
    expect(nearestChannel(velocityKms(obsB, HA, "optical"), velocityMapping)).toBe(387);
  });
});

describe("availability, filtering and label placement", () => {
  it("lineListAvailability names the missing axis, the unknown kind, the missing rest in velocity mode and the channel fallback", () => {
    const none = lineListAvailability(null, "wavelength_vac", null, null, 5);
    expect(none.ok).toBe(false);
    if (!none.ok) expect(none.reason).toContain("no spectral axis");
    const unknown = lineListAvailability(axisOf("unknown", [0, 1]), "wavelength_vac", null, null, 2);
    expect(unknown.ok).toBe(false);
    if (!unknown.ok) expect(unknown.reason).toContain("not a spectral axis");
    const wave = axisOf("wave", linearAxis(0.6, 0.7, 5));
    const noRest = lineListAvailability(wave, "velocity", null, null, 5);
    expect(noRest.ok).toBe(false);
    if (!noRest.ok) expect(noRest.reason).toContain("rest wavelength");
    const short = lineListAvailability(wave, "wavelength_vac", null, { values: [0.6, 0.65, 0.7], label: "", unit: "μm", notes: [] }, 5);
    expect(short.ok).toBe(false);
    if (!short.ok) expect(short.reason).toContain("channel");
    expect(lineListAvailability(wave, "wavelength_vac", null, formatAxis(wave, "wavelength_vac", null, "optical"), 5)).toEqual({ ok: true });
  });

  it("lineMarks keeps only the enabled families inside the visible range in table order and carries the rest wavelength beside the display value", () => {
    const axis = axisOf("wave", linearAxis(0.6, 0.7, 101));
    const base = { axis, mode: "wavelength_vac" as const, restUm: null, convention: "optical" as const, shiftKms: null };
    const all = lineMarks({ ...base, redshift: 0, families: LINE_FAMILIES, xMin: 0.64, xMax: 0.68 });
    expect(all.map((m) => m.id)).toEqual(["h_alpha", "nii_6548", "nii_6583", "sii_6716", "sii_6731"]);
    for (const mark of all) {
      expect(mark.value).toBe(mark.restVacuumUm);
      expect(mark.restVacuumUm).toBe(optical(mark.id));
      expect(mark.label).toBe(lineById(mark.id)?.label);
      expect(mark.family).toBe("optical");
    }
    expect(lineMarks({ ...base, redshift: 0, families: ["nir"], xMin: 0.64, xMax: 0.68 })).toEqual([]);
    expect(lineMarks({ ...base, redshift: 0, families: ["optical"], xMin: 0.49, xMax: 0.51 }).map((m) => m.id)).toEqual(["oiii_4959", "oiii_5007"]);
    expect(lineMarks({ ...base, redshift: 0, families: [], xMin: 0.64, xMax: 0.68 })).toEqual([]);
    const shifted = lineMarks({ ...base, redshift: 0.01, families: LINE_FAMILIES, xMin: 0.64, xMax: 0.67 });
    expect(shifted.map((m) => m.id)).toEqual(["h_alpha", "nii_6548", "nii_6583"]);
    expect(shifted[0].restVacuumUm).toBe(0.656461);
    expectNear(shifted[0].value, 0.66302561, 1e-8);
  });

  it("placeLineMarks converts marks to pixels, clamps labels to the right edge and drops overlapping labels while keeping every line", () => {
    const axis = axisOf("wave", linearAxis(0.6, 0.7, 101));
    const marks = lineMarks({ axis, mode: "wavelength_vac", restUm: null, convention: "optical", shiftKms: null, redshift: 0, families: LINE_FAMILIES, xMin: 0.64, xMax: 0.68 });
    const m = mapping(0.64, 0.68, null, 40);
    const placed = placeLineMarks(marks, m);
    expect(placed.map((p) => p.id)).toEqual(["nii_6548", "h_alpha", "nii_6583", "sii_6716", "sii_6731"]);
    const px = [176.63, 189.1, 206.55, 318.96, 331.11];
    const labelX = [179.63, 192.1, 209.55, 321.96, 333.0];
    placed.forEach((p, i) => {
      expectNear(p.px, px[i], 1e-2);
      expectNear(p.labelX, labelX[i], 1e-2);
      expect(p.labelWidth).toBe(p.id === "h_alpha" ? 11 : 55);
    });
    expect(placed.map((p) => p.labelShown)).toEqual([true, false, false, true, false]);
    expect(placed).toHaveLength(marks.length);
    const nan: LineMark = { id: "nan", label: "x", family: "optical", restVacuumUm: 1, value: NaN };
    expect(placeLineMarks([...marks, nan], m).some((p) => p.id === "nan")).toBe(false);

    const edge: LineMark[] = [
      { id: "a", label: "0123456789", family: "optical", restVacuumUm: 0.66959, value: 0.66959 },
      { id: "b", label: "0123456789", family: "optical", restVacuumUm: 0.67965, value: 0.67965 },
    ];
    const placedEdge = placeLineMarks(edge, m);
    expectNear(placedEdge[0].px, 300.04, 1e-2);
    expectNear(placedEdge[1].px, 385.04, 1e-2);
    expectNear(placedEdge[0].labelX, 303.04, 1e-2);
    expectNear(placedEdge[1].labelX, 333.0, 1e-2);
    expect(placedEdge.map((p) => p.labelShown)).toEqual([true, false]);
  });
});

describe("click hit test and hints", () => {
  it("nearestLineMark picks the closest mark within the tolerance and nothing beyond it", () => {
    const axis = axisOf("wave", linearAxis(0.6, 0.7, 101));
    const base = { axis, mode: "wavelength_vac" as const, restUm: null, convention: "optical" as const, shiftKms: null, families: LINE_FAMILIES };
    const marks = lineMarks({ ...base, redshift: 0, xMin: 0.64, xMax: 0.68 });
    const m = mapping(0.64, 0.68, null, 40);
    expect(LINE_PICK_TOLERANCE_PX).toBe(6);
    expect(nearestLineMark(marks, m, 192.1, LINE_PICK_TOLERANCE_PX)?.id).toBe("h_alpha");
    expect(nearestLineMark(marks, m, 200, 6)).toBeNull();
    expect(nearestLineMark(marks, m, 200, 7)?.id).toBe("nii_6583");
    expect(nearestLineMark(marks, m, 300, LINE_PICK_TOLERANCE_PX)).toBeNull();
    const duplicated = [...marks, { ...marks[0] }];
    expect(nearestLineMark(duplicated, m, 189.1, LINE_PICK_TOLERANCE_PX)).toBe(marks[0]);
    expect(nearestLineMark([], m, 189.1, LINE_PICK_TOLERANCE_PX)).toBeNull();
    const shifted = lineMarks({ ...base, redshift: 0.01, xMin: 0.64, xMax: 0.67 });
    const hit = nearestLineMark(shifted, mapping(0.64, 0.67, null, 40), 309.4, LINE_PICK_TOLERANCE_PX);
    expect(hit?.id).toBe("h_alpha");
    expect(hit?.restVacuumUm).toBe(0.656461);
    expectNear(hit?.value ?? null, 0.66302561, 1e-8);
  });

  it("lineClickSetsRest is true only on wavelength and frequency axes", () => {
    for (const kind of ["wave", "awav", "freq"] as const) expect(lineClickSetsRest(axisOf(kind, [1, 2]))).toBe(true);
    for (const kind of ["vrad", "vopt", "velo", "zopt", "unknown"] as const) expect(lineClickSetsRest(axisOf(kind, [1, 2]))).toBe(false);
    expect(lineClickSetsRest(null)).toBe(false);
    expect(lineClickHint(axisOf("wave", [1, 2]))).toContain("set the rest wavelength");
    expect(lineClickHint(axisOf("vrad", [1, 2]))).toContain("fixed by the header");
    expect(lineClickHint(axisOf("zopt", [1, 2]))).toContain("fixed by the header");
    expect(lineClickHint(null)).toBe("");
  });

  it("redshiftFrameLabel names the corrected frame only when the correction moves the axis, else the stored frame with SPECSYS", () => {
    expect(redshiftFrameLabel("barycentric", "Meeus low-precision ephemeris", "TOPOCENT")).toBe("z and v_sys in the barycentric frame");
    expect(redshiftFrameLabel("heliocentric", "header VELOSYS", "TOPOCENT")).toBe("z and v_sys in the heliocentric frame");
    expect(redshiftFrameLabel("barycentric", "already in LSRK", "LSRK")).toBe("z and v_sys in the stored frame (SPECSYS LSRK)");
    expect(redshiftFrameLabel("heliocentric", "already in BARYCENT", "BARYCENT")).toBe("z and v_sys in the stored frame (SPECSYS BARYCENT)");
    expect(redshiftFrameLabel("barycentric", null, "LSRK")).toBe("z and v_sys in the stored frame (SPECSYS LSRK)");
    expect(redshiftFrameLabel("none", "Meeus low-precision ephemeris", "TOPOCENT")).toBe("z and v_sys in the stored frame (SPECSYS TOPOCENT)");
    expect(redshiftFrameLabel("none", null, null)).toBe("z and v_sys in the stored frame");
  });

  it("pickedLineLabel and familyLabel and lineById format the table entries", () => {
    expect(pickedLineLabel(lineById("co_2_1")!)).toBe("rest ← CO 2-1 1300.403656 μm vacuum (230.538 GHz)");
    expect(pickedLineLabel(lineById("h_alpha")!)).toBe("rest ← Hα 0.656461 μm vacuum");
    expect(lineById("nope")).toBeNull();
    expect(familyLabel("optical")).toBe("Optical");
    expect(familyLabel("nir")).toBe("NIR");
    expect(familyLabel("mir")).toBe("MIR");
    expect(familyLabel("radio")).toBe("Radio");
  });

  it("resyncSystemicText and resyncRedshiftText keep a text that still shows the value and rewrite it when the convention changes the displayed number", () => {
    expect(resyncSystemicText("269.8", 0.0009, "optical")).toBe("269.8");
    expect(resyncSystemicText("269.8", 0.0009, "radio")).toBe("269.6");
    expect(resyncSystemicText("149.9", 0.0005, "radio")).toBe("149.8");
    expect(resyncSystemicText("1000", 0.003335641, "optical")).toBe("1000");
    expect(resyncSystemicText("abc", 0.01, "optical")).toBe("2997.9");
    expect(resyncSystemicText("300000", 1.0006922856, "radio")).toBe("149948.1");
    expect(resyncRedshiftText("0.0012345678", 0.0012345678)).toBe("0.0012345678");
    expect(resyncRedshiftText("1e-3", 0.001)).toBe("1e-3");
    expect(resyncRedshiftText("0.5", 0.25)).toBe("0.25");
    expect(resyncRedshiftText("", 0)).toBe("0");
  });
});

describe("band table", () => {
  const MOLECULAR_IDS = [
    "h2o_2p7",
    "h2o_6p3",
    "co2_4p3",
    "co_4p7",
    "ch4_3p3",
    "ch4_7p7",
    "so2_7p3",
    "so2_8p7",
    "nh3_3p0",
    "nh3_6p1",
    "nh3_10p5",
    "h2s_3p8",
    "h2s_8p5",
    "hcn_3p0",
    "hcn_4p8",
    "c2h2_3p0",
  ];
  const ALL_SPECIES = ["H2O", "CO2", "CO", "CH4", "SO2", "NH3", "H2S", "HCN", "C2H2", "Na", "K", "He", "H", "G395H gap"];

  it("the band table holds the 21 sourced rows with unique ids that no line uses", () => {
    expect(REST_BANDS).toHaveLength(21);
    expect(REST_BANDS.map((b) => b.id)).toEqual([...MOLECULAR_IDS, "na_d", "k_i_doublet", "he_i_triplet", "h_alpha_band", "nirspec_g395h_gap"]);
    expect(new Set(REST_BANDS.map((b) => b.id)).size).toBe(21);
    expect(REST_BANDS.map((b) => b.label)).toEqual([
      "H2O 2.7",
      "H2O 6.3",
      "CO2 4.3",
      "CO 4.7",
      "CH4 3.3",
      "CH4 7.7",
      "SO2 7.3",
      "SO2 8.7",
      "NH3 3.0",
      "NH3 6.1",
      "NH3 10.5",
      "H2S 3.8",
      "H2S 8.5",
      "HCN 3.0",
      "HCN 4.8",
      "C2H2 3.0",
      "Na I D",
      "K I 0.77",
      "He I 1.083",
      "Hα",
      "G395H gap",
    ]);
    const lineIds = new Set(REST_LINES.map((l) => l.id));
    const count: Record<RestBandKind, number> = { molecular_band: 0, atomic_line: 0, instrument: 0 };
    for (const band of REST_BANDS) {
      expect(lineIds.has(band.id)).toBe(false);
      expect(band.loVacuumUm).toBeLessThanOrEqual(band.centreVacuumUm);
      expect(band.centreVacuumUm).toBeLessThanOrEqual(band.hiVacuumUm);
      expect(band.source.length).toBeGreaterThan(0);
      expect(Object.keys(count)).toContain(band.kind);
      count[band.kind]++;
    }
    expect(count).toEqual({ molecular_band: 16, atomic_line: 4, instrument: 1 });
    expect(BAND_SPECIES).toEqual(ALL_SPECIES);
    expect(DEFAULT_BAND_SPECIES).toEqual(["H2O", "CO2", "CO", "CH4", "SO2"]);
    for (const name of DEFAULT_BAND_SPECIES) expect(BAND_SPECIES).toContain(name);
    expect(bandById("nope")).toBeNull();
  });

  it("molecular band centres are the NIST band origins as 10000 / nu", () => {
    const origins: Record<string, [number, string[]]> = {
      h2o_2p7: [2.662406816, ["3756 cm-1", "3657 cm-1"]],
      h2o_6p3: [6.269592476, ["1595 cm-1"]],
      co2_4p3: [4.257130694, ["2349 cm-1"]],
      co_4p7: [4.666355576, ["2143 cm-1"]],
      ch4_3p3: [3.312355084, ["3019 cm-1"]],
      ch4_7p7: [7.656967841, ["1306 cm-1"]],
      so2_7p3: [7.342143906, ["1362 cm-1"]],
      so2_8p7: [8.688097307, ["1151 cm-1"]],
      nh3_3p0: [2.996703626, ["3337 cm-1", "3444 cm-1"]],
      nh3_6p1: [6.1462815, ["1627 cm-1"]],
      nh3_10p5: [10.72386059, ["932.5 and 968.3 cm-1"]],
      h2s_3p8: [3.824091778, ["2615 cm-1", "2626 cm-1"]],
      h2s_8p5: [8.453085376, ["1183 cm-1"]],
      hcn_3p0: [3.019323671, ["3312 cm-1"]],
      hcn_4p8: [4.786979416, ["2089 cm-1"]],
      c2h2_3p0: [3.040437823, ["3289 cm-1"]],
    };
    expect(Object.keys(origins)).toEqual(MOLECULAR_IDS);
    for (const [id, [um, wavenumbers]] of Object.entries(origins)) {
      const band = bandById(id);
      expect(band?.kind).toBe("molecular_band");
      expect(band?.centreVacuumUm).toBeCloseTo(um, 9);
      for (const text of wavenumbers) expect(band?.source).toContain(text);
      if (id === "co_4p7") expect(band?.source.endsWith("display range from Grant et al., arXiv 2304.11994")).toBe(true);
      else expect(band?.source.endsWith("display range editorial")).toBe(true);
    }
  });

  it("molecular display ranges are the ones of the research table", () => {
    const ranges: Record<string, [number, number]> = {
      h2o_2p7: [2.5, 3.0],
      h2o_6p3: [5.5, 7.5],
      co2_4p3: [4.15, 4.5],
      co_4p7: [4.3, 5.1],
      ch4_3p3: [3.15, 3.5],
      ch4_7p7: [7.3, 8.1],
      so2_7p3: [7.1, 7.6],
      so2_8p7: [8.4, 9.0],
      nh3_3p0: [2.85, 3.05],
      nh3_6p1: [5.9, 6.4],
      nh3_10p5: [10.0, 11.0],
      h2s_3p8: [3.7, 4.0],
      h2s_8p5: [8.0, 9.0],
      hcn_3p0: [2.95, 3.1],
      hcn_4p8: [4.7, 4.9],
      c2h2_3p0: [2.95, 3.1],
    };
    expect(Object.keys(ranges)).toEqual(MOLECULAR_IDS);
    for (const [id, [lo, hi]] of Object.entries(ranges)) {
      const band = bandById(id);
      expect(band?.loVacuumUm).toBe(lo);
      expect(band?.hiVacuumUm).toBe(hi);
    }
  });

  it("atomic rows span their NIST multiplet and agree with the line list", () => {
    const rows: Record<string, [string, number, number, number]> = {
      na_d: ["Na", 0.5891583264, 0.5891583264, 0.5897558147],
      k_i_doublet: ["K", 0.7667008906, 0.7667008906, 0.7701083536],
      he_i_triplet: ["He", 1.0832057482, 1.0833306454, 1.0833306454],
      h_alpha_band: ["H", 0.656452, 0.65646, 0.656463],
    };
    for (const [id, [species, lo, centre, hi]] of Object.entries(rows)) {
      const band = bandById(id);
      expect(band?.kind).toBe("atomic_line");
      expect(band?.species).toBe(species);
      expect(band?.loVacuumUm).toBe(lo);
      expect(band?.centreVacuumUm).toBe(centre);
      expect(band?.hiVacuumUm).toBe(hi);
      expect(band?.source.startsWith("NIST ASD vacuum: ")).toBe(true);
    }
    const apart = (a: number | undefined, b: number) => Math.abs((a ?? NaN) - b);
    expect(apart(bandById("na_d")?.loVacuumUm, optical("nai_5890"))).toBeLessThan(1e-7);
    expect(apart(bandById("na_d")?.hiVacuumUm, optical("nai_5896"))).toBeLessThan(1e-7);
    expect(apart(bandById("k_i_doublet")?.loVacuumUm, optical("ki_7665"))).toBeLessThan(1e-7);
    expect(apart(bandById("k_i_doublet")?.hiVacuumUm, optical("ki_7699"))).toBeLessThan(1e-7);
    expect(apart(bandById("he_i_triplet")?.centreVacuumUm, optical("hei_1083"))).toBeLessThan(1e-6);
    expect(apart(bandById("h_alpha_band")?.centreVacuumUm, optical("h_alpha"))).toBeLessThan(2e-6);
  });

  it("the instrument row is the G395H detector gap", () => {
    const gap = bandById("nirspec_g395h_gap");
    expect(gap?.kind).toBe("instrument");
    expect(gap?.species).toBe("G395H gap");
    expect(gap?.loVacuumUm).toBe(3.72);
    expect(gap?.centreVacuumUm).toBe(3.77);
    expect(gap?.hiVacuumUm).toBe(3.82);
    expect(gap?.source).toContain("not redshifted");
    expect(REST_BANDS.filter((b) => b.kind === "instrument")).toEqual([gap]);
  });
});

describe("band marks", () => {
  const bandBase: LineMarksInput = {
    axis: axisOf("wave", linearAxis(2.4, 5.2, 281)),
    mode: "wavelength_vac",
    restUm: null,
    convention: "optical",
    shiftKms: null,
    redshift: 0,
    families: [],
    xMin: 2.4,
    xMax: 5.2,
  };
  const nirMapping = mapping(2.4, 5.2, linearAxis(2.4, 5.2, 281), 281);

  it("bandMarks lists the bands of the chosen species inside the range in table order", () => {
    const all = bandMarks({ ...bandBase, species: BAND_SPECIES });
    expect(all.map((b) => b.id)).toEqual([
      "h2o_2p7",
      "co2_4p3",
      "co_4p7",
      "ch4_3p3",
      "nh3_3p0",
      "h2s_3p8",
      "hcn_3p0",
      "hcn_4p8",
      "c2h2_3p0",
      "nirspec_g395h_gap",
    ]);
    expect(all[1]).toEqual({ id: "co2_4p3", label: "CO2 4.3", species: "CO2", kind: "molecular_band", lo: 4.15, hi: 4.5 });
    expect(bandMarks({ ...bandBase, species: DEFAULT_BAND_SPECIES }).map((b) => b.id)).toEqual(["h2o_2p7", "co2_4p3", "co_4p7", "ch4_3p3"]);
    expect(bandMarks({ ...bandBase, species: [] })).toEqual([]);
    expect(bandMarks({ ...bandBase, families: LINE_FAMILIES, species: ["CO2"] }).map((b) => b.id)).toEqual(["co2_4p3"]);
  });

  it("bandMarks clips a band to the axis and drops one outside it", () => {
    const marks = bandMarks({ ...bandBase, xMin: 2.6, xMax: 4.4, species: BAND_SPECIES });
    const span = (id: string) => {
      const mark = marks.find((b) => b.id === id);
      return mark ? [mark.lo, mark.hi] : null;
    };
    expect(span("h2o_2p7")).toEqual([2.6, 3.0]);
    expect(span("co2_4p3")).toEqual([4.15, 4.4]);
    expect(span("co_4p7")).toEqual([4.3, 4.4]);
    expect(span("hcn_4p8")).toBeNull();
  });

  it("bandMarks moves molecular and atomic rows with the redshift and leaves the instrument row in place", () => {
    const shifted = bandMarks({ ...bandBase, redshift: 1, xMin: 8.1, xMax: 10.5, species: BAND_SPECIES });
    expect(shifted.map((b) => b.id)).toEqual(["co2_4p3", "co_4p7", "hcn_4p8"]);
    const expected = [
      [8.3, 9.0],
      [8.6, 10.2],
      [9.4, 9.8],
    ];
    shifted.forEach((mark, i) => {
      expect(mark.lo).toBeCloseTo(expected[i][0], 9);
      expect(mark.hi).toBeCloseTo(expected[i][1], 9);
    });

    const sodiumShifted = bandMarks({ ...bandBase, redshift: 1, xMin: 1.1, xMax: 1.2, species: ["Na"] });
    expect(sodiumShifted.map((b) => b.id)).toEqual(["na_d"]);
    expect(sodiumShifted[0].lo).toBeCloseTo(1.1783166528, 9);
    expect(sodiumShifted[0].hi).toBeCloseTo(1.1795116294, 9);

    const gapOnly = bandMarks({ ...bandBase, redshift: 1, xMin: 3.5, xMax: 4.0, species: BAND_SPECIES });
    expect(gapOnly.map((b) => b.id)).toEqual(["nirspec_g395h_gap"]);
    expect(gapOnly[0].kind).toBe("instrument");
    expect(gapOnly[0].lo).toBe(3.72);
    expect(gapOnly[0].hi).toBe(3.82);

    const corrected = bandMarks({ ...bandBase, shiftKms: 30, species: BAND_SPECIES });
    const gap = corrected.find((b) => b.id === "nirspec_g395h_gap");
    expectNear(gap?.lo ?? null, 3.72, 1e-9);
    expectNear(gap?.hi ?? null, 3.82, 1e-9);
    const co2 = corrected.find((b) => b.id === "co2_4p3");
    expectNear(co2?.lo ?? null, 4.15 / (1 + 30 / c), 1e-12);
    expectNear(co2?.hi ?? null, 4.5 / (1 + 30 / c), 1e-12);
    expect(co2?.lo as number).toBeLessThan(4.15 - 4e-4);
  });

  it("bandMarks orders the edges on a frequency display and follows the air display", () => {
    const frequency = bandMarks({ ...bandBase, mode: "frequency", xMin: 60000, xMax: 80000, species: ["CO2"] });
    expect(frequency.map((b) => b.id)).toEqual(["co2_4p3"]);
    const loGhz = frequencyGhzFromWavelengthUm(4.5);
    const hiGhz = frequencyGhzFromWavelengthUm(4.15);
    expectNear(loGhz, c / 4.5, 1e-6);
    expectNear(loGhz, 66620.546, 1e-3);
    expectNear(hiGhz, 72239.146, 1e-3);
    expectNear(frequency[0].lo, loGhz, loGhz * 1e-6);
    expectNear(frequency[0].hi, hiGhz, hiGhz * 1e-6);
    expect(frequency[0].lo).toBeLessThan(frequency[0].hi);

    const air = bandMarks({ ...bandBase, mode: "wavelength_air", species: ["CO2"] });
    expect(air.map((b) => b.id)).toEqual(["co2_4p3"]);
    expect(air[0].lo).toBe(vacuumToAirUm(4.15));
    expect(air[0].hi).toBe(vacuumToAirUm(4.5));
    expect(air[0].lo).toBeLessThan(4.15);

    expect(bandMarks({ ...bandBase, mode: "velocity", species: BAND_SPECIES })).toEqual([]);
  });

  it("bandMarks returns the atomic doublets as narrow spans", () => {
    const marks = bandMarks({ ...bandBase, xMin: 0.58, xMax: 0.6, species: ["Na"] });
    expect(marks).toEqual([{ id: "na_d", label: "Na I D", species: "Na", kind: "atomic_line", lo: 0.5891583264, hi: 0.5897558147 }]);
  });

  it("nearestBandMark picks the narrowest band under the pointer and reaches narrow rows through the tolerance", () => {
    const marks = bandMarks({ ...bandBase, species: BAND_SPECIES });
    expect(nearestBandMark(marks, nirMapping, axisValueToPixel(4.4, nirMapping))?.id).toBe("co2_4p3");
    expect(nearestBandMark(marks, nirMapping, axisValueToPixel(3.0, nirMapping))?.id).toBe("hcn_3p0");
    expect(nearestBandMark(marks, nirMapping, axisValueToPixel(3.6, nirMapping))).toBeNull();
    expect(nearestBandMark(marks, nirMapping, axisValueToPixel(3.6, nirMapping), 13)?.id).toBe("h2s_3p8");
    expect(nearestBandMark([], nirMapping, axisValueToPixel(4.4, nirMapping))).toBeNull();

    const sodium = bandMarks({ ...bandBase, xMin: 0.58, xMax: 0.6, species: ["Na"] });
    const opticalMapping = mapping(0.5, 1.0, linearAxis(0.5, 1.0, 501), 501);
    const px = axisValueToPixel(0.5895, opticalMapping);
    expect(nearestBandMark(sodium, opticalMapping, px + 3)).toBe(sodium[0]);
    expect(nearestBandMark(sodium, opticalMapping, px + 20)).toBeNull();
  });

  it("pickedBandLabel names the range it sets", () => {
    expect(pickedBandLabel(bandById("co2_4p3")!)).toBe("range ← CO2 4.3 4.15 to 4.5 μm vacuum (display range)");
    expect(pickedBandLabel(bandById("na_d")!)).toBe("range ← Na I D 0.589158 to 0.589756 μm vacuum");
    expect(pickedBandLabel(bandById("nirspec_g395h_gap")!)).toBe("range ← G395H gap 3.72 to 3.82 μm vacuum");
  });

  it("toggleBandSpecies adds or removes one species and keeps the table order", () => {
    expect(toggleBandSpecies(["H2O", "CO2"], "CH4")).toEqual(["H2O", "CO2", "CH4"]);
    expect(toggleBandSpecies(["CH4", "H2O"], "H2O")).toEqual(["CH4"]);
    expect(toggleBandSpecies(["CO2", "H2O"], "Na")).toEqual(["H2O", "CO2", "Na"]);
    expect(toggleBandSpecies([], "nope")).toEqual([]);
    expect(toggleBandSpecies(["nope", "H2O"], "CO")).toEqual(["H2O", "CO"]);
  });
});
