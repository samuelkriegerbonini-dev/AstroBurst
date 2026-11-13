import { afterEach, describe, expect, it, vi } from "vitest";
import { devicePixelRatioOrOne, sizeCanvas } from "../canvasDpr";

afterEach(() => {
  vi.unstubAllGlobals();
});

function fakeCanvas() {
  const transforms: number[][] = [];
  const ctx = { setTransform: (...args: number[]) => transforms.push(args) };
  const canvas = { width: 0, height: 0, getContext: () => ctx };
  return { canvas: canvas as unknown as HTMLCanvasElement, ctx, transforms };
}

describe("sizeCanvas", () => {
  it("sizes the backing store by the pixel ratio and draws in CSS pixels", () => {
    const double = fakeCanvas();
    expect(sizeCanvas(double.canvas, 400, 180, 2)).toBe(double.ctx);
    expect(double.canvas.width).toBe(800);
    expect(double.canvas.height).toBe(360);
    expect(double.transforms).toEqual([[2, 0, 0, 2, 0, 0]]);
    const scaled = fakeCanvas();
    sizeCanvas(scaled.canvas, 400, 180, 1.25);
    expect(scaled.canvas.width).toBe(500);
    expect(scaled.canvas.height).toBe(225);
    expect(scaled.transforms).toEqual([[1.25, 0, 0, 1.25, 0, 0]]);
    const blind = { width: 0, height: 0, getContext: () => null } as unknown as HTMLCanvasElement;
    expect(sizeCanvas(blind, 400, 180, 2)).toBeNull();
    expect(blind.width).toBe(800);
  });
});

describe("devicePixelRatioOrOne", () => {
  it("falls back to one without a window or with a broken ratio", () => {
    expect(typeof window).toBe("undefined");
    expect(devicePixelRatioOrOne()).toBe(1);
    vi.stubGlobal("window", { devicePixelRatio: 1.5 });
    expect(devicePixelRatioOrOne()).toBe(1.5);
    for (const broken of [0, -2, NaN, Infinity, undefined]) {
      vi.stubGlobal("window", { devicePixelRatio: broken });
      expect(devicePixelRatioOrOne()).toBe(1);
    }
  });
});
