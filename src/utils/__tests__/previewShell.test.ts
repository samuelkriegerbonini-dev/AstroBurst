import { describe, it, expect } from "vitest";
import {
  CANVAS_HINT_CLASS,
  CUBE_SPECTRUM_HINT,
  GPU_LOAD_FAILED_TITLE,
  GPU_PNG_ONLY_TOGGLE_TITLE,
  GPU_PROBING_CPU_VIEWER_TOGGLE_TITLE,
  GPU_PROBING_TOGGLE_TITLE,
  GPU_WAITING_TITLE,
  GPU_WAITING_TOGGLE_TITLE,
  NEEDS_GPU_TITLE,
  PNG_ONLY_TITLE,
  backToFileAction,
  compareDividerGrabbed,
  cpuViewerDisplayTitle,
  cubeSpectrumClickAllowed,
  cubeSpectrumHintShown,
  fileSwitchCompositeAction,
  formatPixelValue,
  formatWavelength,
  gpuAfterProbe,
  gpuDisplayOnScreen,
  gpuToggleView,
  histogramOnPath,
  previewTextureTitle,
  previewViewer,
  reseedsRgbFileView,
  statusStripIdleText,
  statusStripParts,
  viewerClickRoute,
  viewerPublishesPixel,
  type CompareDividerInput,
  type GpuToggleInput,
  type ViewerClickInput,
} from "../previewShell";

describe("viewerPublishesPixel", () => {
  it("is false for the colour composite, whose viewer publishes no mouse pixel", () => {
    expect(viewerPublishesPixel({ composite: true, fileRgbView: false })).toBe(false);
  });

  it("is true for the mono viewer and for an RGB file shown as itself", () => {
    expect(viewerPublishesPixel({ composite: false, fileRgbView: false })).toBe(true);
    expect(viewerPublishesPixel({ composite: true, fileRgbView: true })).toBe(true);
    expect(viewerPublishesPixel({ composite: false, fileRgbView: true })).toBe(true);
  });
});

describe("statusStripIdleText", () => {
  it("asks for a hover only where the viewer publishes a pixel", () => {
    expect(statusStripIdleText(true)).toBe("Hover the image for x, y, value and RA/Dec");
    expect(statusStripIdleText(false)).toBe("No pixel readout for the colour composite");
  });
});

describe("previewTextureTitle", () => {
  const texture = { renderW: 2048, renderH: 1152 };

  it("points to Deep Zoom when the image has a side above the Deep Zoom threshold", () => {
    expect(previewTextureTitle({ ...texture, fitsW: 8000, fitsH: 4500, deepZoomOffered: true })).toBe(
      "The viewer shows a downsampled preview texture (2048×1152 of 8000×4500 FITS pixels). For full-resolution pixels open Image > Deep Zoom.",
    );
  });

  it("only says the texture is a downsampled preview when Deep Zoom does not exist for the image", () => {
    expect(previewTextureTitle({ ...texture, fitsW: 4096, fitsH: 2304, deepZoomOffered: true })).toBe(
      "The viewer shows a downsampled preview texture (2048×1152 of 4096×2304 FITS pixels).",
    );
  });

  it("never points to Deep Zoom when it would open another image than the one on screen", () => {
    expect(previewTextureTitle({ ...texture, fitsW: 8000, fitsH: 4500, deepZoomOffered: false })).not.toContain("Deep Zoom");
  });
});

describe("gpuDisplayOnScreen", () => {
  const onGpu = { hasFile: true, rgbView: false, useGpu: true, hasRawPixels: true, previewOnly: false };

  it("is true only for the mono GPU viewer that shows the display bar", () => {
    expect(gpuDisplayOnScreen(onGpu)).toBe(true);
  });

  it("is false for the CPU viewer, a composite, a PNG-only result, missing pixels or no file", () => {
    expect(gpuDisplayOnScreen({ ...onGpu, useGpu: false })).toBe(false);
    expect(gpuDisplayOnScreen({ ...onGpu, rgbView: true })).toBe(false);
    expect(gpuDisplayOnScreen({ ...onGpu, previewOnly: true })).toBe(false);
    expect(gpuDisplayOnScreen({ ...onGpu, hasRawPixels: false })).toBe(false);
    expect(gpuDisplayOnScreen({ ...onGpu, hasFile: false })).toBe(false);
  });
});

