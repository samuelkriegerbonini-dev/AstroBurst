import { describe, expect, it, vi } from "vitest";
import { MASK_LAYER_ID, MASK_LAYER_KIND, createMaskPainter, isMaskLayer, maskLayer, type MaskSurface } from "../maskPainter";
import type { MaskPreview, MaskStyle } from "../../../../shared/types/masks";
import type { OverlayPaintContext } from "../../../../utils/overlayStore";
import { regionPointToScreen, type RegionMapping } from "../../../../utils/regionCoords";
import type { Pt } from "../../../../utils/regionGeometry";

const RED: MaskStyle = { opacity: 0.45, tint: [255, 0, 0], displayInvert: false };
const SURFACE = { surface: "mask" } as unknown as CanvasImageSource;

interface Surfaced {
  rgba: Uint8ClampedArray;
  width: number;
  height: number;
}

function capturingSurface(): { made: Surfaced[]; surface: MaskSurface } {
  const made: Surfaced[] = [];
  const surface: MaskSurface = (rgba, width, height) => {
    made.push({ rgba, width, height });
    return SURFACE;
  };
  return { made, surface };
}

function channel(rgba: Uint8ClampedArray, offset: number): number[] {
  return Array.from(rgba).filter((_, i) => i % 4 === offset);
}

function preview(width: number, height: number, cells: number[]): MaskPreview {
  return { width, height, cells: new Uint8Array(cells) };
}

interface Drawn {
  image: unknown;
  at: [number, number];
  matrix: number[];
  smoothing: boolean;
}

function fakeContext() {
  const drawn: Drawn[] = [];
  let matrix = [1, 0, 0, 1, 0, 0];
  const ctx = {
    imageSmoothingEnabled: true,
    save: vi.fn(),
    restore: vi.fn(),
    setTransform: vi.fn(),
    transform: vi.fn((...m: number[]) => {
      matrix = m;
    }),
    drawImage: vi.fn((image: unknown, x: number, y: number) => {
      drawn.push({ image, at: [x, y], matrix, smoothing: ctx.imageSmoothingEnabled });
    }),
  };
  return { ctx, drawn };
}

function paint(painter: ReturnType<typeof createMaskPainter>, ctx: unknown, toScreen: (p: Pt) => Pt): void {
  painter({ ctx, toScreen, width: 1200, height: 800, screenPxPerImagePx: 1 } as unknown as OverlayPaintContext);
}

function apply(matrix: number[], u: number, v: number): Pt {
  const [a, b, c, d, e, f] = matrix;
  return { x: a * u + c * v + e, y: b * u + d * v + f };
}

