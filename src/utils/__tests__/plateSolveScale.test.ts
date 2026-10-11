import { describe, it, expect } from "vitest";
import {
  DEFAULT_HINT_TEXTS,
  DEFAULT_SCALE_HIGH_TEXT,
  DEFAULT_SCALE_LOW_TEXT,
  headerHintTexts,
  headerPositionHintTexts,
  mergeHintTexts,
  parsePositionHint,
  parseRadius,
  parseScaleRange,
  positionHintStatus,
  scaleHintFromPixelScale,
  type HintField,
  type HintTexts,
} from "../plateSolveScale";

describe("scaleHintFromPixelScale", () => {
  it("brackets a NIRCam short-wave scale by 0.8x and 1.25x, rounded outwards", () => {
    expect(scaleHintFromPixelScale(0.031)).toEqual({ low: "0.0248", high: "0.0388" });
  });

  it("keeps exact products without drifting past them", () => {
    expect(scaleHintFromPixelScale(0.04)).toEqual({ low: "0.032", high: "0.05" });
    expect(scaleHintFromPixelScale(0.05)).toEqual({ low: "0.04", high: "0.0625" });
  });

  it("brackets an amateur scale above one arcsecond", () => {
    expect(scaleHintFromPixelScale(1.5)).toEqual({ low: "1.2", high: "1.88" });
  });

  it("gives no hint for a missing, zero, negative or non-finite scale", () => {
    expect(scaleHintFromPixelScale(null)).toBeNull();
    expect(scaleHintFromPixelScale(undefined)).toBeNull();
    expect(scaleHintFromPixelScale(0)).toBeNull();
    expect(scaleHintFromPixelScale(-0.1)).toBeNull();
    expect(scaleHintFromPixelScale(Number.NaN)).toBeNull();
    expect(scaleHintFromPixelScale(Number.POSITIVE_INFINITY)).toBeNull();
  });

  it("always returns a range that parses and contains the header scale", () => {
    for (const scale of [0.0123, 0.031, 0.063, 0.1, 0.396, 2.06, 13.7, 206]) {
      const hint = scaleHintFromPixelScale(scale);
      expect(hint).not.toBeNull();
      const parsed = parseScaleRange(hint!.low, hint!.high);
      expect(parsed.error).toBeNull();
      if (parsed.error === null) {
        expect(parsed.low).toBeLessThanOrEqual(scale * 0.8 + 1e-12);
        expect(parsed.high).toBeGreaterThanOrEqual(scale * 1.25 - 1e-12);
      }
    }
  });
});

describe("parseScaleRange", () => {
  it("accepts the defaults", () => {
    expect(parseScaleRange(DEFAULT_SCALE_LOW_TEXT, DEFAULT_SCALE_HIGH_TEXT)).toEqual({ low: 0.1, high: 10, error: null });
  });

  it("accepts a small scale typed digit by digit once complete", () => {
    expect(parseScaleRange("0.05", " 0.08 ")).toEqual({ low: 0.05, high: 0.08, error: null });
    expect(parseScaleRange("1e-2", "2e-2")).toEqual({ low: 0.01, high: 0.02, error: null });
  });

  it("rejects partial or empty text instead of snapping it to a default", () => {
    for (const partial of ["", " ", ".", "0", "0.", "-", "abc", "0.1x"]) {
      expect(parseScaleRange(partial, "10").error).toBe("Scale low must be a number above 0.");
      expect(parseScaleRange("0.1", partial).error).toBe("Scale high must be a number above 0.");
    }
  });

  it("rejects negative and non-finite values", () => {
    expect(parseScaleRange("-1", "10").error).toBe("Scale low must be a number above 0.");
    expect(parseScaleRange("0.1", "Infinity").error).toBe("Scale high must be a number above 0.");
  });

  it("rejects a low bound that is not below the high bound", () => {
    expect(parseScaleRange("5", "5").error).toBe("Scale low must be below scale high.");
    expect(parseScaleRange("10", "0.1").error).toBe("Scale low must be below scale high.");
  });
});

const RA_ERROR = "Centre RA must be decimal degrees from 0 to 360, written with a dot (e.g. 190.03).";
const DEC_ERROR = "Centre Dec must be decimal degrees from -90 to 90, written with a dot (e.g. -11.63).";
const RADIUS_ERROR = "Radius must be decimal degrees above 0 and up to 180, written with a dot (e.g. 1.5).";
const NO_FLAGS = { raInvalid: false, decInvalid: false, radiusInvalid: false, error: null };

