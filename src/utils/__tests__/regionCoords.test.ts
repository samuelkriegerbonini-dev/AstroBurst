import { describe, it, expect } from "vitest";
import {
  isRegionMappingUsable,
  regionPointToScreen,
  regionToleranceImagePx,
  resolveRegionHost,
  screenToRegionPoint,
  type RegionMapping,
} from "../regionCoords";
import { screenToImagePixel, screenPxPerImagePx } from "../pixelMapping";

const ORIGIN = { left: 0, top: 0 };

const GPU_PREVIEW: RegionMapping = {
  transform: { scale: 1, x: 200, y: 50 },
  renderW: 800,
  renderH: 800,
  fitsW: 1600,
  fitsH: 1600,
};

const CPU_FULL: RegionMapping = {
  transform: { scale: 0.33, x: 326, y: 12 },
  renderW: 1600,
  renderH: 1600,
  fitsW: 1600,
  fitsH: 1600,
};

describe("resolveRegionHost", () => {
  const parent = { id: "parent" } as unknown as HTMLElement;
  const canvas = { id: "canvas", parentElement: parent } as unknown as HTMLElement;
  const container = { id: "container" } as unknown as HTMLElement;

  it("prefers the container ref when it is already attached", () => {
    expect(resolveRegionHost(container, canvas)).toBe(container);
  });

  it("falls back to the canvas parent when the container ref is not attached yet", () => {
    expect(resolveRegionHost(null, canvas)).toBe(parent);
  });

  it("returns null when neither the container nor a canvas parent exists", () => {
    const orphan = { id: "orphan", parentElement: null } as unknown as HTMLElement;
    expect(resolveRegionHost(null, orphan)).toBeNull();
  });

  it("returns null when the canvas is detached", () => {
    expect(resolveRegionHost(container, null)).toBeNull();
    expect(resolveRegionHost(null, null)).toBeNull();
  });
});

describe("isRegionMappingUsable", () => {
  it("rejects zero or non-finite mappings", () => {
    expect(isRegionMappingUsable(GPU_PREVIEW)).toBe(true);
    expect(isRegionMappingUsable({ ...GPU_PREVIEW, renderW: 0 })).toBe(false);
    expect(isRegionMappingUsable({ ...GPU_PREVIEW, fitsH: 0 })).toBe(false);
    expect(isRegionMappingUsable({ ...GPU_PREVIEW, transform: { scale: 0, x: 0, y: 0 } })).toBe(false);
    expect(isRegionMappingUsable({ ...GPU_PREVIEW, transform: { scale: NaN, x: 0, y: 0 } })).toBe(false);
    expect(isRegionMappingUsable({ ...GPU_PREVIEW, transform: { scale: 1, x: NaN, y: 0 } })).toBe(false);
  });

  it("returns null from screenToRegionPoint when the mapping is unusable", () => {
    expect(screenToRegionPoint(300, 300, ORIGIN, { ...GPU_PREVIEW, fitsW: 0 })).toBeNull();
  });
});

describe("GPU viewer semantics (downsampled preview, renderW != fitsW)", () => {
  it("places 0-based FITS pixel centres at the centre of the rendered pixel box", () => {
    const px = screenPxPerImagePx(GPU_PREVIEW.transform, GPU_PREVIEW.renderW, GPU_PREVIEW.fitsW);
    expect(px).toBe(0.5);
    const expectScreen = (p: { x: number; y: number }, x: number, y: number) => {
      const s = regionPointToScreen(p, GPU_PREVIEW);
      expect(s.x).toBeCloseTo(x, 9);
      expect(s.y).toBeCloseTo(y, 9);
    };
    expectScreen({ x: 0, y: 0 }, 200.25, 50.25);
    expectScreen({ x: 800, y: 800 }, 600.25, 450.25);
    expectScreen({ x: 1599, y: 1599 }, 999.75, 849.75);
  });

  it("maps a container point back to 0-based FITS centre coordinates", () => {
    const centre = screenToRegionPoint(600.25, 450.25, ORIGIN, GPU_PREVIEW)!;
    expect(centre.x).toBeCloseTo(800, 9);
    expect(centre.y).toBeCloseTo(800, 9);
    const origin = screenToRegionPoint(200.25, 50.25, ORIGIN, GPU_PREVIEW)!;
    expect(origin.x).toBeCloseTo(0, 9);
    expect(origin.y).toBeCloseTo(0, 9);
  });

  it("returns null in the container margin around the scaled image", () => {
    expect(screenToRegionPoint(100, 400, ORIGIN, GPU_PREVIEW)).toBeNull();
    expect(screenToRegionPoint(1150, 400, ORIGIN, GPU_PREVIEW)).toBeNull();
    expect(screenToRegionPoint(600, 10, ORIGIN, GPU_PREVIEW)).toBeNull();
    expect(screenToRegionPoint(600, 880, ORIGIN, GPU_PREVIEW)).toBeNull();
    expect(screenToRegionPoint(199.9, 450, ORIGIN, GPU_PREVIEW)).toBeNull();
    expect(screenToRegionPoint(1000, 450, ORIGIN, GPU_PREVIEW)).toBeNull();
  });

  it("converts a screen-pixel tolerance into image pixels through the render/fits ratio", () => {
    expect(regionToleranceImagePx(6, GPU_PREVIEW)).toBe(12);
  });
});