describe("previewViewer", () => {
  const gpuFits = { hasFile: true, rgbView: false, useGpu: true, hasRawPixels: true, previewOnly: false };

  it("shows the CPU viewer for a PNG-only result while the GPU is on", () => {
    expect(previewViewer({ ...gpuFits, previewOnly: true })).toBe("cpu");
    expect(previewViewer({ ...gpuFits, previewOnly: true, hasRawPixels: false })).toBe("cpu");
  });

  it("shows the CPU viewer for a FITS result whose GPU pixels are not loaded yet or failed to load", () => {
    expect(previewViewer({ ...gpuFits, hasRawPixels: false })).toBe("cpu");
  });

  it("shows the preview tab once the GPU pixels of a FITS result are loaded", () => {
    expect(previewViewer(gpuFits)).toBe("preview-tab");
  });

  it("keeps the preview tab for a colour view, with or without GPU pixels", () => {
    expect(previewViewer({ ...gpuFits, rgbView: true, hasRawPixels: false })).toBe("preview-tab");
    expect(previewViewer({ ...gpuFits, rgbView: true, useGpu: false })).toBe("preview-tab");
  });

  it("shows the CPU viewer whenever the GPU is off", () => {
    expect(previewViewer({ ...gpuFits, useGpu: false })).toBe("cpu");
  });

  it("is empty without a file", () => {
    expect(previewViewer({ ...gpuFits, hasFile: false })).toBe("empty");
    expect(previewViewer({ ...gpuFits, hasFile: false, rgbView: true })).toBe("empty");
  });

  it("uses the preview tab exactly when the GPU display is on screen, outside colour views", () => {
    for (const useGpu of [true, false]) {
      for (const hasRawPixels of [true, false]) {
        for (const previewOnly of [true, false]) {
          const input = { hasFile: true, rgbView: false, useGpu, hasRawPixels, previewOnly };
          expect(previewViewer(input) === "preview-tab").toBe(gpuDisplayOnScreen(input));
        }
      }
    }
  });
});

describe("cpuViewerDisplayTitle", () => {
  it("keeps the needs-GPU reason while the GPU is off, PNG-only or not", () => {
    expect(cpuViewerDisplayTitle({ useGpu: false, previewOnly: false, loadFailed: false })).toBe(NEEDS_GPU_TITLE);
    expect(cpuViewerDisplayTitle({ useGpu: false, previewOnly: true, loadFailed: false })).toBe(NEEDS_GPU_TITLE);
    expect(NEEDS_GPU_TITLE).toBe("needs GPU rendering");
  });

  it("names the PNG-only result while the GPU is on", () => {
    expect(cpuViewerDisplayTitle({ useGpu: true, previewOnly: true, loadFailed: false })).toBe(PNG_ONLY_TITLE);
    expect(PNG_ONLY_TITLE).toContain("PNG-only result");
  });

  it("says it waits for the GPU image while the pixels load", () => {
    expect(cpuViewerDisplayTitle({ useGpu: true, previewOnly: false, loadFailed: false })).toBe(GPU_WAITING_TITLE);
    expect(GPU_WAITING_TITLE).toBe("waiting for the GPU image");
  });

  it("says the GPU image load failed", () => {
    expect(cpuViewerDisplayTitle({ useGpu: true, previewOnly: false, loadFailed: true })).toBe(GPU_LOAD_FAILED_TITLE);
    expect(GPU_LOAD_FAILED_TITLE).toBe("GPU image load failed");
  });
});

