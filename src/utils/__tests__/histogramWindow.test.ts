import { describe, it, expect } from "vitest";
import {
  constrainStf,
  dragStfMarker,
  histogramSkyWindow,
  normToWindow,
  pickStfMarker,
  stfMarkerPositions,
  windowToNorm,
} from "../histogramWindow";

const frame = { min: 0, max: 1000 };
const sky = { lo: 100, hi: 200 };

describe("histogramSkyWindow", () => {
  it("spans median - 5 sigma to median + 50 sigma", () => {
    expect(histogramSkyWindow({ median: 130, sigma: 2, dataMin: 0, dataMax: 60000 })).toEqual({ lo: 120, hi: 230 });
  });

  it("clips the window to the data range", () => {
    expect(histogramSkyWindow({ median: 3, sigma: 1, dataMin: 0, dataMax: 40 })).toEqual({ lo: 0, hi: 40 });
  });

  it("has no window without a positive finite sigma or with an empty span", () => {
    expect(histogramSkyWindow({ median: 10, sigma: 0, dataMin: 0, dataMax: 100 })).toBeNull();
    expect(histogramSkyWindow({ median: 10, sigma: -1, dataMin: 0, dataMax: 100 })).toBeNull();
    expect(histogramSkyWindow({ median: 10, sigma: Number.NaN, dataMin: 0, dataMax: 100 })).toBeNull();
    expect(histogramSkyWindow({ median: Number.NaN, sigma: 1, dataMin: 0, dataMax: 100 })).toBeNull();
    expect(histogramSkyWindow({ median: 10, sigma: 1, dataMin: 50, dataMax: 50 })).toBeNull();
    expect(histogramSkyWindow({ median: 500, sigma: 1, dataMin: 0, dataMax: 100 })).toBeNull();
  });
});

describe("normToWindow", () => {
  it("is the identity without a window", () => {
    expect(normToWindow(0.37, frame, null)).toEqual({ x: 0.37, pinned: false });
  });

  it("places a frame fraction inside the window", () => {
    const placed = normToWindow(0.15, frame, sky);
    expect(placed.x).toBeCloseTo(0.5, 12);
    expect(placed.pinned).toBe(false);
  });

  it("pins markers outside the window to the nearer edge", () => {
    expect(normToWindow(0, frame, sky)).toEqual({ x: 0, pinned: true });
    expect(normToWindow(0.5, frame, sky)).toEqual({ x: 1, pinned: true });
    expect(normToWindow(1, frame, sky)).toEqual({ x: 1, pinned: true });
  });

  it("does not pin the exact window edges", () => {
    expect(normToWindow(0.1, frame, sky).pinned).toBe(false);
    expect(normToWindow(0.2, frame, sky).pinned).toBe(false);
  });
});

describe("windowToNorm", () => {
  it("is the identity without a window, clamped to [0, 1]", () => {
    expect(windowToNorm(0.42, frame, null)).toBe(0.42);
    expect(windowToNorm(-0.2, frame, null)).toBe(0);
    expect(windowToNorm(1.3, frame, null)).toBe(1);
  });

  it("maps a window position back to the frame fraction", () => {
    expect(windowToNorm(0, frame, sky)).toBeCloseTo(0.1, 12);
    expect(windowToNorm(0.5, frame, sky)).toBeCloseTo(0.15, 12);
    expect(windowToNorm(1, frame, sky)).toBeCloseTo(0.2, 12);
  });

  it("round-trips with normToWindow inside the window", () => {
    for (const n of [0.1, 0.12, 0.15, 0.199]) {
      expect(windowToNorm(normToWindow(n, frame, sky).x, frame, sky)).toBeCloseTo(n, 12);
    }
  });

  it("falls back to the identity on a degenerate frame", () => {
    expect(windowToNorm(0.3, { min: 5, max: 5 }, sky)).toBe(0.3);
    expect(normToWindow(0.3, { min: 5, max: 5 }, sky)).toEqual({ x: 0.3, pinned: false });
  });
});

const jwstFrame = { min: -0.5, max: 3000 };
const jwstSky = { lo: 0.2, hi: 1.3 };
const toNorm = (value: number) => (value - jwstFrame.min) / (jwstFrame.max - jwstFrame.min);
const toData = (norm: number) => jwstFrame.min + norm * (jwstFrame.max - jwstFrame.min);
const midtoneData = (stf: { shadow: number; midtone: number; highlight: number }) =>
  toData(stf.shadow + stf.midtone * (stf.highlight - stf.shadow));
const autoInSky = { shadow: toNorm(0.244), midtone: 0.05, highlight: 1 };
const plain = { shadow: 0.2, midtone: 0.5, highlight: 0.8 };