describe("CPU viewer semantics (full-size render, zoomed and offset)", () => {
  it("places 0-based FITS pixel centres through scale and translation", () => {
    const p = regionPointToScreen({ x: 800, y: 800 }, CPU_FULL);
    expect(p.x).toBeCloseTo(800.5 * 0.33 + 326, 9);
    expect(p.y).toBeCloseTo(800.5 * 0.33 + 12, 9);
  });

  it("maps a container point back to 0-based FITS centre coordinates", () => {
    const pt = screenToRegionPoint(800.5 * 0.33 + 326, 800.5 * 0.33 + 12, ORIGIN, CPU_FULL)!;
    expect(pt.x).toBeCloseTo(800, 9);
    expect(pt.y).toBeCloseTo(800, 9);
  });

  it("converts a screen-pixel tolerance into image pixels", () => {
    expect(regionToleranceImagePx(6, CPU_FULL)).toBeCloseTo(6 / 0.33, 9);
  });
});

describe("cross-viewer agreement", () => {
  it("a point picked in the GPU viewer lands on the same FITS pixels in the CPU viewer", () => {
    const fromGpu = screenToRegionPoint(600.25, 450.25, ORIGIN, GPU_PREVIEW)!;
    const onCpu = regionPointToScreen(fromGpu, CPU_FULL);
    const backOnCpu = screenToRegionPoint(onCpu.x, onCpu.y, ORIGIN, CPU_FULL)!;
    expect(backOnCpu.x).toBeCloseTo(fromGpu.x, 9);
    expect(backOnCpu.y).toBeCloseTo(fromGpu.y, 9);
    expect(fromGpu.x).toBeCloseTo(800, 9);
    expect(fromGpu.y).toBeCloseTo(800, 9);
  });

  it("agrees with the integer pixel probe used by the status bar and the Rust core", () => {
    for (const m of [GPU_PREVIEW, CPU_FULL]) {
      for (const [x, y] of [[0, 0], [5, 7], [799, 1201]] as const) {
        const s = regionPointToScreen({ x, y }, m);
        expect(screenToImagePixel(s.x, s.y, ORIGIN, m.transform, m.renderW, m.renderH, m.fitsW, m.fitsH)).toEqual({ x, y });
      }
    }
  });
});

describe("round trip screen -> image -> screen", () => {
  const rects = [ORIGIN, { left: 37.5, top: 11.25 }];
  const mappings: RegionMapping[] = [
    GPU_PREVIEW,
    CPU_FULL,
    { transform: { scale: 4, x: -1200.5, y: -880.25 }, renderW: 1024, renderH: 512, fitsW: 4096, fitsH: 2048 },
    { transform: { scale: 0.125, x: 15, y: -3.5 }, renderW: 2048, renderH: 2048, fitsW: 2048, fitsH: 2048 },
  ];

  it("returns the original screen point for every viewer mapping", () => {
    for (const m of mappings) {
      for (const rect of rects) {
        const inside = regionPointToScreen({ x: m.fitsW / 3, y: m.fitsH / 7 }, m);
        const screen = { x: inside.x + rect.left, y: inside.y + rect.top };
        const pt = screenToRegionPoint(screen.x, screen.y, rect, m);
        expect(pt).not.toBeNull();
        const back = regionPointToScreen(pt!, m);
        expect(Math.abs(back.x + rect.left - screen.x)).toBeLessThan(1e-9);
        expect(Math.abs(back.y + rect.top - screen.y)).toBeLessThan(1e-9);
      }
    }
  });
});