describe("gpuToggleView", () => {
  const onGpu: GpuToggleInput = {
    probing: false,
    loading: false,
    available: true,
    supported: true,
    useGpu: true,
    loadError: null,
    reason: null,
    pngOnlyMono: false,
    cpuViewerShown: false,
  };

  it("shows GPU in the GPU colour while the GPU display is on screen", () => {
    expect(gpuToggleView(onGpu)).toEqual({ state: "gpu", label: "GPU", title: "Rendering on GPU (WebGPU)", tone: "gpu" });
  });

  it("keeps the GPU label but mutes it and says the CPU viewer shows a PNG-only result", () => {
    expect(gpuToggleView({ ...onGpu, pngOnlyMono: true, cpuViewerShown: true })).toEqual({
      state: "png-only",
      label: "GPU",
      title: GPU_PNG_ONLY_TOGGLE_TITLE,
      tone: "muted",
    });
    expect(GPU_PNG_ONLY_TOGGLE_TITLE).toContain("CPU viewer");
  });

  it("says the CPU viewer is shown after a GPU image load failure", () => {
    const view = gpuToggleView({ ...onGpu, loadError: "boom" });
    expect(view).toEqual({
      state: "failed",
      label: "GPU failed",
      title: "GPU image load failed: boom — showing the CPU viewer; click to switch to CPU",
      tone: "failed",
    });
    expect(gpuToggleView({ ...onGpu, loadError: "boom", pngOnlyMono: true }).tone).toBe("failed");
  });

  it("shows a busy label while probing or loading, whatever the record", () => {
    expect(gpuToggleView({ ...onGpu, probing: true, available: null })).toMatchObject({ state: "probing", label: "...", tone: "gpu" });
    expect(gpuToggleView({ ...onGpu, loading: true })).toMatchObject({ state: "loading", label: "...", title: "Rendering on GPU (WebGPU)" });
    expect(gpuToggleView({ ...onGpu, loading: true, pngOnlyMono: true }).tone).toBe("gpu");
  });

  it("says the CPU viewer shows the image while the GPU image is loaded", () => {
    const shown = { ...onGpu, cpuViewerShown: true };
    expect(gpuToggleView({ ...shown, loading: true })).toEqual({
      state: "loading",
      label: "...",
      title: GPU_WAITING_TOGGLE_TITLE,
      tone: "gpu",
    });
    expect(gpuToggleView(shown).title).toBe(GPU_WAITING_TOGGLE_TITLE);
    expect(GPU_WAITING_TOGGLE_TITLE).toContain("CPU viewer");
    expect(GPU_WAITING_TOGGLE_TITLE).not.toContain("Rendering on GPU");
  });

  it("keeps the other titles when the CPU viewer is shown", () => {
    const shown = { ...onGpu, cpuViewerShown: true };
    expect(gpuToggleView({ ...shown, pngOnlyMono: true }).title).toBe(GPU_PNG_ONLY_TOGGLE_TITLE);
    expect(gpuToggleView({ ...shown, useGpu: false }).title).toBe("Rendering on CPU — click to use GPU");
    expect(gpuToggleView({ ...shown, loadError: "boom" }).title).toBe(
      "GPU image load failed: boom — showing the CPU viewer; click to switch to CPU",
    );
  });

  it("says a GPU check is running while probing, whatever the saved choice, and that the CPU viewer shows the image", () => {
    const probing = { ...onGpu, probing: true, available: null, cpuViewerShown: true };
    expect(gpuToggleView(probing)).toEqual({
      state: "probing",
      label: "...",
      title: GPU_PROBING_CPU_VIEWER_TOGGLE_TITLE,
      tone: "gpu",
    });
    expect(gpuToggleView({ ...probing, useGpu: false })).toEqual({
      state: "probing",
      label: "...",
      title: GPU_PROBING_CPU_VIEWER_TOGGLE_TITLE,
      tone: "off",
    });
    expect(gpuToggleView({ ...probing, cpuViewerShown: false }).title).toBe(GPU_PROBING_TOGGLE_TITLE);
    expect(GPU_PROBING_CPU_VIEWER_TOGGLE_TITLE).toContain("CPU viewer");
  });

  it("never invites a click on the disabled toggle while a retry probe runs", () => {
    const retrying = {
      ...onGpu,
      probing: true,
      useGpu: false,
      available: false,
      reason: "GPU device lost — using CPU",
      cpuViewerShown: true,
    };
    expect(gpuToggleView(retrying).title).toBe(GPU_PROBING_CPU_VIEWER_TOGGLE_TITLE);
    expect(gpuToggleView({ ...retrying, probing: false }).title).toBe("GPU device lost — using CPU — click to retry");
    for (const input of [retrying, { ...retrying, available: null }, { ...retrying, useGpu: true, available: null }]) {
      expect(gpuToggleView(input).title).not.toContain("click");
    }
  });

  it("shows CPU with an invitation to switch while the GPU is off", () => {
    expect(gpuToggleView({ ...onGpu, useGpu: false })).toEqual({
      state: "cpu",
      label: "CPU",
      title: "Rendering on CPU — click to use GPU",
      tone: "off",
    });
    expect(gpuToggleView({ ...onGpu, useGpu: false, pngOnlyMono: true }).state).toBe("cpu");
  });

  it("offers a retry with the reason when the GPU is unavailable", () => {
    expect(gpuToggleView({ ...onGpu, useGpu: false, available: false, reason: "GPU device lost — using CPU" })).toEqual({
      state: "unavailable",
      label: "CPU",
      title: "GPU device lost — using CPU — click to retry",
      tone: "off",
    });
    expect(gpuToggleView({ ...onGpu, useGpu: false, available: false, supported: false, reason: "WebGPU not supported by this browser" }).title).toBe(
      "WebGPU not supported by this browser",
    );
  });
});

