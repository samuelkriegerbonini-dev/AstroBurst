import { describe, expect, it } from "vitest";
import {
  STF_LOCK_PNG,
  STF_LOCK_RGB,
  STF_LOCK_STRETCH,
  analysisMeasureKey,
  cpuStfRenderAllowed,
  describeMeasurementSource,
  detectedStarsOnMeasuredImage,
  histogramStfLock,
  isCompositeOnScreen,
  isFileRgbView,
  rgbMeasurePath,
  rgbStfPanelMode,
} from "../analysisTarget";

describe("isFileRgbView", () => {
  it("is true when the RGB view on screen is the selected RGB file's own view, whose channels the global slot may not hold", () => {
    expect(isFileRgbView({ compositePreviewUrl: "asset://a.png", fileIsRgb: true, filePreviewUrl: "asset://a.png" })).toBe(true);
  });

  it("is false for a wizard composite, even over an RGB file", () => {
    expect(isFileRgbView({ compositePreviewUrl: "asset://blend.png", fileIsRgb: true, filePreviewUrl: "asset://a.png" })).toBe(false);
    expect(isFileRgbView({ compositePreviewUrl: "asset://blend.png", fileIsRgb: false, filePreviewUrl: "asset://l.png" })).toBe(false);
  });

  it("is false with no composite preview, even when both preview URLs are missing", () => {
    expect(isFileRgbView({ compositePreviewUrl: null, fileIsRgb: true, filePreviewUrl: null })).toBe(false);
  });
});

describe("isCompositeOnScreen", () => {
  const rgbFile = { fileIsRgb: true, filePreviewUrl: "asset://m31.png" };

  it("is false without a composite preview", () => {
    expect(isCompositeOnScreen({ compositePreviewUrl: null, fileIsRgb: false, filePreviewUrl: null, hasProcessed: false })).toBe(false);
  });

  it("is true for an RGB FITS shown through its own composite view", () => {
    expect(isCompositeOnScreen({ compositePreviewUrl: "asset://m31.png", ...rgbFile, hasProcessed: false })).toBe(true);
  });

  it("is false when an RGB FITS shows a mono processed result instead of its RGB view", () => {
    expect(isCompositeOnScreen({ compositePreviewUrl: "asset://m31.png", ...rgbFile, hasProcessed: true })).toBe(false);
  });

  it("stays true for a wizard composite even when the selected file has a processed result", () => {
    expect(
      isCompositeOnScreen({ compositePreviewUrl: "asset://blend.png", fileIsRgb: false, filePreviewUrl: "asset://l.png", hasProcessed: true }),
    ).toBe(true);
  });
});

describe("describeMeasurementSource", () => {
  const base = {
    measuresComposite: false,
    compositeOnScreen: false,
    measuresFilePlanes: false,
    fileName: "m31.fits",
    processedLabel: null,
    previewOnly: false,
  };

  it("says nothing for the unprocessed original", () => {
    expect(describeMeasurementSource(base)).toBeNull();
  });

  it("names the processed result when measurements run on it", () => {
    const source = describeMeasurementSource({ ...base, processedLabel: "Background" });
    expect(source?.text).toBe("Background");
    expect(source?.tone).toBe("processed");
  });

  it("says the original is measured when the processed result is PNG-only", () => {
    const source = describeMeasurementSource({ ...base, processedLabel: "Cube frame 3", previewOnly: true });
    expect(source?.text).toBe("original");
    expect(source?.title).toContain("PNG-only");
  });

  it("names the selected RGB file when its own planes are measured by path", () => {
    const source = describeMeasurementSource({ ...base, compositeOnScreen: true, measuresComposite: true, measuresFilePlanes: true });
    expect(source?.text).toBe("RGB file");
    expect(source?.title).toContain("planes of m31.fits");
  });

  it("names the wizard composite when the RGB view on screen is a Blend", () => {
    const source = describeMeasurementSource({ ...base, compositeOnScreen: true, measuresComposite: true });
    expect(source?.text).toBe("composite");
    expect(source?.tone).toBe("composite");
    expect(source?.title).not.toContain("last RGB FITS loaded");
  });

  it("says a per-file measurement ignores the RGB view on screen", () => {
    const source = describeMeasurementSource({ ...base, compositeOnScreen: true, processedLabel: "Background" });
    expect(source?.text).toBe("selected file");
    expect(source?.tone).toBe("original");
  });
});

describe("rgbMeasurePath", () => {
  it("measures the selected RGB FITS by its own path while its RGB view is on screen", () => {
    expect(rgbMeasurePath({ composite: true, fileRgbView: true, filePath: "D:/M31_osc.fits" })).toBe("D:/M31_osc.fits");
  });

  it("uses the composite slot for a wizard Blend, and nothing when no RGB view is on screen", () => {
    expect(rgbMeasurePath({ composite: true, fileRgbView: false, filePath: "D:/M31_osc.fits" })).toBeNull();
    expect(rgbMeasurePath({ composite: false, fileRgbView: true, filePath: "D:/M31_osc.fits" })).toBeNull();
  });
});