describe("centre and radius parsing", () => {
  it("rejects a decimal comma in the centre hint instead of truncating it", () => {
    expect(parsePositionHint("190,029137", "-11.633336", "")).toMatchObject({ centerRa: null, raInvalid: true });
    expect(parsePositionHint("190.029137", "-11,633336", "")).toMatchObject({ centerDec: null, decInvalid: true });
    expect(parseRadius("1,5")).toBeNull();
    expect(parsePositionHint("190.03deg", "-11.633336", "")).toMatchObject({ centerRa: null, raInvalid: true });
  });

  it("reads the dot form the header hint writes", () => {
    expect(parsePositionHint((190.029137).toFixed(6), " -11.633336 ", "0.0712")).toEqual({
      centerRa: 190.029137,
      centerDec: -11.633336,
      radius: 0.0712,
      ...NO_FLAGS,
    });
    expect(parseRadius("0.0712")).toBe(0.0712);
    expect(parsePositionHint("", "-11.633336", "")).toEqual({ centerRa: null, centerDec: null, radius: null, ...NO_FLAGS });
    expect(parseRadius("")).toBeNull();
  });

  it("accepts the centre Dec at both poles", () => {
    expect(parsePositionHint("190.5", "-90", "").centerDec).toBe(-90);
    expect(parsePositionHint("190.5", "90", "").centerDec).toBe(90);
  });
});

describe("parsePositionHint", () => {
  it("hints the solve with the header-filled centre and radius", () => {
    expect(parsePositionHint("190.029137", "-11.633133", "0.0712")).toEqual({
      centerRa: 190.029137,
      centerDec: -11.633133,
      radius: 0.0712,
      ...NO_FLAGS,
    });
  });

  it("solves blind without an error while a centre field is blank", () => {
    expect(parsePositionHint("", "", "")).toEqual({ centerRa: null, centerDec: null, radius: null, ...NO_FLAGS });
    expect(parsePositionHint("190.5", "", "")).toEqual({ centerRa: null, centerDec: null, radius: null, ...NO_FLAGS });
    expect(parsePositionHint("190.5", "-11.6", "")).toEqual({ centerRa: 190.5, centerDec: -11.6, radius: null, ...NO_FLAGS });
  });

  it("flags a decimal comma in the centre RA, names the accepted form and range, and sends no hint", () => {
    expect(parsePositionHint("190,5", "-11.6", "1.5")).toEqual({
      centerRa: null,
      centerDec: null,
      radius: null,
      raInvalid: true,
      decInvalid: false,
      radiusInvalid: false,
      error: RA_ERROR,
    });
  });

  it("flags a centre outside its range with the same message", () => {
    expect(parsePositionHint("400", "-11.6", "").error).toBe(RA_ERROR);
    expect(parsePositionHint("190.5", "95", "")).toEqual({
      centerRa: null,
      centerDec: null,
      radius: null,
      raInvalid: false,
      decInvalid: true,
      radiusInvalid: false,
      error: DEC_ERROR,
    });
    expect(parsePositionHint("190.5", "-11,6", "").decInvalid).toBe(true);
  });

  it("flags every invalid centre field and reports the first one", () => {
    const hint = parsePositionHint("190,5", "-11,6", "1,5");
    expect(hint.raInvalid).toBe(true);
    expect(hint.decInvalid).toBe(true);
    expect(hint.radiusInvalid).toBe(false);
    expect(hint.error).toBe(RA_ERROR);
  });

  it("checks the radius only when both centre fields hint the solve", () => {
    expect(parsePositionHint("", "", "1,5")).toEqual({ centerRa: null, centerDec: null, radius: null, ...NO_FLAGS });
    expect(parsePositionHint("190.5", "-11.6", "1,5")).toEqual({
      centerRa: 190.5,
      centerDec: -11.6,
      radius: null,
      raInvalid: false,
      decInvalid: false,
      radiusInvalid: true,
      error: RADIUS_ERROR,
    });
    expect(parsePositionHint("190.5", "-11.6", "0").error).toBe(RADIUS_ERROR);
    expect(parsePositionHint("190.5", "-11.6", "200").error).toBe(RADIUS_ERROR);
  });

  it("flags a negative centre RA, which the message says is outside 0 to 360", () => {
    expect(parsePositionHint("-10", "-11.6", "")).toEqual({
      centerRa: null,
      centerDec: null,
      radius: null,
      raInvalid: true,
      decInvalid: false,
      radiusInvalid: false,
      error: RA_ERROR,
    });
    expect(parsePositionHint("-0.5", "-11.6", "").error).toBe(RA_ERROR);
  });

  it("accepts the centre RA at both ends of 0 to 360", () => {
    expect(parsePositionHint("0", "-11.6", "").centerRa).toBe(0);
    expect(parsePositionHint("360", "-11.6", "").centerRa).toBe(360);
  });
});

