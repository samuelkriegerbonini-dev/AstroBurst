import { describe, expect, it } from "vitest";
import {
  MAG_ERR_PER_RELATIVE_FLUX_ERR,
  isUndersampled,
  radiiRequest,
  radiusInArcsec,
  radiusInPixels,
  skyLevelError,
  systemMagnitude,
} from "../photometryUnits";

const NO_SCALE_ERROR =
  "Radii in arcsec need the pixel scale of a celestial WCS, which this image does not have. Switch the unit to px.";

const PLAIN_ZERO_POINT = { zp: 25, exptimeS: null, exptimeApplied: false };

describe("radiusInPixels and radiusInArcsec", () => {
  it("converts one arcsec on a 0.262 arcsec pixel to the radius the depth golden uses", () => {
    expect(radiusInPixels(1, "arcsec", 0.262)).toBeCloseTo(3.8167938931297707, 12);
  });

  it("leaves a pixel radius alone and needs no scale for it", () => {
    expect(radiusInPixels(5, "px", null)).toBe(5);
  });

  it("returns null without a usable scale or for a radius that is not positive", () => {
    expect(radiusInPixels(1, "arcsec", null)).toBeNull();
    expect(radiusInPixels(1, "arcsec", 0)).toBeNull();
    expect(radiusInPixels(1, "arcsec", Number.NaN)).toBeNull();
    for (const unit of ["px", "arcsec"] as const) {
      expect(radiusInPixels(0, unit, 0.262)).toBeNull();
      expect(radiusInPixels(-1, unit, 0.262)).toBeNull();
      expect(radiusInPixels(Number.NaN, unit, 0.262)).toBeNull();
    }
  });

  it("shows a pixel radius in arcsec", () => {
    expect(radiusInArcsec(5, 0.262)).toBeCloseTo(1.31, 12);
    expect(radiusInArcsec(4.8, 0.031)).toBeCloseTo(0.1488, 12);
    expect(radiusInArcsec(null, 0.262)).toBeNull();
    expect(radiusInArcsec(5, null)).toBeNull();
  });

  it("round trips", () => {
    for (const a of [0.5, 1, 2, 3.5, 7]) {
      for (const s of [0.031, 0.1, 0.262, 0.39603565421543135]) {
        const back = radiusInArcsec(radiusInPixels(a, "arcsec", s), s);
        expect(back).not.toBeNull();
        expect(Math.abs((back as number) - a)).toBeLessThanOrEqual(1e-12);
      }
    }
  });
});

describe("radiiRequest", () => {
  it("passes pixel radii through under the service names", () => {
    expect(radiiRequest({ aperture: 5, skyInner: 10, skyOuter: 15 }, "px", null)).toEqual({
      radii: { apertureRadius: 5, annulusInner: 10, annulusOuter: 15 },
      error: null,
    });
    expect(radiiRequest({}, "px", 0.262)).toEqual({ radii: {}, error: null });
  });

  it("converts the three radii with one scale", () => {
    const request = radiiRequest({ aperture: 1, skyInner: 2, skyOuter: 3 }, "arcsec", 0.262);
    expect(request.error).toBeNull();
    expect(request.radii.apertureRadius).toBeCloseTo(3.8167938931297707, 12);
    expect(request.radii.annulusInner).toBeCloseTo(7.633587786259541, 12);
    expect(request.radii.annulusOuter).toBeCloseTo(11.450381679389313, 12);
  });

  it("keeps blank members blank in arcsec", () => {
    const request = radiiRequest({ aperture: 2 }, "arcsec", 0.263);
    expect(Object.keys(request.radii)).toEqual(["apertureRadius"]);
    expect(request.radii.apertureRadius).toBeCloseTo(7.604562737642585, 12);
    expect(request.error).toBeNull();
  });

  it("refuses arcsec radii on an image without a pixel scale", () => {
    expect(radiiRequest({ aperture: 1 }, "arcsec", null)).toEqual({ radii: {}, error: NO_SCALE_ERROR });
    expect(radiiRequest({}, "arcsec", null).error).toBeNull();
  });

  it("names an aperture that leaves the 2 to 60 px limits, with the limits in arcsec", () => {
    expect(radiiRequest({ aperture: 7 }, "arcsec", 0.1).error).toBe(
      "Aperture radius 7 arcsec is 70.00 px at 0.1000 arcsec/px; it must be between 2 and 60 px (0.2000 to 6.000 arcsec).",
    );
    expect(radiiRequest({ aperture: 0.19 }, "arcsec", 0.1).error).toBe(
      "Aperture radius 0.19 arcsec is 1.90 px at 0.1000 arcsec/px; it must be between 2 and 60 px (0.2000 to 6.000 arcsec).",
    );
    const upper = radiiRequest({ aperture: 6 }, "arcsec", 0.1);
    expect(upper.error).toBeNull();
    expect(upper.radii.apertureRadius).toBeCloseTo(60, 12);
    const lower = radiiRequest({ aperture: 0.2 }, "arcsec", 0.1);
    expect(lower.error).toBeNull();
    expect(lower.radii.apertureRadius).toBeCloseTo(2, 12);
  });

  it("names a sky annulus that leaves its limits", () => {
    expect(radiiRequest({ skyOuter: 60 }, "arcsec", 0.1).error).toBe(
      "Sky outer radius 60 arcsec is 600.0 px at 0.1000 arcsec/px; it must be at most 512 px (51.20 arcsec).",
    );
    expect(radiiRequest({ skyInner: 0.05 }, "arcsec", 0.1).error).toBe(
      "Sky inner radius 0.05 arcsec is 0.50 px at 0.1000 arcsec/px; it must be at least 1 px (0.1000 arcsec).",
    );
  });

  it("reports the aperture before the annulus when both fail", () => {
    const request = radiiRequest({ aperture: 7, skyOuter: 60 }, "arcsec", 0.1);
    expect(request.error?.startsWith("Aperture radius")).toBe(true);
    expect(request.radii).toEqual({});
  });
});

