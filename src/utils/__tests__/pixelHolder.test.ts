import { describe, it, expect } from "vitest";
import { makePixelHolder, rawPixelsHolder } from "../pixelHolder";
import { parseRawRgbPixelBuffer } from "../../infrastructure/tauri/parsers";

function enumerated(obj: object): string[] {
  const keys: string[] = [];
  for (const k in obj) if (Object.prototype.hasOwnProperty.call(obj, k)) keys.push(k);
  return keys;
}

describe("makePixelHolder", () => {
  const pixels = new Float32Array(2048 * 1211);

  it("keeps the pixel array reachable through .data but out of every enumeration", () => {
    const holder = makePixelHolder({ width: 2048, height: 1211, min: -1, max: 9 }, pixels);
    expect(holder.data).toBe(pixels);
    expect(holder.width).toBe(2048);
    expect(holder.height).toBe(1211);
    expect(holder.min).toBe(-1);
    expect(holder.max).toBe(9);
    expect(Object.keys(holder)).toEqual(["width", "height", "min", "max"]);
    expect(enumerated(holder)).not.toContain("data");
    expect(JSON.stringify(holder).length).toBeLessThan(300);
  });

  it("does not leak the array through a spread copy", () => {
    const holder = makePixelHolder({ min: 0, max: 1 }, pixels);
    expect({ ...holder }).toEqual({ min: 0, max: 1 });
  });

  it("does not mutate the metadata object it was given", () => {
    const meta = { min: 0, max: 1 };
    const holder = makePixelHolder(meta, pixels);
    expect(holder).not.toBe(meta);
    expect("data" in meta).toBe(false);
  });

  it("accepts a later data assignment and keeps it non-enumerable", () => {
    const holder: { data: Float32Array; min: number } = makePixelHolder({ min: 0 }, pixels);
    const next = new Float32Array(4);
    expect(() => { holder.data = next; }).not.toThrow();
    expect(holder.data).toBe(next);
    expect(Object.keys(holder)).toEqual(["min"]);
  });
});

describe("rawPixelsHolder", () => {
  it("maps the parsed mono result to a holder whose data is hidden from enumeration", () => {
    const pixels = new Float32Array(6);
    const result = { width: 3, height: 2, dataMin: -2, dataMax: 7, pixels };
    const holder = rawPixelsHolder(result);
    expect(Object.keys(holder)).toEqual(["width", "height", "min", "max"]);
    expect(holder.data).toBe(result.pixels);
    expect(holder.min).toBe(-2);
    expect(holder.max).toBe(7);
    expect(JSON.stringify(holder)).toBe('{"width":3,"height":2,"min":-2,"max":7}');
  });
});

describe("parseRawRgbPixelBuffer planes", () => {
  it("expose data as a non-enumerable own property", () => {
    const npix = 4 * 3;
    const buf = new ArrayBuffer(32 + npix * 4 * 3);
    const view = new DataView(buf);
    view.setUint32(0, 4, true);
    view.setUint32(4, 3, true);
    for (let i = 0; i < npix * 3; i++) view.setFloat32(32 + i * 4, i + 0.5, true);
    const parsed = parseRawRgbPixelBuffer(buf);
    for (const plane of [parsed.r, parsed.g, parsed.b]) {
      expect(Object.keys(plane)).toEqual(["min", "max"]);
      expect(enumerated(plane)).not.toContain("data");
      expect(plane.data.length).toBe(npix);
    }
    expect(parsed.r.data[0]).toBe(0.5);
    expect(parsed.b.data[npix - 1]).toBe(npix * 3 - 0.5);
    expect(JSON.stringify(parsed).length).toBeLessThan(300);
  });
});
