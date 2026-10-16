import { describe, expect, it } from "vitest";
import { fieldText, orientationText, pixelScaleText, scaleBar } from "../logic/quickWcs";
import type { MastQuickWcs } from "../../../shared/types/mast";

const L3: MastQuickWcs = {
  centreRaDeg: 274.7,
  centreDecDeg: -13.85,
  pixelScaleArcsec: 0.031,
  fieldArcmin: [7.41, 4.46],
  orientation: "unreliable",
  ctype: "RA---TAN",
};

const L2: MastQuickWcs = { ...L3, pixelScaleArcsec: 0.0628, fieldArcmin: [2.14, 2.14], orientation: "cd_matrix" };

describe("pixelScaleText", () => {
  it("prints arcseconds per pixel with three decimals below 1\"", () => {
    expect(pixelScaleText(L3)).toBe('0.031"/px');
    expect(pixelScaleText(L2)).toBe('0.063"/px');
    expect(pixelScaleText({ ...L3, pixelScaleArcsec: 0.04 })).toBe('0.040"/px');
  });

  it("uses fewer decimals for coarse scales", () => {
    expect(pixelScaleText({ ...L3, pixelScaleArcsec: 1.254 })).toBe('1.25"/px');
    expect(pixelScaleText({ ...L3, pixelScaleArcsec: 21.04 })).toBe('21.0"/px');
  });
});

describe("fieldText", () => {
  it("prints the field in arcminutes with one decimal", () => {
    expect(fieldText(L3)).toBe("field 7.4' x 4.5'");
  });

  it("switches to arcseconds below one arcminute", () => {
    expect(fieldText({ ...L3, fieldArcmin: [0.75, 0.5] })).toBe('field 45" x 30"');
  });
});

describe("orientationText", () => {
  it("states an orientation only for a CD matrix in the pane, and says it is not given in the Large view rail", () => {
    expect(orientationText(L2, "pane")).toBe("orientation from the CD matrix");
    expect(orientationText(L3, "pane")).toBeNull();
    expect(orientationText(L3, "rail")).toBe("orientation not given");
    expect(orientationText(L2, "rail")).toBe("orientation from the CD matrix");
  });
});

describe("scaleBar", () => {
  it("picks a 1-2-5 length that fits the target width", () => {
    expect(scaleBar(0.5, 160)).toEqual({ px: 120, label: "1'" });
    expect(scaleBar(0.031 * 2, 160)).toEqual({ px: 80.64516129032258, label: '5"' });
    expect(scaleBar(3, 160)).toEqual({ px: 100, label: "5'" });
  });

  it("uses degrees for very coarse scales and arcseconds for fine ones", () => {
    expect(scaleBar(60, 160)).toEqual({ px: 120, label: "2°" });
    expect(scaleBar(0.005, 160)).toEqual({ px: 100, label: '0.5"' });
  });

  it("returns null when the scale is unusable", () => {
    expect(scaleBar(0, 160)).toBeNull();
    expect(scaleBar(Number.NaN, 160)).toBeNull();
  });
});
