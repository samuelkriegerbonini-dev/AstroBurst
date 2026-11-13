import { describe, expect, it } from "vitest";
import type { BatchPhotometryResult, PhotCal, PhotometryMeasurement, StarPhotometry } from "../../services/analysis";
import {
  GAIA_MATCH_PREF_KEY,
  appliedDepthFactor,
  batchWarnings,
  calibratedMagnitude,
  calibrationText,
  convertedRadiusText,
  createClickLogGate,
  depthNoiseText,
  depthRadiusText,
  detectedStarsLabel,
  gaiaFailureWarning,
  limitSentence,
  loadGaiaMatchPreference,
  magnitudeColumnHeader,
  magnitudeTitle,
  measurementWarnings,
  nativeZeroPointOf,
  photometryLimit,
  replaceNewest,
  saveGaiaMatchPreference,
  userZeroPoint,
  withWarning,
} from "../photometryPanel";
import { noiseFactorOffer, type NoiseFactorOffer } from "../forcedPhotometryRequest";
import { formatFluxDensityJy, formatWithUncertainty } from "../formatUncertainty";
import type { ImageDepth } from "../../shared/types/depth";

function memoryStorage(initial: Record<string, string> = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => {
      map.set(k, v);
    },
    map,
  };
}

function measurement(warnings: string[] = []): PhotometryMeasurement {
  return {
    photometry: {} as PhotometryMeasurement["photometry"],
    sky: null,
    gaia: null,
    photcal: null,
    warnings,
    masked: false,
    elapsed_ms: 1,
  };
}

describe("Gaia match preference", () => {
  it("is off when nothing is stored", () => {
    expect(loadGaiaMatchPreference(memoryStorage())).toBe(false);
  });

  it("remembers the last choice", () => {
    const s = memoryStorage();
    saveGaiaMatchPreference(true, s);
    expect(s.map.get(GAIA_MATCH_PREF_KEY)).toBe("true");
    expect(loadGaiaMatchPreference(s)).toBe(true);
    saveGaiaMatchPreference(false, s);
    expect(loadGaiaMatchPreference(s)).toBe(false);
  });

  it("treats a garbage value as off", () => {
    expect(loadGaiaMatchPreference(memoryStorage({ [GAIA_MATCH_PREF_KEY]: "yes" }))).toBe(false);
  });

  it("is off when the storage throws, and saving into a throwing storage does not throw", () => {
    const throwing = {
      getItem: (): string | null => {
        throw new Error("blocked");
      },
      setItem: (): void => {
        throw new Error("blocked");
      },
    };
    expect(loadGaiaMatchPreference(throwing)).toBe(false);
    expect(() => saveGaiaMatchPreference(true, throwing)).not.toThrow();
  });
});

describe("Gaia failure warning", () => {
  it("names the reason of an Error and of a plain string", () => {
    expect(gaiaFailureWarning(new Error("timed out"))).toBe("Gaia query failed: timed out");
    expect(gaiaFailureWarning("offline")).toBe("Gaia query failed: offline");
  });

  it("adds a warning once without touching the original result", () => {
    const base = measurement(["saturated core"]);
    const once = withWarning(base, "Gaia query failed: offline");
    expect(once.warnings).toEqual(["saturated core", "Gaia query failed: offline"]);
    expect(base.warnings).toEqual(["saturated core"]);
    expect(withWarning(once, "Gaia query failed: offline")).toBe(once);
  });
});

describe("replaceNewest", () => {
  it("swaps the newest entry when it is the local measurement", () => {
    const local = measurement();
    const older = measurement();
    const matched = measurement(["Gaia: no G<17 star within 5\""]);
    expect(replaceNewest([local, older], local, matched)).toEqual([matched, older]);
  });

  it("leaves the history alone when a newer click already replaced the head", () => {
    const local = measurement();
    const newer = measurement();
    const history = [newer, local];
    expect(replaceNewest(history, local, measurement())).toBe(history);
    const empty: PhotometryMeasurement[] = [];
    expect(replaceNewest(empty, local, measurement())).toBe(empty);
  });
});