const HINTED_STATUS = "Hinted solve around the field centre";
const BLIND_STATUS = "Blind solve: fill both centre fields to hint it";

describe("positionHintStatus", () => {
  it("names a hinted or a blind solve while every hint field is valid", () => {
    expect(positionHintStatus(parsePositionHint("190.5", "-11.6", "1.5"))).toBe(HINTED_STATUS);
    expect(positionHintStatus(parsePositionHint("190.5", "", ""))).toBe(BLIND_STATUS);
    expect(positionHintStatus(parsePositionHint("", "", ""))).toBe(BLIND_STATUS);
  });

  it("says neither blind nor hinted while the alert reports an invalid field and Solve is disabled", () => {
    expect(positionHintStatus(parsePositionHint("190,5", "-11.6", ""))).toBeNull();
    expect(positionHintStatus(parsePositionHint("190.5", "-11,6", ""))).toBeNull();
    expect(positionHintStatus(parsePositionHint("190.5", "-11.6", "1,5"))).toBeNull();
  });
});

describe("headerPositionHintTexts", () => {
  it("writes the header centre and half-diagonal radius with a dot", () => {
    expect(headerPositionHintTexts({ center_ra: 190.029137, center_dec: -11.633133, fov_arcmin: [6.4, 6.4] })).toEqual({
      ra: "190.029137",
      dec: "-11.633133",
      radius: "0.0754",
    });
  });

  it("writes every finite header RA inside 0 to 360, so the header fill never trips the RA check", () => {
    expect(headerPositionHintTexts({ center_ra: -0.5, center_dec: 10, fov_arcmin: [60, 45] })).toEqual({
      ra: "359.500000",
      dec: "10.000000",
      radius: "0.6250",
    });
    for (const headerRa of [-720.25, -360, -0.5, -1e-9, 0, 190.03, 359.9999999, 360, 725.5]) {
      const texts = headerPositionHintTexts({ center_ra: headerRa, center_dec: 10, fov_arcmin: [60, 45] });
      const hint = parsePositionHint(texts!.ra, texts!.dec, texts!.radius!);
      expect(hint.error, `header RA ${headerRa} written as ${texts!.ra}`).toBeNull();
      expect(hint.centerRa).not.toBeNull();
    }
  });

  it("leaves the radius out when the header field of view gives one the radius check rejects", () => {
    expect(headerPositionHintTexts({ center_ra: 190, center_dec: -11, fov_arcmin: [24000, 18000] })).toEqual({
      ra: "190.000000",
      dec: "-11.000000",
      radius: null,
    });
    expect(headerPositionHintTexts({ center_ra: 190, center_dec: -11, fov_arcmin: [0.003, 0.003] })?.radius).toBeNull();
  });

  it("gives no centre for a non-finite header centre and no radius for a non-finite field of view", () => {
    expect(headerPositionHintTexts({ center_ra: Number.NaN, center_dec: 0, fov_arcmin: [60, 45] })).toBeNull();
    expect(headerPositionHintTexts({ center_ra: 10, center_dec: Number.POSITIVE_INFINITY, fov_arcmin: [60, 45] })).toBeNull();
    expect(headerPositionHintTexts({ center_ra: 10, center_dec: 0, fov_arcmin: [Number.NaN, 45] })?.radius).toBeNull();
  });
});

const F200W_HEADER = { pixel_scale_arcsec: 0.031, center_ra: 190.029137, center_dec: -11.633133, fov_arcmin: [6.4, 6.4] as [number, number] };
const F200W_HINTS: HintTexts = {
  scaleLow: "0.0248",
  scaleHigh: "0.0388",
  scaleUnits: "arcsecperpix",
  centerRa: "190.029137",
  centerDec: "-11.633133",
  radius: "0.0754",
};
const NOTHING_TOUCHED: ReadonlySet<HintField> = new Set();