describe("gpuAfterProbe", () => {
  it("falls back to the CPU viewer when the probe fails, whatever was saved", () => {
    expect(gpuAfterProbe(false, true, true)).toBe(false);
    expect(gpuAfterProbe(false, null, false)).toBe(false);
  });

  it("turns the GPU on after a successful probe when nothing was saved", () => {
    expect(gpuAfterProbe(true, null, false)).toBe(true);
  });

  it("keeps the saved choice after a successful probe", () => {
    expect(gpuAfterProbe(true, false, false)).toBe(false);
    expect(gpuAfterProbe(true, true, true)).toBe(true);
  });
});

describe("CANVAS_HINT_CLASS", () => {
  it("keeps the shared cube hint on one line, out of the pointer's way and above the regions layer (z-index 5)", () => {
    const classes = CANVAS_HINT_CLASS.split(" ");
    expect(classes).toContain("whitespace-nowrap");
    expect(classes).toContain("pointer-events-none");
    expect(classes).toContain("z-[6]");
    expect(CUBE_SPECTRUM_HINT).toBe("Click to extract spectrum");
  });
});

describe("cubeSpectrumHintShown", () => {
  it("shows the hint on a spectral cube and on maps on its spatial grid", () => {
    expect(cubeSpectrumHintShown({ isSpectralCube: true, fileDims: [53, 55], displayedDims: [53, 55] })).toBe(true);
  });

  it("hides the hint on a PV diagram, whose pixels are offset x channel", () => {
    expect(cubeSpectrumHintShown({ isSpectralCube: true, fileDims: [53, 55], displayedDims: [44, 3814] })).toBe(false);
  });

  it("hides the hint on a ramp and its quick slope, where a click plots the pixel ramp", () => {
    expect(cubeSpectrumHintShown({ isSpectralCube: false, fileDims: [2048, 3200], displayedDims: [2048, 3200] })).toBe(false);
    expect(cubeSpectrumHintShown({ isSpectralCube: false, fileDims: [2048, 3200], displayedDims: [2048, 2048] })).toBe(false);
  });

  it("hides the hint while the file or the displayed image has no dimensions", () => {
    expect(cubeSpectrumHintShown({ isSpectralCube: true, fileDims: null, displayedDims: [53, 55] })).toBe(false);
    expect(cubeSpectrumHintShown({ isSpectralCube: true, fileDims: [53, 55], displayedDims: null })).toBe(false);
  });
});

