import { describe, it, expect } from "vitest";
import {
  DEFAULT_FORCED_FIELDS,
  apertureMarks,
  bandChoice,
  bandDisplayName,
  bandOfPath,
  forcedRequest,
  isStale,
  noiseFactorOffer,
  parseForcedPositions,
  positionsCsvText,
  requestKey,
  sizeBand,
  sizeSource,
  type ForcedBandChoice,
  type ForcedFields,
  type ForcedRequestPlan,
} from "../forcedPhotometryRequest";
import type { ForcedBand, ForcedCell, ForcedPhotometryResult } from "../../shared/types/forcedPhotometry";

function choice(path: string, fileName: string, overrides: Partial<ForcedBandChoice> = {}): ForcedBandChoice {
  return { path, fileName, label: "", dataset: "", fCorr: null, ...overrides };
}

function fields(overrides: Partial<ForcedFields> = {}): ForcedFields {
  return {
    ...DEFAULT_FORCED_FIELDS,
    positionsText: "53.1 -28.1 a",
    bands: [choice("C:/d/r.fits", "r.fits")],
    ...overrides,
  };
}

function cell(overrides: Partial<ForcedCell> = {}): ForcedCell {
  return {
    x: 100.25,
    y: 200.5,
    on_image: true,
    error: null,
    flux_native: 10,
    flux_err_native: 1,
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
    flags_or: null,
    saturated: false,
    fwhm_px: 3.74,
    ...overrides,
  };
}

function band(index: number, path: string, overrides: Partial<ForcedBand> = {}): ForcedBand {
  const fileName = path.slice(path.lastIndexOf("/") + 1);
  return {
    index,
    path,
    file_name: fileName,
    label: fileName,
    error: null,
    filter: null,
    photcal: null,
    calibration_origin: null,
    jy_per_native_unit: null,
    bunit: null,
    pixel_scale_arcsec: 0.262,
    aperture_radius_px: null,
    annulus_px: null,
    aperture_correction: null,
    err_origin: null,
    dq_source: null,
    f_corr: null,
    n_measured: 0,
    n_failed: 0,
    warnings: [],
    mag_system: null,
    ...overrides,
  };
}

function result(bands: ForcedBand[], cellsBySource: (ForcedCell | null)[][]): ForcedPhotometryResult {
  return {
    bands,
    sources: cellsBySource.map((cells, i) => ({ id: `s${i}`, ra_deg: 53.1, dec_deg: -28.1, cells })),
    aperture: { radius_arcsec: 1, correction: "none" },
    limit_sigma: 3,
    elapsed_ms: 0,
    warnings: [],
  };
}

function keyOf(plan: ForcedRequestPlan): string {
  if (plan.request === null) throw new Error(`blocked: ${plan.blockers.join(" ")}`);
  return requestKey(plan.request);
}

const POSITION_SENTENCE = "Enter at least one position: RA and Dec in degrees, or sexagesimal with RA in hours.";