describe("click log gate", () => {
  function recorder() {
    const rows: string[] = [];
    return { rows, append: (row: string) => () => void rows.push(row) };
  }

  it("logs one row for a click whose panel goes away while Gaia is pending", () => {
    const gate = createClickLogGate();
    const log = recorder();
    const seq = gate.begin();
    gate.hold(seq, log.append("local"));
    gate.supersede();
    expect(gate.isCurrent(seq)).toBe(false);
    gate.commit(seq, log.append("matched"));
    expect(log.rows).toEqual(["local"]);
  });

  it("logs only the matched row when Gaia answers before anything else happens", () => {
    const gate = createClickLogGate();
    const log = recorder();
    const seq = gate.begin();
    gate.hold(seq, log.append("local"));
    expect(gate.isCurrent(seq)).toBe(true);
    gate.commit(seq, log.append("matched"));
    gate.supersede();
    gate.begin();
    expect(log.rows).toEqual(["matched"]);
  });

  it("logs the held row of the previous click when a new click starts, and drops its late match", () => {
    const gate = createClickLogGate();
    const log = recorder();
    const first = gate.begin();
    gate.hold(first, log.append("A local"));
    const second = gate.begin();
    expect(log.rows).toEqual(["A local"]);
    gate.commit(first, log.append("A matched"));
    gate.commit(second, log.append("B local"));
    expect(log.rows).toEqual(["A local", "B local"]);
  });

  it("ignores a hold that arrives after the click was superseded", () => {
    const gate = createClickLogGate();
    const log = recorder();
    const seq = gate.begin();
    gate.supersede();
    gate.hold(seq, log.append("stale"));
    gate.supersede();
    gate.begin();
    expect(log.rows).toEqual([]);
  });
});

const FWHM_FALLBACK = "FWHM not measured: the automatic aperture radius of 6.0 px is a fallback, not 1.5 x FWHM";
const NO_DATA_3 = "3 aperture pixels without data (non-finite): the flux is a lower limit";
const NO_DATA_9 = "9 aperture pixels without data (non-finite): the flux is a lower limit";
const PROCESSED = "photometry on processed data (stack)";

function starWarnings(warnings: string[]): StarPhotometry {
  return { warnings } as StarPhotometry;
}

describe("measurementWarnings", () => {
  it("shows the per-star warnings after the image warnings, each once", () => {
    const result = { ...measurement([PROCESSED, FWHM_FALLBACK]), photometry: starWarnings([FWHM_FALLBACK, NO_DATA_3]) };
    expect(measurementWarnings(result)).toEqual([PROCESSED, FWHM_FALLBACK, NO_DATA_3]);
  });

  it("returns the image warnings alone when the star has none", () => {
    expect(measurementWarnings({ ...measurement([PROCESSED]), photometry: starWarnings([]) })).toEqual([PROCESSED]);
  });
});

describe("batchWarnings", () => {
  it("adds each distinct per-star warning of the measured rows once, after the image warnings", () => {
    const result: BatchPhotometryResult = {
      rows: [
        { index: 0, photometry: starWarnings([FWHM_FALLBACK]), sky: null, error: null },
        { index: 1, photometry: null, sky: null, error: "outside the image" },
        { index: 2, photometry: starWarnings([FWHM_FALLBACK, NO_DATA_3]), sky: null, error: null },
        { index: 3, photometry: starWarnings([NO_DATA_9, PROCESSED]), sky: null, error: null },
      ],
      photcal: null,
      warnings: [PROCESSED],
      masked: false,
      n_measured: 3,
      n_failed: 1,
      elapsed_ms: 4,
    };
    expect(batchWarnings(result)).toEqual([PROCESSED, FWHM_FALLBACK, NO_DATA_3, NO_DATA_9]);
  });
});

describe("detectedStarsLabel", () => {
  it("names the cap when detection found more stars than it kept", () => {
    expect(detectedStarsLabel(200, 3120)).toBe("Detected stars (200 of 3120)");
  });

  it("shows the plain count when nothing was dropped or the total is unknown", () => {
    expect(detectedStarsLabel(57, 57)).toBe("Detected stars (57)");
    expect(detectedStarsLabel(57, null)).toBe("Detected stars (57)");
    expect(detectedStarsLabel(57, undefined)).toBe("Detected stars (57)");
    expect(detectedStarsLabel(0, 3120)).toBe("Detected stars (0)");
    expect(detectedStarsLabel(200, Number.NaN)).toBe("Detected stars (200)");
  });
});