describe("cubeSpectrumClickAllowed", () => {
  it("allows a spectrum click on the cube and on maps on its spatial grid", () => {
    expect(cubeSpectrumClickAllowed({ isCube: true, hasRamp: false, fileDims: [53, 55], displayedDims: [53, 55] })).toBe(true);
  });

  it("refuses a click on a PV diagram, whose pixels are offset x channel and not spaxels", () => {
    expect(cubeSpectrumClickAllowed({ isCube: true, hasRamp: false, fileDims: [53, 55], displayedDims: [40, 3814] })).toBe(false);
  });

  it("allows a click on a ramp on another grid, whose own mapper handles the displayed grid", () => {
    expect(cubeSpectrumClickAllowed({ isCube: true, hasRamp: true, fileDims: [2048, 3200], displayedDims: [2048, 2048] })).toBe(true);
  });

  it("refuses a click on a plain image", () => {
    expect(cubeSpectrumClickAllowed({ isCube: false, hasRamp: false, fileDims: [53, 55], displayedDims: [53, 55] })).toBe(false);
  });

  it("refuses a click while the displayed image has no dimensions, as sameGrid does", () => {
    expect(cubeSpectrumClickAllowed({ isCube: true, hasRamp: false, fileDims: [53, 55], displayedDims: null })).toBe(false);
  });
});

describe("histogramOnPath", () => {
  const cubeHist = { data_min: 0, data_max: 9 };

  it("uses the histogram only when it was computed for the displayed path", () => {
    expect(histogramOnPath(cubeHist, "/data/cube_s3d.fits", "/data/cube_s3d.fits")).toBe(cubeHist);
  });

  it("drops the cube histogram once a channel FITS is displayed and its own histogram has not landed", () => {
    expect(histogramOnPath(cubeHist, "/data/cube_s3d.fits", "/out/cube_frame_ab12_13.fits")).toBeNull();
  });

  it("is empty while no histogram path is known", () => {
    expect(histogramOnPath(cubeHist, null, "/data/cube_s3d.fits")).toBeNull();
    expect(histogramOnPath(cubeHist, null, null)).toBeNull();
    expect(histogramOnPath(null, "/data/cube_s3d.fits", "/data/cube_s3d.fits")).toBeNull();
  });
});

describe("compareDividerGrabbed", () => {
  const divider: CompareDividerInput = {
    compareMode: true,
    hasComparison: true,
    viewerError: false,
    offsetX: 400,
    width: 800,
    comparePos: 50,
  };

  it("grabs the drawn divider within 12 px of it", () => {
    expect(compareDividerGrabbed(divider)).toBe(true);
    expect(compareDividerGrabbed({ ...divider, offsetX: 411.5 })).toBe(true);
    expect(compareDividerGrabbed({ ...divider, offsetX: 388.5 })).toBe(true);
    expect(compareDividerGrabbed({ ...divider, offsetX: 200, comparePos: 25 })).toBe(true);
  });

  it("leaves presses 12 px or more away from the divider to the image", () => {
    expect(compareDividerGrabbed({ ...divider, offsetX: 412 })).toBe(false);
    expect(compareDividerGrabbed({ ...divider, offsetX: 388 })).toBe(false);
  });

  it("never grabs a divider that is not drawn", () => {
    expect(compareDividerGrabbed({ ...divider, compareMode: false })).toBe(false);
    expect(compareDividerGrabbed({ ...divider, hasComparison: false })).toBe(false);
    expect(compareDividerGrabbed({ ...divider, viewerError: true })).toBe(false);
  });
});

describe("viewerClickRoute", () => {
  const click: ViewerClickInput = {
    press: { x: 100, y: 100 },
    release: { x: 100, y: 100 },
    cursorMode: "crosshair",
    onImage: true,
    hasPixelHandler: true,
    hasCanvasHandler: true,
  };

  it("routes a crosshair click to the pixel handler and a pan click to the canvas handler", () => {
    expect(viewerClickRoute(click)).toBe("pixel");
    expect(viewerClickRoute({ ...click, cursorMode: "pan" })).toBe("canvas");
    expect(viewerClickRoute({ ...click, hasPixelHandler: false })).toBe("canvas");
  });

  it("drops a pan click without a canvas handler and any click off the image", () => {
    expect(viewerClickRoute({ ...click, cursorMode: "pan", hasCanvasHandler: false })).toBe("none");
    expect(viewerClickRoute({ ...click, onImage: false })).toBe("none");
  });

  it("counts a release up to 4 px from the press as a click and anything further as a drag", () => {
    expect(viewerClickRoute({ ...click, release: { x: 104, y: 100 } })).toBe("pixel");
    expect(viewerClickRoute({ ...click, release: { x: 103, y: 104 } })).toBe("none");
    expect(viewerClickRoute({ ...click, cursorMode: "pan", release: { x: 140, y: 100 } })).toBe("none");
  });

  it("drops a release whose press grabbed the compare divider", () => {
    expect(viewerClickRoute({ ...click, press: null })).toBe("none");
    expect(viewerClickRoute({ ...click, press: null, cursorMode: "pan" })).toBe("none");
  });

  it("still clicks the image centre when compare mode was left on after the comparison went away", () => {
    const centre = { x: 400, y: 300 };
    const grabbed = compareDividerGrabbed({
      compareMode: true,
      hasComparison: false,
      viewerError: false,
      offsetX: centre.x,
      width: 800,
      comparePos: 50,
    });
    const press = grabbed ? null : centre;
    expect(grabbed).toBe(false);
    expect(viewerClickRoute({ ...click, press, release: centre })).toBe("pixel");
    expect(viewerClickRoute({ ...click, press, release: centre, cursorMode: "pan" })).toBe("canvas");
  });
});

