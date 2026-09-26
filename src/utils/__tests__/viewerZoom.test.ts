import { describe, it, expect } from "vitest";
import {
  FIT_SCALE_CAP,
  VIEWER_ZOOM_MAX,
  VIEWER_ZOOM_MIN,
  clampViewerScale,
  fitScale,
  fitsPerRenderPx,
  imageRenderingFor,
  isZoomPresetActive,
  previewTextureBadge,
  renderScaleForFits,
  wheelZoomFactor,
  zoomPercentLabel,
} from "../viewerZoom";

describe("fitsPerRenderPx", () => {
  it("is the FITS width over the preview texture width", () => {
    expect(fitsPerRenderPx(5000, 2000)).toBe(2.5);
  });

  it("is 1 when the FITS width is unknown, zero, negative or not finite", () => {
    expect(fitsPerRenderPx(undefined, 2000)).toBe(1);
    expect(fitsPerRenderPx(0, 2000)).toBe(1);
    expect(fitsPerRenderPx(-5, 2000)).toBe(1);
    expect(fitsPerRenderPx(Number.NaN, 2000)).toBe(1);
    expect(fitsPerRenderPx(5000, 0)).toBe(1);
  });
});

describe("zoom expressed in FITS pixels", () => {
  it("labels a texture scale of 1 on a 2.6:1 preview as about 38 percent of FITS pixels", () => {
    expect(zoomPercentLabel(1, 5200 / 2000)).toBe("38%");
  });

  it("labels the identity as 100 percent when the texture is the full image", () => {
    expect(zoomPercentLabel(1, 1)).toBe("100%");
  });

  it("keeps one decimal under 10 percent so a fitted large image does not read 0 percent", () => {
    expect(zoomPercentLabel(0.1, 20)).toBe("0.5%");
    expect(zoomPercentLabel(0.2, 2.5)).toBe("8.0%");
  });

  it("maps a FITS preset to the texture scale that shows it", () => {
    expect(renderScaleForFits(1, 2.6)).toBeCloseTo(2.6, 12);
    expect(renderScaleForFits(0.5, 1)).toBe(0.5);
  });

  it("marks a preset active when the FITS scale matches it, not the texture scale", () => {
    expect(isZoomPresetActive(2.6, 1, 2.6)).toBe(true);
    expect(isZoomPresetActive(1, 1, 2.6)).toBe(false);
    expect(isZoomPresetActive(1.3, 0.5, 2.6)).toBe(true);
  });

  it("tolerates a relative error under one percent for every preset", () => {
    expect(isZoomPresetActive(8 * 1.005, 8, 1)).toBe(true);
    expect(isZoomPresetActive(8 * 1.02, 8, 1)).toBe(false);
    expect(isZoomPresetActive(0.25 * 1.005, 0.25, 1)).toBe(true);
  });
});

describe("clampViewerScale", () => {
  it("keeps the texture scale inside the minimum and the FITS maximum", () => {
    expect(clampViewerScale(0.01, 1)).toBe(VIEWER_ZOOM_MIN);
    expect(clampViewerScale(100, 1)).toBe(VIEWER_ZOOM_MAX);
  });

  it("lets a downsampled preview reach the maximum zoom in FITS pixels", () => {
    expect(clampViewerScale(8 * 4, 4)).toBe(32);
    expect(clampViewerScale(1000, 4)).toBe(VIEWER_ZOOM_MAX * 4);
  });

  it("returns the minimum for a non-finite scale", () => {
    expect(clampViewerScale(Number.NaN, 1)).toBe(VIEWER_ZOOM_MIN);
  });
});

describe("fitScale", () => {
  it("magnifies a small image up to the cap instead of stopping at 1", () => {
    expect(fitScale(800, 600, 64, 64, 1)).toBe(600 / 64);
    expect(fitScale(4000, 4000, 16, 16, 1)).toBe(FIT_SCALE_CAP);
  });

  it("shrinks a large texture to the limiting side", () => {
    expect(fitScale(500, 400, 2000, 1000, 2.5)).toBe(0.25);
  });

  it("caps in FITS pixels, so a downsampled texture may exceed 16 texture pixels only by its ratio", () => {
    expect(fitScale(10000, 10000, 10, 10, 2)).toBe(FIT_SCALE_CAP * 2);
  });
});

describe("wheelZoomFactor", () => {
  it("gives about 1.15 for one 100 px notch towards the user and its inverse away", () => {
    expect(wheelZoomFactor(-100, 0)).toBeCloseTo(1.15, 10);
    expect(wheelZoomFactor(100, 0)).toBeCloseTo(1 / 1.15, 10);
  });

  it("scales with the delta so ten 10 px touchpad events equal one notch", () => {
    let f = 1;
    for (let i = 0; i < 10; i++) f *= wheelZoomFactor(-10, 0);
    expect(f).toBeCloseTo(1.15, 10);
    expect(wheelZoomFactor(-1, 0)).toBeLessThan(1.002);
  });

  it("treats three lines as one notch and one page as one notch", () => {
    expect(wheelZoomFactor(-3, 1)).toBeCloseTo(1.15, 10);
    expect(wheelZoomFactor(-1, 2)).toBeCloseTo(1.15, 10);
  });

  it("is neutral for a zero or non-finite delta", () => {
    expect(wheelZoomFactor(0, 0)).toBe(1);
    expect(wheelZoomFactor(Number.NaN, 0)).toBe(1);
    expect(wheelZoomFactor(Number.POSITIVE_INFINITY, 0)).toBe(1);
  });
});

describe("imageRenderingFor", () => {
  it("is pixelated whenever a texture pixel is magnified", () => {
    expect(imageRenderingFor(1.01)).toBe("pixelated");
    expect(imageRenderingFor(2)).toBe("pixelated");
  });

  it("is smooth at or below one screen pixel per texture pixel", () => {
    expect(imageRenderingFor(1)).toBe("auto");
    expect(imageRenderingFor(0.25)).toBe("auto");
  });
});

describe("previewTextureBadge", () => {
  it("names the texture width and the FITS-to-texture ratio when the preview is downsampled", () => {
    expect(previewTextureBadge(1920, 5000)).toBe("preview 1920 px (2.6:1)");
  });

  it("is null when the texture holds every FITS pixel or the FITS width is unknown", () => {
    expect(previewTextureBadge(512, 512)).toBeNull();
    expect(previewTextureBadge(512, undefined)).toBeNull();
    expect(previewTextureBadge(0, 5000)).toBeNull();
  });
});
