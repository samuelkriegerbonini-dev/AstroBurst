import { describe, it, expect } from "vitest";
import {
  backToFileAction,
  formatPixelValue,
  gpuAfterProbe,
  gpuDisplayOnScreen,
  keptToolFileKey,
  previewTextureTitle,
  rightToolSlots,
  statusStripIdleText,
  statusStripParts,
  viewerPublishesPixel,
  type RightToolSlotInput,
} from "../previewShell";

function slots(patch: Partial<RightToolSlotInput>) {
  return rightToolSlots({
    rightTool: null,
    displayTool: null,
    columnMounted: false,
    fileKey: "a",
    keptFileKey: null,
    ...patch,
  });
}

describe("rightToolSlots", () => {
  it("mounts nothing before any tool is opened", () => {
    expect(slots({})).toEqual({ kept: null, transient: null });
  });

  it("shows analysis as the active kept slot while it is the open tool", () => {
    expect(slots({ rightTool: "analysis", displayTool: "analysis", columnMounted: true, keptFileKey: "a" })).toEqual({
      kept: { visible: true, active: true },
      transient: null,
    });
  });

  it("mounts analysis on the render that opens it, before the kept file key catches up", () => {
    expect(slots({ rightTool: "analysis", displayTool: "analysis", columnMounted: true, keptFileKey: null }).kept).toEqual({
      visible: true,
      active: true,
    });
  });

  it("keeps analysis mounted but hidden and inactive while another tool is shown", () => {
    expect(slots({ rightTool: "headers", displayTool: "headers", columnMounted: true, keptFileKey: "a" })).toEqual({
      kept: { visible: false, active: false },
      transient: { id: "headers", active: true },
    });
  });

  it("keeps analysis visible but inactive during the close animation", () => {
    expect(slots({ rightTool: null, displayTool: "analysis", columnMounted: true, keptFileKey: "a" })).toEqual({
      kept: { visible: true, active: false },
      transient: null,
    });
  });

  it("keeps analysis mounted, hidden and inactive after the column has closed", () => {
    expect(slots({ rightTool: null, displayTool: "analysis", columnMounted: false, keptFileKey: "a" })).toEqual({
      kept: { visible: false, active: false },
      transient: null,
    });
  });

  it("drops the hidden analysis when another file is loaded", () => {
    expect(slots({ rightTool: "headers", displayTool: "headers", columnMounted: true, fileKey: "b", keptFileKey: "a" }).kept).toBeNull();
    expect(slots({ rightTool: null, displayTool: "analysis", columnMounted: false, fileKey: "b", keptFileKey: "a" }).kept).toBeNull();
  });

  it("keeps the open analysis mounted across a file change", () => {
    expect(slots({ rightTool: "analysis", displayTool: "analysis", columnMounted: true, fileKey: "b", keptFileKey: "a" }).kept).toEqual({
      visible: true,
      active: true,
    });
  });

  it("marks another tool inactive while the column closes and unmounts it once closed", () => {
    expect(slots({ rightTool: null, displayTool: "headers", columnMounted: true }).transient).toEqual({ id: "headers", active: false });
    expect(slots({ rightTool: null, displayTool: "headers", columnMounted: false }).transient).toBeNull();
  });

  it("never keeps analysis without a file", () => {
    expect(slots({ fileKey: null, keptFileKey: null, rightTool: "headers", displayTool: "headers", columnMounted: true }).kept).toBeNull();
  });
});

describe("keptToolFileKey", () => {
  it("records the file the analysis tool was opened on and keeps it while another tool is shown", () => {
    expect(keptToolFileKey("analysis", "a", null)).toBe("a");
    expect(keptToolFileKey("analysis", "b", "a")).toBe("b");
    expect(keptToolFileKey("headers", "a", "a")).toBe("a");
    expect(keptToolFileKey(null, "a", "a")).toBe("a");
  });

  it("forgets the file as soon as another file is loaded", () => {
    expect(keptToolFileKey("headers", "b", "a")).toBeNull();
    expect(keptToolFileKey(null, "b", "a")).toBeNull();
    expect(keptToolFileKey(null, null, "a")).toBeNull();
  });

  it("does not mount a hidden analysis on returning to a file after A -> B -> A", () => {
    for (const other of ["headers", null] as const) {
      let kept = keptToolFileKey("analysis", "a", null);
      kept = keptToolFileKey(other, "a", kept);
      kept = keptToolFileKey(other, "b", kept);
      kept = keptToolFileKey(other, "a", kept);
      expect(kept).toBeNull();
      expect(slots({ rightTool: other, displayTool: other, columnMounted: other !== null, fileKey: "a", keptFileKey: kept }).kept).toBeNull();
    }
  });
});

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
      "The viewer shows a downsampled preview texture (2048×1152 of 8000×4500 FITS pixels). For full-resolution pixels open Analysis > Deep Zoom.",
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
      statusStripParts({ x: 12, y: 34 }, { x: 12, y: 34, value: 1.5, unit: "MJy/sr" }, { x: 12, y: 34, radec: [150, -2.5] }),
    ).toEqual({ position: "x 12  y 34", value: "1.5000 MJy/sr", sky: "RA 10h00m00.00s  Dec -02°30'00.0\" ICRS" });
  });

  it("omits the unit when the file has none", () => {
    expect(statusStripParts({ x: 1, y: 2 }, { x: 1, y: 2, value: 7, unit: null }, null)).toEqual({
      position: "x 1  y 2",
      value: "7",
      sky: null,
    });
  });

  it("does not show a value or a sky position measured at another pixel", () => {
    expect(
      statusStripParts({ x: 5, y: 5 }, { x: 4, y: 5, value: 3, unit: "e-" }, { x: 5, y: 4, radec: [10, 10] }),
    ).toEqual({ position: "x 5  y 5", value: "…", sky: null });
  });
});