describe("backToFileAction", () => {
  it("shows the RGB file itself when an unprocessed RGB file is loaded", () => {
    expect(backToFileAction({ isRgbFile: true, hasProcessed: false, wizardCompositeReady: true })).toBe("rgb-file");
  });

  it("only resets the display while the wizard composite is ready, so its planes survive", () => {
    expect(backToFileAction({ isRgbFile: false, hasProcessed: false, wizardCompositeReady: true })).toBe("display-only");
    expect(backToFileAction({ isRgbFile: true, hasProcessed: true, wizardCompositeReady: true })).toBe("display-only");
  });

  it("clears the composite cache when no wizard composite depends on it", () => {
    expect(backToFileAction({ isRgbFile: false, hasProcessed: false, wizardCompositeReady: false })).toBe("clear");
  });
});

describe("fileSwitchCompositeAction", () => {
  const WIZARD = "asset://rgb_composite_1.png";
  const PREVIOUS_RGB = "asset://drizzle_rgb.png";

  it("parks the wizard composite that is on screen when the wizard composite is ready", () => {
    expect(fileSwitchCompositeAction({ livePreviewUrl: WIZARD, previousFileRgbUrl: null, wizardCompositeReady: true })).toBe("park");
    expect(fileSwitchCompositeAction({ livePreviewUrl: WIZARD, previousFileRgbUrl: PREVIOUS_RGB, wizardCompositeReady: true })).toBe("park");
  });

  it("resets when the screen shows the previous file's own RGB view", () => {
    expect(fileSwitchCompositeAction({ livePreviewUrl: PREVIOUS_RGB, previousFileRgbUrl: PREVIOUS_RGB, wizardCompositeReady: true })).toBe("reset");
  });

  it("resets when no composite is on screen, or when the wizard has no ready composite", () => {
    expect(fileSwitchCompositeAction({ livePreviewUrl: null, previousFileRgbUrl: null, wizardCompositeReady: true })).toBe("reset");
    expect(fileSwitchCompositeAction({ livePreviewUrl: WIZARD, previousFileRgbUrl: null, wizardCompositeReady: false })).toBe("reset");
  });
});

describe("reseedsRgbFileView", () => {
  const OLD = "asset://drizzle_rgb.png";
  const NEW = "asset://drizzle_rgb.png?v=3";
  const base = { sameFile: true, isRgb: true, previousPreviewUrl: OLD, nextPreviewUrl: NEW, livePreviewUrl: OLD };

  it("re-seeds an RGB file reloaded in place while its own view is on screen", () => {
    expect(reseedsRgbFileView(base)).toBe(true);
  });

  it("leaves a wizard composite shown over the reloaded RGB file alone", () => {
    expect(reseedsRgbFileView({ ...base, livePreviewUrl: "asset://rgb_composite_1.png" })).toBe(false);
    expect(reseedsRgbFileView({ ...base, livePreviewUrl: null })).toBe(false);
  });

  it("ignores file switches, mono files and results whose preview did not change", () => {
    expect(reseedsRgbFileView({ ...base, sameFile: false })).toBe(false);
    expect(reseedsRgbFileView({ ...base, isRgb: false })).toBe(false);
    expect(reseedsRgbFileView({ ...base, nextPreviewUrl: OLD })).toBe(false);
    expect(reseedsRgbFileView({ ...base, nextPreviewUrl: null })).toBe(false);
    expect(reseedsRgbFileView({ ...base, previousPreviewUrl: null, livePreviewUrl: null })).toBe(false);
  });
});