describe("convertedRadiusText", () => {
  it("converts a typed radius when the unit changes and keeps four significant digits", () => {
    expect(convertedRadiusText("5", "px", "arcsec", 0.262)).toBe("1.31");
    expect(convertedRadiusText("1.31", "arcsec", "px", 0.262)).toBe("5");
    expect(convertedRadiusText("1", "arcsec", "px", 0.262)).toBe("3.817");
  });

  it("leaves the text alone when it cannot convert", () => {
    expect(convertedRadiusText("", "px", "arcsec", 0.262)).toBe("");
    expect(convertedRadiusText("2,5", "px", "arcsec", 0.262)).toBe("2,5");
    expect(convertedRadiusText("abc", "arcsec", "px", 0.262)).toBe("abc");
    expect(convertedRadiusText("5", "px", "arcsec", null)).toBe("5");
    expect(convertedRadiusText("1", "arcsec", "px", null)).toBe("1");
    expect(convertedRadiusText("5", "px", "px", 0.262)).toBe("5");
    expect(convertedRadiusText("1.31", "arcsec", "arcsec", 0.262)).toBe("1.31");
  });
});

const AB_USER: PhotCal = {
  area_applied: false,
  bunit: "adu",
  convention: { exptime: null, exptime_applied: false, keyword: "user", kind: "generic_zero_point", per_second: false, system: "ab", zp: 25.0 },
  jy_per_native_unit: 3.63078054770101e-7,
  mag_system: "ab",
  notes: ["user zero point: mag = 25.0 - 2.5 log10(sum)"],
  origin: "user zero point 25.0 AB",
  warnings: ["calibrated with a user zero point"],
  label: "user zero point (AB)",
};

const VEGA_USER: PhotCal = {
  area_applied: false,
  bunit: "adu",
  convention: { exptime: null, exptime_applied: false, keyword: "user", kind: "generic_zero_point", per_second: false, system: "vega", zp: 21.432 },
  jy_per_native_unit: null,
  mag_system: "vega",
  notes: ["user zero point: mag = 21.432 - 2.5 log10(sum)"],
  origin: "user zero point 21.432 Vega",
  warnings: [
    "calibrated with a user zero point",
    "the user zero point is in the Vega system: magnitudes are in the Vega system and no AB magnitude or flux in Jy is given",
  ],
  label: "user zero point (Vega)",
};

const JWST_I2D: PhotCal = {
  convention: { kind: "jwst_mjy_sr", pixar_sr: 2.29e-14, pixar_a2: 0.000974, derived_pixar: false },
  bunit: "MJy/sr",
  notes: [],
  warnings: [],
  label: "JWST MJy/sr, PIXAR_SR from header",
};

function star(over: Partial<StarPhotometry>): StarPhotometry {
  return { mag_inst: -7.5, net_flux: 1000, flux_err: 10, mag_ab: null, mag_ab_err: null, ...over } as StarPhotometry;
}

describe("calibration of the magnitude", () => {
  it("titles a magnitude by the system of its zero point", () => {
    expect(magnitudeTitle("ab")).toBe("AB mag");
    expect(magnitudeTitle("unknown")).toBe("AB mag (system assumed)");
    expect(magnitudeTitle("vega")).toBe("Vega mag");
    expect(magnitudeTitle("st")).toBe("ST mag");
    expect(magnitudeTitle(null)).toBe("AB mag");
    expect(magnitudeTitle(undefined)).toBe("AB mag");
    expect(magnitudeColumnHeader("ab")).toBe("AB");
    expect(magnitudeColumnHeader("unknown")).toBe("AB");
    expect(magnitudeColumnHeader("vega")).toBe("Vega");
    expect(magnitudeColumnHeader("st")).toBe("ST");
    expect(magnitudeColumnHeader(undefined)).toBe("AB");
  });

  it("reads the native zero point of a generic zero point convention only", () => {
    expect(nativeZeroPointOf(VEGA_USER)).toEqual({ zp: 21.432, exptimeS: null, exptimeApplied: false });
    const perSecond: PhotCal = {
      ...VEGA_USER,
      convention: { exptime: 600, exptime_applied: true, keyword: "user", kind: "generic_zero_point", per_second: true, system: "vega", zp: 21.432 },
    };
    expect(nativeZeroPointOf(perSecond)).toEqual({ zp: 21.432, exptimeS: 600, exptimeApplied: true });
    expect(nativeZeroPointOf(JWST_I2D)).toBeNull();
    expect(nativeZeroPointOf(null)).toBeNull();
    expect(nativeZeroPointOf(undefined)).toBeNull();
  });

  it("computes a Vega magnitude from the instrumental magnitude and the zero point", () => {
    const m = calibratedMagnitude(star({ mag_inst: -9.87, net_flux: 8872, flux_err: 50, mag_ab: null, mag_ab_err: null }), VEGA_USER);
    expect(m.mag).toBeCloseTo(11.562, 10);
    expect(m.magErr).toBeCloseTo((1.0857362047581294 * 50) / 8872, 12);
    expect(m.system).toBe("vega");
  });

  it("passes the AB magnitude of the backend through", () => {
    expect(calibratedMagnitude(star({ mag_ab: 17.5, mag_ab_err: 0.011 }), AB_USER)).toEqual({ mag: 17.5, magErr: 0.011, system: "ab" });
    expect(calibratedMagnitude(star({ mag_ab: 17.5, mag_ab_err: 0.011 }), null)).toEqual({ mag: null, magErr: null, system: null });
    expect(calibratedMagnitude(star({ mag_ab: 17.5, mag_ab_err: 0.011 }), JWST_I2D)).toEqual({ mag: 17.5, magErr: 0.011, system: "ab" });
    expect(calibratedMagnitude(star({ mag_ab: 17.5, mag_ab_err: 0.011 }), { ...AB_USER, mag_system: "unknown" }).system).toBe("unknown");
  });

  it("reads the calibration sentence, falling back to the label", () => {
    expect(calibrationText({ ...JWST_I2D, origin: "MAGZERO = 30.0, AB assumed" })).toBe("MAGZERO = 30.0, AB assumed");
    expect(calibrationText({ ...JWST_I2D, origin: "" })).toBe(JWST_I2D.label);
    expect(calibrationText(JWST_I2D)).toBe(JWST_I2D.label);
    expect(calibrationText(null)).toBeNull();
    expect(calibrationText(undefined)).toBeNull();
  });

  it("builds the wire zero point only from a valid entry", () => {
    expect(userZeroPoint("25", "ab", false)).toEqual({ zp: 25, system: "ab", per_second: false });
    expect(userZeroPoint("21.432", "vega", true)).toEqual({ zp: 21.432, system: "vega", per_second: true });
    for (const text of ["", "  ", "2,5", "abc", "250"]) expect(userZeroPoint(text, "ab", false), text).toBeUndefined();
  });
});