describe("mask painter", () => {
  it("maskPainter_alpha_is_proportional_to_m_and_honours_display_invert", () => {
    const cells = [0, 1, 51, 128, 204, 255];
    const { made, surface } = capturingSurface();
    const layer = maskLayer({ maskId: "m01", preview: preview(3, 2, cells), dims: [3, 2], style: RED }, surface);

    expect(made).toHaveLength(1);
    expect([made[0].width, made[0].height, made[0].rgba.length]).toEqual([3, 2, 24]);
    const alpha = channel(made[0].rgba, 3);
    expect(alpha).toEqual([0, 0, 23, 58, 92, 115]);
    cells.forEach((cell, i) => expect(Math.abs(alpha[i] / 255 - (cell / 255) * RED.opacity)).toBeLessThanOrEqual(0.5 / 255));
    expect(channel(made[0].rgba, 0)).toEqual(new Array(6).fill(255));
    expect(channel(made[0].rgba, 1)).toEqual(new Array(6).fill(0));
    expect(channel(made[0].rgba, 2)).toEqual(new Array(6).fill(0));
    expect(layer).toMatchObject({ id: MASK_LAYER_ID, kind: MASK_LAYER_KIND, visible: true, maskId: "m01", markedCells: 5 });

    const inverted = maskLayer({ maskId: "m01", preview: preview(3, 2, cells), dims: [3, 2], style: { ...RED, displayInvert: true } }, surface);
    const invertedAlpha = channel(made[1].rgba, 3);
    expect(invertedAlpha).toEqual([115, 114, 92, 57, 23, 0]);
    cells.forEach((cell, i) => expect(Math.abs(invertedAlpha[i] / 255 - (1 - cell / 255) * RED.opacity)).toBeLessThanOrEqual(0.5 / 255));
    expect(inverted?.markedCells).toBe(5);

    maskLayer({ maskId: "m02", preview: preview(3, 2, cells), dims: [3, 2], style: { opacity: 1, tint: [0, 200, 255], displayInvert: false } }, surface);
    expect(channel(made[2].rgba, 3)).toEqual(cells);
    expect(channel(made[2].rgba, 1)).toEqual(new Array(6).fill(200));

    expect(maskLayer({ maskId: "m01", preview: preview(3, 2, cells), dims: [3, 2], style: RED }, () => null)).toBeNull();
    expect(layer && isMaskLayer(layer)).toBe(true);
    expect(isMaskLayer({ id: "gaia-catalog", kind: "catalog", visible: true, paint: () => {} })).toBe(false);
  });

  it("maskPainter_affine_maps_the_image_corners_through_toScreen", () => {
    const dims: [number, number] = [4500, 600];
    const grid = { width: 2048, height: 273 };
    const mapping: RegionMapping = { transform: { scale: 0.75, x: 31, y: -12 }, renderW: 2048, renderH: 273, fitsW: 4500, fitsH: 600 };
    const scaled = (p: Pt): Pt => regionPointToScreen(p, mapping);
    const angle = Math.PI / 6;
    const pivot = { x: 400, y: 150 };
    const rotated = (p: Pt): Pt => {
      const q = scaled(p);
      const dx = q.x - pivot.x;
      const dy = q.y - pivot.y;
      return { x: pivot.x + dx * Math.cos(angle) - dy * Math.sin(angle), y: pivot.y + dx * Math.sin(angle) + dy * Math.cos(angle) };
    };
    const corners: [number, number, Pt][] = [
      [0, 0, { x: -0.5, y: -0.5 }],
      [grid.width, 0, { x: dims[0] - 0.5, y: -0.5 }],
      [0, grid.height, { x: -0.5, y: dims[1] - 0.5 }],
      [grid.width, grid.height, { x: dims[0] - 0.5, y: dims[1] - 0.5 }],
    ];

    for (const toScreen of [scaled, rotated]) {
      const { ctx, drawn } = fakeContext();
      paint(createMaskPainter(SURFACE, dims, grid), ctx, toScreen);
      expect(drawn).toHaveLength(1);
      expect(drawn[0].image).toBe(SURFACE);
      expect(drawn[0].at).toEqual([0, 0]);
      expect(drawn[0].smoothing).toBe(false);
      for (const [u, v, corner] of corners) {
        const mapped = apply(drawn[0].matrix, u, v);
        const expected = toScreen(corner);
        expect(mapped.x).toBeCloseTo(expected.x, 9);
        expect(mapped.y).toBeCloseTo(expected.y, 9);
      }
      expect(ctx.setTransform).not.toHaveBeenCalled();
      expect(ctx.save).toHaveBeenCalledTimes(1);
      expect(ctx.restore).toHaveBeenCalledTimes(1);
    }

    const { ctx, drawn } = fakeContext();
    paint(createMaskPainter(SURFACE, dims, grid), ctx, scaled);
    expect(drawn[0].matrix).toEqual([0.75, 0, 0, 0.75, 31, -12]);

    const lost = fakeContext();
    paint(createMaskPainter(SURFACE, dims, grid), lost.ctx, () => ({ x: Number.NaN, y: 0 }));
    expect(lost.drawn).toHaveLength(0);
  });
});
