import { describe, it, expect } from "vitest";
import { isCompositeMode } from "../useCompositeMode";

describe("isCompositeMode", () => {
  it("is false with no composite on screen", () => {
    expect(isCompositeMode({ compositePreviewUrl: null, fileIsRgb: false, filePreviewUrl: "asset://l.png" })).toBe(false);
    expect(isCompositeMode({ compositePreviewUrl: null, fileIsRgb: true, filePreviewUrl: null })).toBe(false);
  });

  it("is true for a wizard composite over a mono file", () => {
    expect(isCompositeMode({ compositePreviewUrl: "asset://blend.png", fileIsRgb: false, filePreviewUrl: "asset://l.png" })).toBe(true);
  });

  it("is true for a wizard composite over an RGB file whose own view is not on screen", () => {
    expect(isCompositeMode({ compositePreviewUrl: "asset://blend.png", fileIsRgb: true, filePreviewUrl: "asset://m31.png" })).toBe(true);
  });

  it("is false on an RGB file's own colour view, so its panels run in file mode", () => {
    expect(isCompositeMode({ compositePreviewUrl: "asset://m31.png", fileIsRgb: true, filePreviewUrl: "asset://m31.png" })).toBe(false);
  });

  it("is false once an RGB file shows a processed result and the composite preview is gone", () => {
    expect(isCompositeMode({ compositePreviewUrl: null, fileIsRgb: true, filePreviewUrl: "asset://m31.png" })).toBe(false);
  });

  it("is true for a mono file whose preview happens to match a composite URL only when the file is RGB", () => {
    expect(isCompositeMode({ compositePreviewUrl: "asset://same.png", fileIsRgb: false, filePreviewUrl: "asset://same.png" })).toBe(true);
  });
});