describe("forcedPhotometryRequest", () => {
  it("names a band after its file and the plane of its key", () => {
    expect(bandDisplayName({ name: "brick-r.fits", path: "C:/d/brick-r.fits" })).toBe("brick-r.fits");
    expect(bandDisplayName({ name: "cut.fits", path: "C:/d/cut.fits#hdu=0&plane=1" })).toBe("cut.fits #hdu=0&plane=1");
    expect(bandDisplayName({ name: "mef.fits", path: "C:/d/mef.fits#hdu=2" })).toBe("mef.fits #hdu=2");
    expect(bandChoice({ name: "cut.fits", path: "C:/d/cut.fits#hdu=0&plane=1" })).toEqual({
      path: "C:/d/cut.fits#hdu=0&plane=1",
      fileName: "cut.fits #hdu=0&plane=1",
      label: "",
      dataset: "",
      fCorr: null,
    });
  });

  it("reads positions in degrees and names them by label or row number", () => {
    const parsed = parseForcedPositions(
      "53.13873891283809 -28.08538886599608 A1\n53.1464291497623 -28.10260430600036\n",
    );
    expect(parsed.positions).toEqual([
      { id: "A1", ra_deg: 53.138738912838, dec_deg: -28.08538886599608 },
      { id: "2", ra_deg: 53.146429149762, dec_deg: -28.10260430600036 },
    ]);
    expect(parsed.errors).toEqual([]);
    expect(parsed.total).toBe(2);
    expect(parseForcedPositions("")).toEqual({ positions: [], errors: [], total: 0 });
    expect(parseForcedPositions("  \n ")).toEqual({ positions: [], errors: [], total: 0 });
  });

  it("reads sexagesimal right ascension in hours", () => {
    const parsed = parseForcedPositions("03:32:33.3 -28:05:07.4 s");
    expect(parsed.positions).toHaveLength(1);
    expect(parsed.positions[0].id).toBe("s");
    expect(parsed.positions[0].ra_deg).toBe(53.13875);
    expect(Math.abs(parsed.positions[0].dec_deg - -28.08538888888889)).toBeLessThanOrEqual(1e-12);
    expect(parseForcedPositions("359.9999999999999 10").positions[0].ra_deg).toBe(0);
  });

  it("takes a csv with ra, dec and id columns", () => {
    const parsed = parseForcedPositions("id,ra,dec\nstar 1,53.1,-28.1\nstar 2,53.2,-28.2");
    expect(parsed.positions.map((p) => p.id)).toEqual(["star 1", "star 2"]);
    expect(parsed.positions[0].ra_deg).toBe(53.1);
    expect(parsed.positions[0].dec_deg).toBe(-28.1);
    expect(parsed.positions[1].ra_deg).toBe(53.2);
    expect(parsed.positions[1].dec_deg).toBe(-28.2);
    expect(parsed.errors).toEqual([]);
  });

  it("renames repeated ids and says so", () => {
    const repeated = parseForcedPositions("10 20 a\n11 21 a\n12 22 a");
    expect(repeated.positions.map((p) => p.id)).toEqual(["a", "a_2", "a_3"]);
    expect(repeated.errors).toEqual(["id 'a' is used 3 times: the later rows are named a_2, a_3"]);
    const suffixInUse = parseForcedPositions("10 20 a\n11 21 a_2\n12 22 a");
    expect(suffixInUse.positions.map((p) => p.id)).toEqual(["a", "a_2", "a_3"]);
  });

  it("keeps the readable lines and reports the others", () => {
    const parsed = parseForcedPositions("53.1 -28.1 ok\nnonsense\n53.2 -95 bad");
    expect(parsed.positions.map((p) => p.id)).toEqual(["ok"]);
    expect(parsed.total).toBe(1);
    expect(parsed.errors).toEqual(["line 2: needs at least RA and Dec", "line 3: Dec -95 is outside -90..90 degrees"]);
  });

  it("cuts the list at 5000 positions with a message", () => {
    const text = Array.from({ length: 5003 }, (_, i) => `${10 + i / 1000} 20`).join("\n");
    const parsed = parseForcedPositions(text);
    expect(parsed.positions).toHaveLength(5000);
    expect(parsed.total).toBe(5003);
    expect(parsed.errors[parsed.errors.length - 1]).toBe(
      "only the first 5000 positions are measured: 3 more were left out",
    );
    expect(parsed.positions[4999].id).toBe("5000");
  });

  it("writes positions as a csv that reads back the same", () => {
    const text = positionsCsvText([
      { id: "A1", ra_deg: 53.138738912838, dec_deg: -28.08538886599608 },
      { id: "x, y #2", ra_deg: 1e-7, dec_deg: -0.5 },
    ]);
    expect(text).toBe("id,ra,dec\nA1,53.138738912838,-28.08538886599608\nx_ y _2,1e-7,-0.5");
    const back = parseForcedPositions(text);
    expect(back.positions.map((p) => p.id)).toEqual(["A1", "x_ y _2"]);
    expect(back.positions[0].ra_deg).toBe(53.138738912838);
    expect(back.positions[0].dec_deg).toBe(-28.08538886599608);
    expect(back.positions[1].ra_deg).toBe(1e-7);
    expect(back.positions[1].dec_deg).toBe(-0.5);
    expect(positionsCsvText([])).toBe("");
  });

  it("builds the request in the unit the user chose and leaves out what is not set", () => {
    expect(Object.isFrozen(DEFAULT_FORCED_FIELDS)).toBe(true);
    const bands = [choice("C:/d/g.fits", "g.fits", { label: " g ", dataset: "DECAM" }), choice("C:/d/r.fits", "r.fits")];
    const plan = forcedRequest(fields({ bands, radius: 2 }));
    expect(plan.blockers).toEqual([]);
    expect(plan.request).toEqual({
      bands: [{ path: "C:/d/g.fits", label: "g", dataset: "DECAM" }, { path: "C:/d/r.fits" }],
      positions: [{ id: "a", ra_deg: 53.1, dec_deg: -28.1 }],
      aperture: { radius_arcsec: 2, correction: "none" },
      options: { exclude_dq: false, limit_sigma: 3 },
    });
    expect(Object.keys(plan.request?.aperture ?? {})).toEqual(["radius_arcsec", "correction"]);

    const pixelPlan = forcedRequest(
      fields({
        bands: [{ ...bands[0], fCorr: 1.05 }, bands[1]],
        radius: 11.11,
        radiusUnit: "px",
        annulusInner: 25,
        annulusOuter: 35,
        growthCorrection: true,
        referenceRadiusArcsec: 3.5,
        limitSigma: 5,
        excludeDq: true,
        gain: 4.76,
      }),
    );
    expect(pixelPlan.blockers).toEqual([]);
    expect(pixelPlan.request?.aperture).toEqual({
      radius_px: 11.11,
      annulus_px: [25, 35],
      correction: "growth_curve",
      reference_radius_arcsec: 3.5,
    });
    expect(pixelPlan.request?.options).toEqual({ exclude_dq: true, limit_sigma: 5, gain: 4.76, f_corr: [1.05, null] });

    const noCorrection = forcedRequest(fields({ referenceRadiusArcsec: 3.5, growthCorrection: false }));
    expect(noCorrection.request?.aperture).toEqual({ radius_arcsec: 1, correction: "none" });
    expect(Object.keys(noCorrection.request?.aperture ?? {})).not.toContain("reference_radius_arcsec");
  });

  it("says why a run cannot start", () => {
    const cases: [Partial<ForcedFields>, string[]][] = [
      [{ bands: [] }, ["Choose at least one band image."]],
      [
        { bands: Array.from({ length: 13 }, (_, i) => choice(`C:/d/b${i}.fits`, `b${i}.fits`)) },
        ["At most 12 bands can be measured in one run (13 chosen)."],
      ],
      [{ positionsText: "" }, [POSITION_SENTENCE]],
      [{ radius: null }, ["Enter the aperture radius."]],
      [{ annulusInner: 4 }, ["The sky annulus needs both an inner and an outer radius."]],
      [{ annulusInner: 6, annulusOuter: 5 }, ["The outer sky radius must be larger than the inner one."]],
      [{ radius: 2, annulusInner: 2, annulusOuter: 5 }, ["The sky annulus must start outside the aperture."]],
      [
        { bands: [], positionsText: "", radius: null },
        ["Choose at least one band image.", POSITION_SENTENCE, "Enter the aperture radius."],
      ],
    ];
    for (const [overrides, blockers] of cases) {
      const plan = forcedRequest(fields(overrides));
      expect(plan.blockers).toEqual(blockers);
      expect(plan.request).toBeNull();
    }
  });

  it("tells when the fields no longer match the result", () => {
    const first = forcedRequest(fields());
    const same = forcedRequest(fields());
    const firstKey = keyOf(first);
    expect(keyOf(same)).toBe(firstKey);
    expect(isStale(null, first)).toBe(false);
    expect(isStale(firstKey, first)).toBe(false);
    expect(isStale(firstKey, forcedRequest(fields({ radius: 2.5 })))).toBe(true);
    expect(isStale(firstKey, forcedRequest(fields({ bands: [] })))).toBe(true);
  });

  it("offers the depth noise factor only for the aperture it was measured with", () => {
    const depth = { radius_arcsec: 1.0, radius_px: 3.8167938931297707, f_corr: 1.647 };
    expect(noiseFactorOffer(depth, 1.0, "arcsec")).toEqual({ kind: "usable", fCorr: 1.647, radius: 1, unit: "arcsec" });
    expect(noiseFactorOffer(depth, 1.005, "arcsec").kind).toBe("usable");
    expect(noiseFactorOffer(depth, 1.02, "arcsec")).toEqual({ kind: "other_radius", radius: 1, unit: "arcsec" });
    expect(noiseFactorOffer(depth, 2.0, "arcsec")).toEqual({ kind: "other_radius", radius: 1, unit: "arcsec" });
    expect(noiseFactorOffer(depth, 3.8168, "px")).toEqual({
      kind: "usable",
      fCorr: 1.647,
      radius: 3.8167938931297707,
      unit: "px",
    });
    expect(noiseFactorOffer({ ...depth, f_corr: null }, 1.0, "arcsec")).toEqual({ kind: "none" });
    expect(noiseFactorOffer({ ...depth, radius_arcsec: null }, 1.0, "arcsec")).toEqual({ kind: "none" });
    expect(noiseFactorOffer(null, 1.0, "arcsec")).toEqual({ kind: "none" });
    expect(noiseFactorOffer(undefined, 1.0, "arcsec")).toEqual({ kind: "none" });
    expect(noiseFactorOffer(depth, null, "arcsec")).toEqual({ kind: "none" });
  });

  it("sends a typed zero point per band and nothing when none is typed", () => {
    const ab = { zp: 25, system: "ab" as const, perSecond: false };
    const vega = { zp: 22.5, system: "vega" as const, perSecond: true };
    const g = choice("C:/d/g.fits", "g.fits");
    const r = choice("C:/d/r.fits", "r.fits");
    const one = forcedRequest(fields({ bands: [{ ...g, zeroPoint: ab }, r] }));
    expect(one.request?.options).toEqual({
      exclude_dq: false,
      limit_sigma: 3,
      zero_points: [{ zp: 25, system: "ab", per_second: false }, null],
    });
    const both = forcedRequest(fields({ bands: [{ ...g, zeroPoint: ab }, { ...r, zeroPoint: vega }] }));
    expect(both.request?.options?.zero_points?.[1]).toEqual({ zp: 22.5, system: "vega", per_second: true });
    const withFactor = forcedRequest(fields({ bands: [{ ...g, fCorr: 1.647, zeroPoint: ab }, r] }));
    expect(Object.keys(withFactor.request?.options ?? {})).toEqual(["exclude_dq", "limit_sigma", "f_corr", "zero_points"]);
    const cleared = forcedRequest(fields({ bands: [{ ...g, zeroPoint: null }, { ...r, zeroPoint: null }] }));
    expect(Object.keys(cleared.request?.options ?? {})).not.toContain("zero_points");
    expect(cleared.request?.options).toEqual({ exclude_dq: false, limit_sigma: 3 });
    const absent = forcedRequest(fields({ bands: [g, r] }));
    expect(Object.keys(absent.request?.options ?? {})).not.toContain("zero_points");
    expect(absent.request?.options).toEqual({ exclude_dq: false, limit_sigma: 3 });
    const zeroIsAValue = forcedRequest(fields({ bands: [{ ...g, zeroPoint: { ...ab, zp: 0 } }] }));
    expect(zeroIsAValue.request?.options?.zero_points).toEqual([{ zp: 0, system: "ab", per_second: false }]);
  });

  it("a band choice from a file has no zero point", () => {
    expect("zeroPoint" in bandChoice({ name: "r.fits", path: "C:/d/r.fits" })).toBe(false);
  });

  it("gives the size inputs of a measured cell in arcsec", () => {
    const r = band(0, "C:/d/r.fits");
    const size = sizeSource(cell(), r);
    expect(size).not.toBeNull();
    expect(Math.abs((size?.fwhmArcsec ?? Number.NaN) - 0.97988)).toBeLessThanOrEqual(1e-12);
    expect(size?.snr).toBe(20);
    expect(sizeSource(cell({ fwhm_px: null }), r)).toBeNull();
    expect(sizeSource(cell({ on_image: false }), r)).toBeNull();
    expect(sizeSource(cell({ error: "x" }), r)).toBeNull();
    expect(sizeSource(null, r)).toBeNull();
    expect(sizeSource(cell(), band(0, "C:/d/r.fits", { pixel_scale_arcsec: null }))).toBeNull();
    expect(sizeSource(cell(), band(0, "C:/d/r.fits", { error: "no WCS" }))).toBeNull();
    expect(sizeSource(cell({ snr: null }), r)?.snr).toBeNull();
  });

  it("picks the band of the image on screen for sizes when it has one", () => {
    const bands = [band(0, "C:/d/g.fits"), band(1, "C:/d/r.fits")];
    const gWithoutSize = result(bands, [[cell({ fwhm_px: null }), cell({ fwhm_px: 3.74 })]]);
    expect(sizeBand(gWithoutSize, "C:/d/g.fits")).toBe(1);
    expect(sizeBand(gWithoutSize, "C:/d/r.fits")).toBe(1);
    expect(sizeBand(gWithoutSize, null)).toBe(1);
    const gWithSize = result(bands, [[cell({ fwhm_px: 3.1 }), cell({ fwhm_px: 3.74 })]]);
    expect(sizeBand(gWithSize, "C:/d/g.fits")).toBe(0);
    expect(sizeBand(gWithSize, "C:/d/r.fits")).toBe(1);
    expect(sizeBand(gWithSize, null)).toBe(0);
    const noSize = result(bands, [[cell({ fwhm_px: null }), cell({ fwhm_px: null })]]);
    expect(sizeBand(noSize, "C:/d/g.fits")).toBeNull();
  });

  it("finds the band of a path and marks its apertures", () => {
    const bands = [band(0, "C:/d/g.fits"), band(1, "C:/d/r.fits"), band(2, "C:/d/i.fits", { error: "no WCS" })];
    const measured = result(bands, [
      [cell(), cell(), null],
      [cell(), cell({ upper_limit_jy: 3e-7 }), null],
      [cell(), null, null],
      [cell(), cell({ on_image: false }), null],
    ]);
    expect(bandOfPath(measured, "C:/d/r.fits")).toBe(1);
    expect(bandOfPath(measured, "C:/d/x.fits")).toBeNull();
    expect(bandOfPath(measured, null)).toBeNull();
    expect(bandOfPath(measured, "C:/d/i.fits")).toBeNull();
    expect(apertureMarks(measured, 1)).toEqual([
      { source: 0, id: "s0", x: 100.25, y: 200.5, limit: false, failed: false },
      { source: 1, id: "s1", x: 100.25, y: 200.5, limit: true, failed: false },
    ]);
    const failed = result(bands, [[cell(), cell({ error: "the aperture holds no pixel with data" }), null]]);
    expect(apertureMarks(failed, 1)).toEqual([{ source: 0, id: "s0", x: 100.25, y: 200.5, limit: false, failed: true }]);
    expect(apertureMarks(measured, 9)).toEqual([]);
  });
});