const LEGACY_DEPTH = { radius_arcsec: 1.0, radius_px: 3.8167938931297707, f_corr: 1.647 } as unknown as ImageDepth;

function phot(overrides: Partial<StarPhotometry> = {}): StarPhotometry {
  return {
    x: 10,
    y: 12,
    peak: 40,
    net_flux: 20,
    flux_err: 12.3,
    mag_inst: -3.25,
    snr: 1.6,
    fwhm: 3.2,
    aperture_radius: 3.82,
    aperture_pixels: 46,
    aperture_area: 45.8,
    bg_mean: 10,
    bg_sigma: 1.5,
    bg_pixels: 300,
    saturated: false,
    saturation_source: "none",
    n_masked: 0,
    n_nonfinite: 0,
    n_saturated: 0,
    err_used: true,
    aperture_correction: null,
    ee_in_aperture: null,
    flux_total: null,
    plateau_radius: null,
    flux_jy: 2.0e-7,
    flux_err_jy: 1.1e-7,
    mag_ab: 25.647,
    mag_ab_err: 0.6,
    mag_ab_total: null,
    st_mag: null,
    sky_inner: 7.64,
    sky_outer: 11.46,
    growth_curve: [],
    ee50_radius: null,
    ee80_radius: null,
    warnings: [],
    ...overrides,
  };
}

describe("noise factor of the Depth section", () => {
  it("offers the factor of the Depth section only for the aperture it was measured with and applies it only once accepted", () => {
    const usable = noiseFactorOffer(LEGACY_DEPTH, 1, "arcsec");
    expect(appliedDepthFactor(usable, null)).toBeNull();
    expect(appliedDepthFactor(usable, 1.647)).toBe(1.647);
    expect(appliedDepthFactor(usable, 1.65)).toBeNull();
    expect(depthRadiusText(usable)).toBe("r = 1 arcsec");
    expect(depthNoiseText(usable, null)).toBe(
      "Depth section: f_corr 1.65 for this image at r = 1 arcsec, the radius of this aperture. Not applied: click Use to multiply the errors by it before the limit test.",
    );
    expect(depthNoiseText(usable, 1.647)).toBe(
      "f_corr 1.65 from the Depth section (r = 1 arcsec) multiplies the errors of the next measurements before the limit test.",
    );
    expect(depthRadiusText(noiseFactorOffer(LEGACY_DEPTH, 3.8168, "px"))).toBe("r = 3.817 px");

    const other = noiseFactorOffer(LEGACY_DEPTH, 2, "arcsec");
    expect(appliedDepthFactor(other, 1.647)).toBeNull();
    expect(depthNoiseText(other, null)).toBe(
      "The Depth section measured this image at r = 1 arcsec, not with this aperture: its f_corr is not offered. Measure the depth again with this radius to get one.",
    );

    const none: NoiseFactorOffer = { kind: "none" };
    expect(appliedDepthFactor(none, 1.647)).toBeNull();
    expect(depthRadiusText(none)).toBeNull();
    expect(depthNoiseText(none, null)).toBeNull();
  });
});

