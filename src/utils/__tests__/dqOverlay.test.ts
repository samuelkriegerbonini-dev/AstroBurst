import { describe, it, expect } from "vitest";
import { paintDqMask, DQ_OVERLAY_RGB, DQ_OVERLAY_ALPHA } from "../dqOverlay";

describe("paintDqMask", () => {
  it("writes rgba only for cells equal to 1", () => {
    const cells = new Uint8Array([1, 0, 0, 1]);
    const out = new Uint8ClampedArray(2 * 2 * 4);
    paintDqMask(cells, 2, 2, [255, 64, 64], 0.45, out);
    expect(out.length).toBe(16);
    expect(Array.from(out.subarray(0, 4))).toEqual([255, 64, 64, Math.round(255 * 0.45)]);
    expect(Array.from(out.subarray(4, 8))).toEqual([0, 0, 0, 0]);
    expect(Array.from(out.subarray(8, 12))).toEqual([0, 0, 0, 0]);
    expect(Array.from(out.subarray(12, 16))).toEqual([255, 64, 64, Math.round(255 * 0.45)]);
  });

  it("clears stale bytes for zero cells", () => {
    const cells = new Uint8Array([0]);
    const out = new Uint8ClampedArray([9, 9, 9, 9]);
    paintDqMask(cells, 1, 1, [1, 2, 3], 1, out);
    expect(Array.from(out)).toEqual([0, 0, 0, 0]);
  });

  it("rounds the alpha byte", () => {
    const out = new Uint8ClampedArray(4);
    paintDqMask(new Uint8Array([1]), 1, 1, [10, 20, 30], 0.5, out);
    expect(out[3]).toBe(128);
    paintDqMask(new Uint8Array([1]), 1, 1, [10, 20, 30], 1, out);
    expect(out[3]).toBe(255);
  });

  it("exposes the default overlay colour and alpha", () => {
    expect(DQ_OVERLAY_RGB).toEqual([255, 64, 64]);
    expect(DQ_OVERLAY_ALPHA).toBe(0.45);
  });

  it("throws when the output buffer is too small", () => {
    expect(() => paintDqMask(new Uint8Array(4), 2, 2, [1, 1, 1], 1, new Uint8ClampedArray(8))).toThrow(/16 bytes/);
  });
});