describe("detectedStarsOnMeasuredImage", () => {
  it("refuses stars detected on a wizard composite for a table that measures the selected file", () => {
    expect(detectedStarsOnMeasuredImage({ compositeOnScreen: true, measuresFilePlanes: false })).toBe(false);
  });

  it("keeps stars detected on the planes of the RGB file the table measures", () => {
    expect(detectedStarsOnMeasuredImage({ compositeOnScreen: true, measuresFilePlanes: true })).toBe(true);
  });

  it("keeps stars detected on the mono image the table measures", () => {
    expect(detectedStarsOnMeasuredImage({ compositeOnScreen: false, measuresFilePlanes: false })).toBe(true);
  });
});

describe("analysisMeasureKey", () => {
  it("changes when a step re-publishes the same FITS path", () => {
    const first = analysisMeasureKey({ path: "out/R_arcsinh.fits", composite: false, processedFitsPath: "out/R_arcsinh.fits", processedVersion: 1 });
    const second = analysisMeasureKey({ path: "out/R_arcsinh.fits", composite: false, processedFitsPath: "out/R_arcsinh.fits", processedVersion: 2 });
    expect(second).not.toBe(first);
  });

  it("stays stable across PNG-only publishes and while the composite is measured", () => {
    expect(analysisMeasureKey({ path: "cube.fits", composite: false, processedFitsPath: null, processedVersion: 3 })).toBe(
      analysisMeasureKey({ path: "cube.fits", composite: false, processedFitsPath: null, processedVersion: 4 }),
    );
    expect(analysisMeasureKey({ path: "rgb.fits", composite: true, processedFitsPath: "rgb.fits", processedVersion: 5 })).toBe(
      analysisMeasureKey({ path: "rgb.fits", composite: true, processedFitsPath: "rgb.fits", processedVersion: 6 }),
    );
  });

  it("is null without a path", () => {
    expect(analysisMeasureKey({ path: null, composite: false, processedFitsPath: null, processedVersion: 7 })).toBeNull();
  });
});

describe("histogramStfLock", () => {
  it("unlocks the mtf stretch on a mono view", () => {
    expect(histogramStfLock({ stretch: "mtf", compositeOnScreen: false, previewOnly: false })).toBeNull();
  });

  it("locks the mono histogram while an RGB view is on screen", () => {
    expect(histogramStfLock({ stretch: "mtf", compositeOnScreen: true, previewOnly: false })).toBe(STF_LOCK_RGB);
  });

  it("locks the sliders on a PNG-only result", () => {
    expect(histogramStfLock({ stretch: "mtf", compositeOnScreen: false, previewOnly: true })).toBe(STF_LOCK_PNG);
  });

  it("keeps the non-mtf stretch lock", () => {
    expect(histogramStfLock({ stretch: "asinh", compositeOnScreen: false, previewOnly: false })).toBe(STF_LOCK_STRETCH);
  });
});

describe("cpuStfRenderAllowed", () => {
  it("renders on the CPU when no raw pixels exist and none are loading", () => {
    expect(cpuStfRenderAllowed({ hasRawPixels: false, rawPixelsLoading: false, locked: false })).toBe(true);
  });

  it("does not render while GPU raw pixels are still loading", () => {
    expect(cpuStfRenderAllowed({ hasRawPixels: false, rawPixelsLoading: true, locked: false })).toBe(false);
  });

  it("does not render when the GPU already draws the image or the sliders are locked", () => {
    expect(cpuStfRenderAllowed({ hasRawPixels: true, rawPixelsLoading: false, locked: false })).toBe(false);
    expect(cpuStfRenderAllowed({ hasRawPixels: false, rawPixelsLoading: false, locked: true })).toBe(false);
  });
});

describe("rgbStfPanelMode", () => {
  it("offers live channel STF on a linear GPU composite", () => {
    expect(rgbStfPanelMode({ compositeOnScreen: true, hasRgbRawPixels: true, displayReferred: false })).toBe("live");
  });

  it("does not offer live sliders once the composite is display-referred", () => {
    expect(rgbStfPanelMode({ compositeOnScreen: true, hasRgbRawPixels: true, displayReferred: true })).toBe("baked");
  });

  it("is hidden without a composite on screen or without GPU pixels", () => {
    expect(rgbStfPanelMode({ compositeOnScreen: false, hasRgbRawPixels: true, displayReferred: false })).toBe("hidden");
    expect(rgbStfPanelMode({ compositeOnScreen: true, hasRgbRawPixels: false, displayReferred: false })).toBe("hidden");
  });
});