describe("upper limit of a photometry measurement", () => {
  it("calls a flux below three times its error a limit and gives the limit in Jy and AB", () => {
    expect(photometryLimit(phot(), null)).toEqual({
      sigma: 3,
      fCorr: null,
      isLimit: true,
      limitJy: expect.closeTo(3.3e-7, 12),
      limitAb: expect.closeTo(25.103715150305284, 12),
      limitNative: expect.closeTo(36.900000000000006, 12),
    });
    const detected = photometryLimit(phot({ flux_jy: 5.0e-7, net_flux: 50 }), null);
    expect(detected.isLimit).toBe(false);
    expect([detected.limitJy, detected.limitAb, detected.limitNative]).toEqual([null, null, null]);
  });

  it("applies an accepted factor of the Depth section to a measurement with an error layer and nothing without it", () => {
    const measured = phot({ flux_jy: 5.0e-7, net_flux: 50 });
    const accepted = photometryLimit(measured, 1.65);
    expect(accepted.isLimit).toBe(true);
    expect(accepted.fCorr).toBe(1.65);
    expect(accepted.limitJy).toBeCloseTo(5.445e-7, 15);
    expect(accepted.limitAb).toBeCloseTo(24.560005289770515, 12);
    expect(accepted.limitNative).toBeCloseTo(60.885, 12);

    const notAccepted = photometryLimit(measured, null);
    expect(notAccepted.fCorr).toBeNull();
    expect(notAccepted.isLimit).toBe(false);

    const skyScatter = photometryLimit(phot({ flux_jy: 5.0e-7, net_flux: 50, err_used: false }), 1.65);
    expect(skyScatter.fCorr).toBeNull();
    expect(skyScatter.isLimit).toBe(false);
  });

  it("keeps a negative flux as a limit and never calls a measurement without an error a limit", () => {
    const negative = photometryLimit(phot({ flux_jy: -1.0e-7, net_flux: -10 }), null);
    expect(negative.isLimit).toBe(true);
    expect(negative.limitJy).toBeCloseTo(3.3e-7, 15);
    expect(photometryLimit(phot({ flux_err: 0, flux_err_jy: 0 }), null).isLimit).toBe(false);
  });

  it("gives the limit in the unit of the net flux when there is no flux in Jy", () => {
    const native = photometryLimit(phot({ flux_jy: null, flux_err_jy: null }), null);
    expect(native.isLimit).toBe(true);
    expect(native.limitJy).toBeNull();
    expect(native.limitAb).toBeNull();
    expect(native.limitNative).toBeCloseTo(36.9, 12);
  });

  it("words the limit for the panel", () => {
    expect(formatFluxDensityJy(3.3e-7, null)).toBe("330.0 nJy");
    expect(limitSentence(photometryLimit(phot(), null), "adu")).toBe(
      `not detected at 3 sigma: AB > 25.10 (flux below ${formatFluxDensityJy(3.3e-7, null)})`,
    );
    const native = photometryLimit(phot({ flux_jy: null, flux_err_jy: null }), null);
    expect(formatWithUncertainty(36.900000000000006, null)).toBe("36.90");
    expect(limitSentence(native, "adu")).toBe(`not detected at 3 sigma: net flux below ${formatWithUncertainty(36.900000000000006, null)} adu`);
    expect(limitSentence(native, null)).toBe(`not detected at 3 sigma: net flux below ${formatWithUncertainty(36.900000000000006, null)}`);
    const withFactor = limitSentence(photometryLimit(phot({ flux_jy: 5.0e-7, net_flux: 50 }), 1.65), "adu");
    expect(withFactor).toBe(
      `not detected at 3 sigma: AB > 24.56 (flux below ${formatFluxDensityJy(5.445e-7, null)}); errors multiplied by f_corr 1.65 from the Depth section`,
    );
    expect(withFactor!.endsWith("; errors multiplied by f_corr 1.65 from the Depth section")).toBe(true);
    expect(limitSentence(photometryLimit(phot({ flux_jy: 5.0e-7, net_flux: 50 }), null), "adu")).toBeNull();
  });
});