describe("pickStfMarker", () => {
  it("grabs M, not H, when both sit pinned on the right edge of the sky window", () => {
    const positions = stfMarkerPositions(autoInSky, jwstFrame, jwstSky);
    expect(positions.midtone).toEqual({ x: 1, pinned: true });
    expect(positions.highlight).toEqual({ x: 1, pinned: true });
    expect(pickStfMarker(0.995, autoInSky, jwstFrame, jwstSky, 0.03)).toBe("midtone");
  });

  it("grabs the innermost marker when S and M sit pinned on the left edge", () => {
    const stf = { shadow: 0, midtone: 0.2, highlight: toNorm(1) };
    expect(stfMarkerPositions(stf, jwstFrame, jwstSky).midtone).toEqual({ x: 0, pinned: true });
    expect(pickStfMarker(0.004, stf, jwstFrame, jwstSky, 0.03)).toBe("midtone");
  });

  it("keeps the nearest marker without a window and ignores clicks beyond the threshold", () => {
    expect(pickStfMarker(0.21, plain, frame, null, 0.03)).toBe("shadow");
    expect(pickStfMarker(0.49, plain, frame, null, 0.03)).toBe("midtone");
    expect(pickStfMarker(0.79, plain, frame, null, 0.03)).toBe("highlight");
    expect(pickStfMarker(0.4, plain, frame, null, 0.03)).toBeNull();
  });
});

describe("dragStfMarker", () => {
  it("moves H inside the sky window instead of snapping it to S + 1% of the frame", () => {
    const next = dragStfMarker("highlight", 0.9, autoInSky, jwstFrame, jwstSky)!;
    expect(toData(next.highlight)).toBeCloseTo(1.19, 6);
    expect(normToWindow(next.highlight, jwstFrame, jwstSky).pinned).toBe(false);
    expect(next.shadow).toBe(autoInSky.shadow);
    expect(next.midtone).toBe(autoInSky.midtone);
  });

  it("places M anywhere inside the sky window while H stays pinned at the frame maximum", () => {
    const centre = dragStfMarker("midtone", 0.5, autoInSky, jwstFrame, jwstSky)!;
    expect(midtoneData(centre)).toBeCloseTo(0.75, 6);
    expect(centre.highlight).toBe(1);
    const nearShadow = dragStfMarker("midtone", 0.045, autoInSky, jwstFrame, jwstSky)!;
    expect(midtoneData(nearShadow)).toBeCloseTo(0.2495, 6);
    expect(nearShadow.midtone).toBeLessThan(0.001);
    expect(nearShadow.midtone).toBeGreaterThan(0);
  });

  it("keeps the S-H gap at 1% of the window span, not of the frame", () => {
    const stf = { shadow: toNorm(0.5), midtone: 0.5, highlight: toNorm(1) };
    const shadow = dragStfMarker("shadow", 1, stf, jwstFrame, jwstSky)!;
    expect(toData(shadow.shadow)).toBeCloseTo(0.989, 6);
    const highlight = dragStfMarker("highlight", 0, stf, jwstFrame, jwstSky)!;
    expect(toData(highlight.highlight)).toBeCloseTo(0.511, 6);
  });

  it("skips a drag that would leave the marker pinned on the same edge", () => {
    const aboveSky = { shadow: toNorm(5), midtone: 0.3, highlight: 1 };
    expect(dragStfMarker("highlight", 0.5, aboveSky, jwstFrame, jwstSky)).toBeNull();
    expect(dragStfMarker("midtone", 0.5, aboveSky, jwstFrame, jwstSky)).toBeNull();
    const shadow = dragStfMarker("shadow", 0.5, aboveSky, jwstFrame, jwstSky)!;
    expect(toData(shadow.shadow)).toBeCloseTo(0.75, 6);
  });

  it("keeps the full-frame clamps without a window", () => {
    expect(dragStfMarker("highlight", 0.1, plain, frame, null)!.highlight).toBeCloseTo(0.21, 12);
    expect(dragStfMarker("shadow", 0.9, plain, frame, null)!.shadow).toBeCloseTo(0.79, 12);
    expect(dragStfMarker("midtone", 0.1, plain, frame, null)!.midtone).toBe(0.001);
    expect(dragStfMarker("midtone", 0.95, plain, frame, null)!.midtone).toBe(0.999);
    expect(dragStfMarker("midtone", 0.35, plain, frame, null)!.midtone).toBeCloseTo(0.25, 12);
  });

  it("refuses a midtone drag over a collapsed S-H range", () => {
    expect(dragStfMarker("midtone", 0.5, { shadow: 0.5, midtone: 0.5, highlight: 0.5 }, frame, null)).toBeNull();
  });
});

describe("constrainStf", () => {
  it("keeps a typed H inside the sky window and scales the midtone floor to the window", () => {
    const typed = constrainStf({ shadow: toNorm(0.244), midtone: 1e-4, highlight: toNorm(0.9) }, jwstFrame, jwstSky);
    expect(toData(typed.highlight)).toBeCloseTo(0.9, 6);
    expect(typed.midtone).toBe(0.001);
    const pinnedH = constrainStf({ ...autoInSky, midtone: 1e-4 }, jwstFrame, jwstSky);
    expect(pinnedH.midtone).toBe(1e-4);
  });

  it("matches the full-frame manual clamps without a window", () => {
    expect(constrainStf({ shadow: -1, midtone: 0, highlight: 0.001 }, frame, null)).toEqual({
      shadow: 0,
      midtone: 0.001,
      highlight: 0.01,
    });
    expect(constrainStf({ shadow: 2, midtone: 2, highlight: 0.5 }, frame, null)).toEqual({
      shadow: 0.99,
      midtone: 0.999,
      highlight: 1,
    });
  });
});