describe("headerHintTexts", () => {
  it("fills the scale range in arcsec/px and the centre and radius from the header WCS", () => {
    expect(headerHintTexts(F200W_HEADER)).toEqual(F200W_HINTS);
  });

  it("gives no scale hint without a usable pixel scale and no radius without a usable field of view", () => {
    expect(headerHintTexts({ ...F200W_HEADER, pixel_scale_arcsec: 0 })).toEqual({
      centerRa: "190.029137",
      centerDec: "-11.633133",
      radius: "0.0754",
    });
    expect(headerHintTexts({ ...F200W_HEADER, fov_arcmin: [Number.NaN, 6.4] })).toEqual({
      scaleLow: "0.0248",
      scaleHigh: "0.0388",
      scaleUnits: "arcsecperpix",
      centerRa: "190.029137",
      centerDec: "-11.633133",
    });
    expect(headerHintTexts({ ...F200W_HEADER, pixel_scale_arcsec: Number.NaN, center_ra: Number.NaN })).toEqual({});
  });
});

describe("mergeHintTexts", () => {
  it("fills every field from the header while the user has typed nothing", () => {
    expect(mergeHintTexts(DEFAULT_HINT_TEXTS, F200W_HINTS, NOTHING_TOUCHED)).toEqual(F200W_HINTS);
  });

  it("keeps a typed scale low when a later publish brings the header hints back", () => {
    const typed: HintTexts = { ...F200W_HINTS, scaleLow: "0.05" };
    expect(mergeHintTexts(typed, F200W_HINTS, new Set<HintField>(["scaleLow"]))).toEqual(typed);
  });

  it("keeps every typed field and the scale range that goes with typed units, and refreshes the rest", () => {
    const current: HintTexts = {
      scaleLow: "1",
      scaleHigh: "2",
      scaleUnits: "arcminwidth",
      centerRa: "10.5",
      centerDec: "",
      radius: "3",
    };
    const touched = new Set<HintField>(["scaleUnits", "centerRa", "radius"]);
    expect(mergeHintTexts(current, F200W_HINTS, touched)).toEqual({
      scaleLow: "1",
      scaleHigh: "2",
      scaleUnits: "arcminwidth",
      centerRa: "10.5",
      centerDec: "-11.633133",
      radius: "3",
    });
  });

  it("keeps the default range in the units the user picked when a solved header with a new pixel scale is published", () => {
    const current: HintTexts = { ...DEFAULT_HINT_TEXTS, scaleUnits: "degwidth" };
    const solved = headerHintTexts({ ...F200W_HEADER, pixel_scale_arcsec: 1.8 });
    expect(solved).toMatchObject({ scaleLow: "1.44", scaleHigh: "2.25", scaleUnits: "arcsecperpix" });
    expect(mergeHintTexts(current, solved, new Set<HintField>(["scaleUnits"]))).toEqual({
      ...F200W_HINTS,
      scaleLow: DEFAULT_SCALE_LOW_TEXT,
      scaleHigh: DEFAULT_SCALE_HIGH_TEXT,
      scaleUnits: "degwidth",
    });
  });

  it("keeps the whole typed scale range when only one bound was typed and the pixel scale changes", () => {
    const typed: HintTexts = { ...F200W_HINTS, scaleLow: "0.02" };
    const longWave = headerHintTexts({ ...F200W_HEADER, pixel_scale_arcsec: 0.063 });
    expect(longWave.scaleHigh).not.toBe(F200W_HINTS.scaleHigh);
    expect(mergeHintTexts(typed, longWave, new Set<HintField>(["scaleLow"]))).toEqual(typed);
  });

  it("leaves a field the header gives no hint for as it is", () => {
    const current: HintTexts = { ...DEFAULT_HINT_TEXTS, radius: "2" };
    const hints = headerHintTexts({ ...F200W_HEADER, fov_arcmin: [Number.NaN, 6.4] });
    expect(mergeHintTexts(current, hints, NOTHING_TOUCHED)).toEqual({ ...F200W_HINTS, radius: "2" });
  });

  it("does not change the current texts it was given", () => {
    const current: HintTexts = { ...DEFAULT_HINT_TEXTS };
    mergeHintTexts(current, F200W_HINTS, NOTHING_TOUCHED);
    expect(current).toEqual(DEFAULT_HINT_TEXTS);
  });
});