describe("formatPixelValue", () => {
  it("prints integers as they are and fractions with four or two decimals", () => {
    expect(formatPixelValue(12)).toBe("12");
    expect(formatPixelValue(0)).toBe("0");
    expect(formatPixelValue(1.23456)).toBe("1.2346");
    expect(formatPixelValue(-123.456)).toBe("-123.46");
  });

  it("switches to exponent notation for tiny and huge values", () => {
    expect(formatPixelValue(0.000123456)).toBe("1.235e-4");
    expect(formatPixelValue(1234567.5)).toBe("1.235e+6");
  });

  it("shows a dash for a missing or non-finite value", () => {
    expect(formatPixelValue(null)).toBe("—");
    expect(formatPixelValue(Number.NaN)).toBe("—");
  });
});

describe("statusStripParts", () => {
  it("is empty while the cursor is off the image", () => {
    expect(statusStripParts(null, null, null)).toBeNull();
  });

  it("shows the 0-based position, the value with its unit and the ICRS position", () => {
    expect(
      statusStripParts(
        { x: 12, y: 34 },
        { x: 12, y: 34, value: 1.5, unit: "MJy/sr", wavelength: null },
        { x: 12, y: 34, radec: [150, -2.5] },
      ),
    ).toEqual({ position: "x 12  y 34", value: "1.5000 MJy/sr", sky: "RA 10h00m00.00s  Dec -02°30'00.0\" ICRS", wavelength: null });
  });

  it("omits the unit when the file has none", () => {
    expect(statusStripParts({ x: 1, y: 2 }, { x: 1, y: 2, value: 7, unit: null, wavelength: null }, null)).toEqual({
      position: "x 1  y 2",
      value: "7",
      sky: null,
      wavelength: null,
    });
  });

  it("does not show a value or a sky position measured at another pixel", () => {
    expect(
      statusStripParts({ x: 5, y: 5 }, { x: 4, y: 5, value: 3, unit: "e-", wavelength: null }, { x: 5, y: 4, radec: [10, 10] }),
    ).toEqual({ position: "x 5  y 5", value: "…", sky: null, wavelength: null });
  });

  it("adds the wavelength of the WAVELENGTH companion at this pixel", () => {
    const parts = statusStripParts({ x: 3, y: 4 }, { x: 3, y: 4, value: 2, unit: "MJy/sr", wavelength: { value: 2.12341, unit: "um" } }, null);
    expect(parts?.wavelength).toBe("λ 2.1234 um");
  });

  it("has no wavelength part without the plane or for a probe made at another pixel", () => {
    expect(statusStripParts({ x: 3, y: 4 }, { x: 3, y: 4, value: 2, unit: null, wavelength: null }, null)?.wavelength).toBeNull();
    expect(
      statusStripParts({ x: 3, y: 5 }, { x: 3, y: 4, value: 2, unit: null, wavelength: { value: 2.1, unit: "um" } }, null)?.wavelength,
    ).toBeNull();
  });
});

describe("formatWavelength", () => {
  it("prints four decimals below 10 and three from 10 up, with the unit", () => {
    expect(formatWavelength({ value: 2.12341, unit: "um" })).toBe("λ 2.1234 um");
    expect(formatWavelength({ value: 1.654, unit: "um" })).toBe("λ 1.6540 um");
    expect(formatWavelength({ value: 12.34567, unit: "um" })).toBe("λ 12.346 um");
    expect(formatWavelength({ value: 6563.2, unit: "Angstrom" })).toBe("λ 6563.200 Angstrom");
  });

  it("omits the unit when the plane has none", () => {
    expect(formatWavelength({ value: 2.5, unit: null })).toBe("λ 2.5000");
  });
});
