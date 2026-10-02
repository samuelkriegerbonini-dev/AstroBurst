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
  ACTUAL_SIZE_SCALE,
  actualSizeView,
  clampZoomPanScale,
  panForZoom,
  wheelGestureZooms,
  ZOOM_PAN_MAX,
  ZOOM_PAN_MIN,
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

describe("ZoomPanView scale limits", () => {
  const fit = Math.min(640 / 2048, 126 / 1365);

  it("lets zoom-out stop at the fit scale when the fit is below the fixed minimum", () => {
    expect(fit).toBeLessThan(ZOOM_PAN_MIN);
    expect(clampZoomPanScale(fit * 0.87, fit)).toBe(fit);
    expect(clampZoomPanScale(fit / 1.5, fit)).toBe(fit);
    expect(clampZoomPanScale(fit * 1.15, fit)).toBeCloseTo(fit * 1.15, 12);
  });

  it("keeps the fixed minimum when the fit is above it", () => {
    expect(clampZoomPanScale(0.1, 0.6)).toBe(ZOOM_PAN_MIN);
    expect(clampZoomPanScale(0.1, 1)).toBe(ZOOM_PAN_MIN);
    expect(clampZoomPanScale(0.5, 1)).toBe(0.5);
  });

  it("caps zoom-in and ignores an unknown fit", () => {
    expect(clampZoomPanScale(40, 0.5)).toBe(ZOOM_PAN_MAX);
    expect(clampZoomPanScale(0.1, 0)).toBe(ZOOM_PAN_MIN);
    expect(clampZoomPanScale(0.1, Number.NaN)).toBe(ZOOM_PAN_MIN);
  });

  it("zooms on every wheel gesture by default and only with Ctrl or Cmd in modifier mode", () => {
    const plain = { ctrlKey: false, metaKey: false };
    expect(wheelGestureZooms("always", plain)).toBe(true);
    expect(wheelGestureZooms("modifier", plain)).toBe(false);
    expect(wheelGestureZooms("modifier", { ctrlKey: true, metaKey: false })).toBe(true);
    expect(wheelGestureZooms("modifier", { ctrlKey: false, metaKey: true })).toBe(true);
  });
});

describe("ZoomPanView actual size", () => {
  const imageW = 2048;
  const imageH = 1365;
  const containerW = 640;
  const containerH = 180;
  const fit = Math.min(containerW / imageW, containerH / imageH);
  const fitPan = { x: (containerW - imageW * fit) / 2, y: (containerH - imageH * fit) / 2 };

  it("keeps the image point under the anchor fixed", () => {
    const pan = panForZoom(0.5, { x: 10, y: 20 }, 2, 100, 50);
    const imageXBefore = (100 - 10) / 0.5;
    const imageXAfter = (100 - pan.x) / 2;
    expect(imageXAfter).toBeCloseTo(imageXBefore, 12);
    expect((50 - pan.y) / 2).toBeCloseTo((50 - 20) / 0.5, 12);
  });

  it("shows one preview pixel per screen pixel, centred on the image when coming from the fit", () => {
    const view = actualSizeView(fit, fitPan, fit, containerW, containerH);
    expect(view.scale).toBe(ACTUAL_SIZE_SCALE);
    expect(view.pan.x + (imageW / 2) * view.scale).toBeCloseTo(containerW / 2, 9);
    expect(view.pan.y + (imageH / 2) * view.scale).toBeCloseTo(containerH / 2, 9);
  });

  it("keeps the point at the centre of the view when the user had panned", () => {
    const pan = { x: -900, y: -300 };
    const scale = 3;
    const centreBefore = { x: (containerW / 2 - pan.x) / scale, y: (containerH / 2 - pan.y) / scale };
    const view = actualSizeView(scale, pan, fit, containerW, containerH);
    expect((containerW / 2 - view.pan.x) / view.scale).toBeCloseTo(centreBefore.x, 9);
    expect((containerH / 2 - view.pan.y) / view.scale).toBeCloseTo(centreBefore.y, 9);
  });

  it("makes a 14 px offset on a 6000 px frame span several screen pixels instead of under one at the fit", () => {
    const previewPerFrame = 2048 / 6000;
    expect(14 * previewPerFrame * fit).toBeLessThan(1);
    expect(14 * previewPerFrame * actualSizeView(fit, fitPan, fit, containerW, containerH).scale).toBeGreaterThan(4);
  });
});
