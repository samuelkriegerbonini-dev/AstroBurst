import { describe, expect, it } from "vitest";
import { rgbProductOf, rgbProductPath, rgbViewOf, type RgbProductInput } from "../rgbProduct";
import type { ProcessedResult } from "../../shared/types/preview";

const RGB = "C:/data/runtime-rgb.fits";
const RGB_PNG = "asset://localhost/runtime-rgb.png";
const CURVES = "C:/history/344da451b072ae3a/n0001.fits";
const BLEND_PNG = "asset://localhost/rgb_composite_blend.png";

const CURVES_RESULT: ProcessedResult = {
  fitsPath: CURVES,
  previewUrl: "asset://localhost/n0001.png?v=2",
  dimensions: [128, 128],
  label: "Curves",
  kind: "processing",
  inputPath: RGB,
  planes: 3,
  domain: "display_referred",
};

const RGB_FILE: RgbProductInput = { compositePreviewUrl: RGB_PNG, fileIsRgb: true, filePath: RGB, filePreviewUrl: RGB_PNG, processed: null };
const MONO_FILE: RgbProductInput = { compositePreviewUrl: null, fileIsRgb: false, filePath: "C:/data/m16.fits", filePreviewUrl: "asset://localhost/m16.png", processed: null };

describe("rgbProductOf", () => {
  it("is the RGB file itself while nothing is processed, with or without its RGB view seeded", () => {
    expect(rgbProductOf(RGB_FILE)).toEqual({ kind: "file", path: RGB });
    expect(rgbProductOf({ ...RGB_FILE, compositePreviewUrl: null })).toEqual({ kind: "file", path: RGB });
  });

  it("is the processed 3-plane result once one is shown, before and after the file's RGB view is reset", () => {
    const expected = { kind: "result", path: CURVES, label: "Curves", displayReferred: true };
    expect(rgbProductOf({ ...RGB_FILE, compositePreviewUrl: null, processed: CURVES_RESULT })).toEqual(expected);
    expect(rgbProductOf({ ...RGB_FILE, processed: CURVES_RESULT })).toEqual(expected);
  });

  it("tells a linear 3-plane result from a display-referred one", () => {
    const linear = rgbProductOf({ ...RGB_FILE, compositePreviewUrl: null, processed: { ...CURVES_RESULT, domain: "linear" } });
    expect(linear).toMatchObject({ kind: "result", displayReferred: false });
  });

  it("is no RGB product while a mono, PNG-only or plane-less result is shown", () => {
    const over = (processed: ProcessedResult) => rgbProductOf({ ...RGB_FILE, compositePreviewUrl: null, processed });
    expect(over({ ...CURVES_RESULT, planes: 1 })).toBeNull();
    expect(over({ ...CURVES_RESULT, fitsPath: null })).toBeNull();
    expect(over({ ...CURVES_RESULT, planes: undefined, domain: undefined })).toBeNull();
    expect(rgbProductOf({ ...MONO_FILE, processed: { ...CURVES_RESULT, planes: 1 } })).toBeNull();
  });

  it("is the composite whenever a Compose composite is on screen", () => {
    expect(rgbProductOf({ ...RGB_FILE, compositePreviewUrl: BLEND_PNG })).toEqual({ kind: "composite" });
    expect(rgbProductOf({ ...RGB_FILE, compositePreviewUrl: BLEND_PNG, processed: CURVES_RESULT })).toEqual({ kind: "composite" });
    expect(rgbProductOf({ ...MONO_FILE, compositePreviewUrl: BLEND_PNG })).toEqual({ kind: "composite" });
  });

  it("is no RGB product for a mono image or a science cube", () => {
    expect(rgbProductOf(MONO_FILE)).toBeNull();
    expect(rgbProductOf({ ...MONO_FILE, filePath: "C:/data/jw01266_s3d.fits" })).toBeNull();
  });
});

describe("rgbViewOf", () => {
  it("waits for the file's RGB view to be seeded before showing the RGB file", () => {
    expect(rgbViewOf(RGB_FILE)).toEqual({ kind: "file", path: RGB });
    expect(rgbViewOf({ ...RGB_FILE, compositePreviewUrl: null })).toBeNull();
  });

  it("shows the processed RGB result and the composite like the exporters resolve them", () => {
    for (const input of [
      { ...RGB_FILE, compositePreviewUrl: null, processed: CURVES_RESULT },
      { ...RGB_FILE, processed: CURVES_RESULT },
      { ...RGB_FILE, compositePreviewUrl: BLEND_PNG },
      MONO_FILE,
    ]) {
      expect(rgbViewOf(input)).toEqual(rgbProductOf(input));
    }
  });
});

describe("rgbProductPath", () => {
  it("gives the FITS the RGB loaders read, and null for the in-memory composite", () => {
    expect(rgbProductPath({ kind: "file", path: RGB })).toBe(RGB);
    expect(rgbProductPath({ kind: "result", path: CURVES, label: "Curves", displayReferred: true })).toBe(CURVES);
    expect(rgbProductPath({ kind: "composite" })).toBeNull();
    expect(rgbProductPath(null)).toBeNull();
  });
});