describe("isUndersampled", () => {
  it("flags a width below two pixels and nothing else", () => {
    expect(isUndersampled(1.66)).toBe(true);
    expect(isUndersampled(1.999)).toBe(true);
    expect(isUndersampled(2)).toBe(false);
    expect(isUndersampled(3.2)).toBe(false);
    expect(isUndersampled(null)).toBe(false);
    expect(isUndersampled(undefined)).toBe(false);
    expect(isUndersampled(Number.NaN)).toBe(false);
  });
});

describe("skyLevelError", () => {
  it("divides the pixel scatter by the square root of the pixel count", () => {
    expect(skyLevelError(1.5, 300)).toBeCloseTo(0.08660254037844385, 12);
    expect(skyLevelError(2, 200)).toBeCloseTo(0.1414213562373095, 12);
  });

  it("returns null without pixels or with a scatter that is not a number", () => {
    expect(skyLevelError(1.5, 0)).toBeNull();
    expect(skyLevelError(Number.NaN, 10)).toBeNull();
    expect(skyLevelError(null, 10)).toBeNull();
    expect(skyLevelError(1.5, null)).toBeNull();
    expect(skyLevelError(-1, 10)).toBeNull();
    expect(skyLevelError(0, 10)).toBe(0);
  });
});

describe("systemMagnitude", () => {
  it("adds the zero point to the instrumental magnitude", () => {
    expect(systemMagnitude(-7.5, 1000, null, PLAIN_ZERO_POINT)).toEqual({ mag: 17.5, magErr: null });
  });

  it("adds the exposure term of a per-second zero point on counts", () => {
    const result = systemMagnitude(-7.5, 1000, null, { zp: 25, exptimeS: 600, exptimeApplied: true });
    expect(result?.mag).toBeCloseTo(24.44537812595911, 12);
  });

  it("ignores an exposure time that was not applied", () => {
    expect(systemMagnitude(-7.5, 1000, null, { zp: 25, exptimeS: 600, exptimeApplied: false })?.mag).toBe(17.5);
  });

  it("reproduces the worked case of a Gaia zero point without a colour term", () => {
    const result = systemMagnitude(-9.87, 8872, 50, { zp: 21.432, exptimeS: null, exptimeApplied: false });
    expect(result?.mag).toBeCloseTo(11.562, 12);
  });

  it("propagates the relative flux error", () => {
    const result = systemMagnitude(-7.728727735669878, 1234.5, 12.3, PLAIN_ZERO_POINT);
    expect(result?.magErr).toBeCloseTo(0.010817784786168483, 14);
  });

  it("has no magnitude for a flux that is not positive or a zero point that is not finite", () => {
    expect(systemMagnitude(null, 1000, 10, PLAIN_ZERO_POINT)).toBeNull();
    expect(systemMagnitude(undefined, 1000, 10, PLAIN_ZERO_POINT)).toBeNull();
    expect(systemMagnitude(Number.NaN, 1000, 10, PLAIN_ZERO_POINT)).toBeNull();
    expect(systemMagnitude(-7.5, 1000, 10, { zp: Number.NaN, exptimeS: null, exptimeApplied: false })).toBeNull();
    expect(systemMagnitude(-7.5, 1000, 10, { zp: 25, exptimeS: null, exptimeApplied: true })).toBeNull();
    expect(systemMagnitude(-7.5, 1000, 10, { zp: 25, exptimeS: 0, exptimeApplied: true })).toBeNull();
    expect(systemMagnitude(-7.5, -3, 10, PLAIN_ZERO_POINT)).toEqual({ mag: 17.5, magErr: null });
    expect(systemMagnitude(-7.5, 1000, null, PLAIN_ZERO_POINT)).toEqual({ mag: 17.5, magErr: null });
  });

  it("uses the magnitude error constant of the backend", () => {
    expect(Math.abs(MAG_ERR_PER_RELATIVE_FLUX_ERR - 2.5 / Math.LN10)).toBeLessThanOrEqual(1e-15);
    expect(Math.abs(MAG_ERR_PER_RELATIVE_FLUX_ERR - 1.085736204758129)).toBeLessThanOrEqual(1e-15);
  });
});
