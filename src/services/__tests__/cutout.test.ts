import { describe, it, expect } from "vitest";
import { cutoutDefaultFileName, describeCutout, manualCutoutBox, selectedBoxRegion } from "../cutout";
import type { CutoutExportResult } from "../../shared/types/cutout";

const base = { centreX: 100.5, centreY: 40, width: 20, height: 10 };

describe("manualCutoutBox", () => {
  it("builds an axis-aligned pixel box from a pixel-sized form", () => {
    expect(manualCutoutBox({ ...base, unit: "px", pixelScaleArcsec: null })).toEqual({
      shape: "box", x: 100.5, y: 40, width: 20, height: 10, angle: 0,
    });
  });

  it("converts arcsecond sizes through the pixel scale and keeps the centre in pixels", () => {
    const box = manualCutoutBox({ ...base, unit: "arcsec", pixelScaleArcsec: 0.5 });
    expect(box).toEqual({ shape: "box", x: 100.5, y: 40, width: 40, height: 20, angle: 0 });
  });

  it("refuses arcsecond sizes without a usable pixel scale", () => {
    expect(manualCutoutBox({ ...base, unit: "arcsec", pixelScaleArcsec: null })).toBeNull();
    expect(manualCutoutBox({ ...base, unit: "arcsec", pixelScaleArcsec: 0 })).toBeNull();
    expect(manualCutoutBox({ ...base, unit: "arcsec", pixelScaleArcsec: Number.NaN })).toBeNull();
  });

  it("refuses non-finite centres and non-positive sizes", () => {
    expect(manualCutoutBox({ ...base, centreX: Number.NaN, unit: "px", pixelScaleArcsec: null })).toBeNull();
    expect(manualCutoutBox({ ...base, width: 0, unit: "px", pixelScaleArcsec: null })).toBeNull();
    expect(manualCutoutBox({ ...base, height: -3, unit: "px", pixelScaleArcsec: null })).toBeNull();
  });
});

describe("selectedBoxRegion", () => {
  const regions = [
    { id: "a", shape: { shape: "circle", x: 1, y: 1, r: 2 } as const },
    { id: "b", shape: { shape: "box", x: 5, y: 6, width: 7, height: 8, angle: 0 } as const },
  ];

  it("returns the selected region only when it is a box", () => {
    expect(selectedBoxRegion(regions, "b")).toEqual(regions[1].shape);
    expect(selectedBoxRegion(regions, "a")).toBeNull();
    expect(selectedBoxRegion(regions, "missing")).toBeNull();
    expect(selectedBoxRegion(regions, null)).toBeNull();
  });
});

describe("describeCutout and cutoutDefaultFileName", () => {
  const result: CutoutExportResult = {
    output_path: "C:/out/x.fits",
    rect: { x0: -2, y0: 30, width: 40, height: 12 },
    fraction_on_image: 20 / 36,
    hdus: ["SCI", "ERR", "DQ"],
    ltv1: 2,
    ltv2: -30,
    rotated_box_used_bounds: false,
    warnings: [],
    elapsed_ms: 3,
  };

  it("summarises the rect, coverage and extensions", () => {
    expect(describeCutout(result)).toBe("40x12 px at (-2, 30), 56% on image, HDUs SCI+ERR+DQ");
  });

  it("derives the default file name from the source stem without the plane suffix", () => {
    expect(cutoutDefaultFileName("C:\\data\\jw01234_cal.fits#hdu=1")).toBe("jw01234_cal_cutout.fits");
    expect(cutoutDefaultFileName("/d/r0000.asdf#array=roman.dq")).toBe("r0000_cutout.fits");
    expect(cutoutDefaultFileName("/d/frame.zip")).toBe("frame_cutout.fits");
    expect(cutoutDefaultFileName("")).toBe("image_cutout.fits");
  });

  it("strips compressed source extensions from the stem", () => {
    expect(cutoutDefaultFileName("/d/jw01234_cal.fits.fz")).toBe("jw01234_cal_cutout.fits");
    expect(cutoutDefaultFileName("C:\\d\\hst_flt.fits.gz#hdu=1")).toBe("hst_flt_cutout.fits");
    expect(cutoutDefaultFileName("/d/frame.fit.gz")).toBe("frame_cutout.fits");
    expect(cutoutDefaultFileName("/d/plain.gz")).toBe("plain_cutout.fits");
    expect(cutoutDefaultFileName("/d/notes.fitsx")).toBe("notes.fitsx_cutout.fits");
  });
});
