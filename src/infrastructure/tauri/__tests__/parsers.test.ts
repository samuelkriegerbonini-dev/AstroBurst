import { describe, it, expect } from "vitest";
import { parseDqMaskBuffer, parseFftBuffer, parseRawPixelBuffer, parseRawRgbPixelBuffer, toUint8Array } from "../parsers";

function rawPixelPayload(width: number, height: number, min: number, max: number, pixels: number[]): ArrayBuffer {
  const buf = new ArrayBuffer(16 + pixels.length * 4);
  const view = new DataView(buf);
  view.setUint32(0, width, true);
  view.setUint32(4, height, true);
  view.setFloat32(8, min, true);
  view.setFloat32(12, max, true);
  pixels.forEach((v, i) => view.setFloat32(16 + i * 4, v, true));
  return buf;
}

function rawRgbPayload(
  width: number,
  height: number,
  ranges: [number, number, number, number, number, number],
  planes: [number[], number[], number[]],
  flag?: number,
): ArrayBuffer {
  const npix = width * height;
  const buf = new ArrayBuffer(32 + npix * 4 * 3 + (flag === undefined ? 0 : 1));
  const view = new DataView(buf);
  view.setUint32(0, width, true);
  view.setUint32(4, height, true);
  ranges.forEach((v, i) => view.setFloat32(8 + i * 4, v, true));
  planes.forEach((plane, p) => plane.forEach((v, i) => view.setFloat32(32 + (p * npix + i) * 4, v, true)));
  if (flag !== undefined) view.setUint8(32 + npix * 4 * 3, flag);
  return buf;
}

describe("parseRawPixelBuffer", () => {
  it("round-trips width, height, min, max and a 2x2 Float32 payload", () => {
    const parsed = parseRawPixelBuffer(rawPixelPayload(2, 2, -1.5, 8, [1, 2, 3, 4]));
    expect(parsed.width).toBe(2);
    expect(parsed.height).toBe(2);
    expect(parsed.dataMin).toBe(-1.5);
    expect(parsed.dataMax).toBe(8);
    expect(parsed.pixels).toBeInstanceOf(Float32Array);
    expect(Array.from(parsed.pixels)).toEqual([1, 2, 3, 4]);
  });

  it("throws when the buffer is shorter than the 16-byte header", () => {
    expect(() => parseRawPixelBuffer(new ArrayBuffer(15))).toThrow(/too small \(15 bytes\)/);
  });

  it("throws when the payload is shorter than width*height*4", () => {
    const buf = rawPixelPayload(2, 2, 0, 1, [1, 2, 3, 4]).slice(0, 16 + 3 * 4);
    expect(() => parseRawPixelBuffer(buf)).toThrow(/expected 32 bytes, got 28/);
  });

  it("copies the pixel block when the byte offset is not 4-aligned", () => {
    const src = new Uint8Array(rawPixelPayload(2, 2, 0, 1, [5, 6, 7, 8]));
    const padded = new Uint8Array(src.length + 1);
    padded.set(src, 1);
    const unaligned = new Uint8Array(padded.buffer, 1, src.length);
    const parsed = parseRawPixelBuffer(unaligned);
    expect(Array.from(parsed.pixels)).toEqual([5, 6, 7, 8]);
    expect(parsed.pixels.buffer).not.toBe(padded.buffer);
  });

  it("views the pixel block in place when the byte offset is 4-aligned", () => {
    const src = new Uint8Array(rawPixelPayload(1, 1, 0, 1, [9]));
    const parsed = parseRawPixelBuffer(src);
    expect(parsed.pixels.buffer).toBe(src.buffer);
    expect(parsed.pixels.byteOffset).toBe(16);
  });
});

describe("parseRawRgbPixelBuffer", () => {
  it("parses the planar layout with per-channel min/max", () => {
    const parsed = parseRawRgbPixelBuffer(
      rawRgbPayload(2, 1, [0, 10, 1, 20, 2, 30], [[1, 2], [3, 4], [5, 6]]),
    );
    expect(parsed.width).toBe(2);
    expect(parsed.height).toBe(1);
    expect(parsed.displayReferred).toBe(false);
    expect(Array.from(parsed.r.data)).toEqual([1, 2]);
    expect(Array.from(parsed.g.data)).toEqual([3, 4]);
    expect(Array.from(parsed.b.data)).toEqual([5, 6]);
    expect([parsed.r.min, parsed.r.max]).toEqual([0, 10]);
    expect([parsed.g.min, parsed.g.max]).toEqual([1, 20]);
    expect([parsed.b.min, parsed.b.max]).toEqual([2, 30]);
  });

  it("forces min/max to 0..1 when the displayReferred flag byte is 1", () => {
    const parsed = parseRawRgbPixelBuffer(
      rawRgbPayload(1, 1, [0, 10, 1, 20, 2, 30], [[0.5], [0.25], [0.75]], 1),
    );
    expect(parsed.displayReferred).toBe(true);
    for (const ch of [parsed.r, parsed.g, parsed.b]) {
      expect(ch.min).toBe(0);
      expect(ch.max).toBe(1);
    }
    expect(Array.from(parsed.r.data)).toEqual([0.5]);
  });

  it("keeps the raw ranges when the trailing flag byte is 0", () => {
    const parsed = parseRawRgbPixelBuffer(
      rawRgbPayload(1, 1, [0, 10, 1, 20, 2, 30], [[0.5], [0.25], [0.75]], 0),
    );
    expect(parsed.displayReferred).toBe(false);
    expect([parsed.r.min, parsed.r.max]).toEqual([0, 10]);
  });

  it("throws on a buffer shorter than the 32-byte header", () => {
    expect(() => parseRawRgbPixelBuffer(new ArrayBuffer(31))).toThrow(/too small \(31 bytes\)/);
  });

  it("throws when the planes are shorter than width*height*4*3", () => {
    const buf = rawRgbPayload(2, 1, [0, 1, 0, 1, 0, 1], [[1, 2], [3, 4], [5, 6]]).slice(0, 32 + 8 * 3 - 4);
    expect(() => parseRawRgbPixelBuffer(buf)).toThrow(/expected 56 bytes, got 52/);
  });

  it("copies planes when the byte offset is not 4-aligned", () => {
    const src = new Uint8Array(rawRgbPayload(1, 1, [0, 1, 0, 1, 0, 1], [[1], [2], [3]]));
    const padded = new Uint8Array(src.length + 3);
    padded.set(src, 3);
    const parsed = parseRawRgbPixelBuffer(new Uint8Array(padded.buffer, 3, src.length));
    expect(Array.from(parsed.r.data)).toEqual([1]);
    expect(Array.from(parsed.g.data)).toEqual([2]);
    expect(Array.from(parsed.b.data)).toEqual([3]);
    expect(parsed.b.data.buffer).not.toBe(padded.buffer);
  });
});

describe("parseFftBuffer", () => {
  function fftPayload(width: number, height: number, windowed: number, pixels: number[]): Uint8Array {
    const buf = new ArrayBuffer(32 + pixels.length);
    const view = new DataView(buf);
    view.setUint32(0, width, true);
    view.setUint32(4, height, true);
    view.setFloat32(8, 1.5, true);
    view.setFloat32(12, 2.5, true);
    view.setUint32(16, 42, true);
    view.setUint32(20, 1024, true);
    view.setUint32(24, windowed, true);
    new Uint8Array(buf, 32).set(pixels);
    return new Uint8Array(buf);
  }

  it("parses the header fields and exposes the pixel view", () => {
    const parsed = parseFftBuffer(fftPayload(2, 2, 1, [10, 20, 30, 40]));
    expect(parsed.width).toBe(2);
    expect(parsed.height).toBe(2);
    expect(parsed.dc_magnitude).toBe(1.5);
    expect(parsed.max_magnitude).toBe(2.5);
    expect(parsed.elapsed_ms).toBe(42);
    expect(parsed.original_size).toBe(1024);
    expect(parsed.windowed).toBe(true);
    expect(Array.from(parsed.pixels)).toEqual([10, 20, 30, 40]);
    expect(parsed.pixels.byteOffset).toBe(32);
  });

  it("reports windowed=false for a zero flag", () => {
    expect(parseFftBuffer(fftPayload(1, 1, 0, [7])).windowed).toBe(false);
  });

  it("throws on a short header and on a short pixel block", () => {
    expect(() => parseFftBuffer(new Uint8Array(31))).toThrow(/too small \(31 bytes\)/);
    expect(() => parseFftBuffer(fftPayload(2, 2, 0, [1, 2, 3]))).toThrow(/expected 36 bytes, got 35/);
  });
});

describe("parseDqMaskBuffer", () => {
  function dqPayload(width: number, height: number, mask: number, tableId: number, cells: number[]): ArrayBuffer {
    const buf = new ArrayBuffer(16 + cells.length);
    const view = new DataView(buf);
    view.setUint32(0, width, true);
    view.setUint32(4, height, true);
    view.setUint32(8, mask, true);
    view.setUint32(12, tableId, true);
    new Uint8Array(buf, 16).set(cells);
    return buf;
  }

  it("parses the header fields and exposes the cells view", () => {
    const parsed = parseDqMaskBuffer(dqPayload(3, 2, 7, 0, [1, 0, 1, 0, 0, 1]));
    expect(parsed.width).toBe(3);
    expect(parsed.height).toBe(2);
    expect(parsed.mask).toBe(7);
    expect(parsed.tableId).toBe(0);
    expect(parsed.cells).toBeInstanceOf(Uint8Array);
    expect(Array.from(parsed.cells)).toEqual([1, 0, 1, 0, 0, 1]);
    expect(parsed.cells.length).toBe(6);
  });

  it("reads mask bit 31 and the unknown table id as unsigned", () => {
    const parsed = parseDqMaskBuffer(dqPayload(1, 1, 0x80000001, 0xffffffff, [1]));
    expect(parsed.mask).toBe(2147483649);
    expect(parsed.tableId).toBe(4294967295);
  });

  it("views cells in place at an unaligned byte offset", () => {
    const src = new Uint8Array(dqPayload(2, 1, 1, 2, [1, 0]));
    const padded = new Uint8Array(src.length + 1);
    padded.set(src, 1);
    const parsed = parseDqMaskBuffer(new Uint8Array(padded.buffer, 1, src.length));
    expect(Array.from(parsed.cells)).toEqual([1, 0]);
    expect(parsed.cells.byteOffset).toBe(17);
  });

  it("throws on a short header and on a short cell block", () => {
    expect(() => parseDqMaskBuffer(new ArrayBuffer(15))).toThrow(/too small \(15 bytes\)/);
    expect(() => parseDqMaskBuffer(dqPayload(2, 2, 1, 0, [1, 1, 1]))).toThrow(/expected 20 bytes, got 19/);
  });

  it("accepts exactly 16 + w*h bytes", () => {
    const parsed = parseDqMaskBuffer(dqPayload(4, 4, 1, 1, new Array(16).fill(0)));
    expect(parsed.cells.length).toBe(16);
  });
});

describe("toUint8Array", () => {
  it("wraps an ArrayBuffer", () => {
    const out = toUint8Array(new Uint8Array([1, 2, 3]).buffer);
    expect(out).toBeInstanceOf(Uint8Array);
    expect(Array.from(out)).toEqual([1, 2, 3]);
  });

  it("returns a Uint8Array as-is", () => {
    const input = new Uint8Array([4, 5]);
    expect(toUint8Array(input)).toBe(input);
  });

  it("views a DataView without copying", () => {
    const buf = new Uint8Array([0, 6, 7, 8]).buffer;
    const out = toUint8Array(new DataView(buf, 1, 3));
    expect(out.buffer).toBe(buf);
    expect(out.byteOffset).toBe(1);
    expect(Array.from(out)).toEqual([6, 7, 8]);
  });

  it("converts a number[]", () => {
    expect(Array.from(toUint8Array([9, 255, 256]))).toEqual([9, 255, 0]);
  });

  it("throws a message naming the constructor for unsupported input", () => {
    class Weird {}
    expect(() => toUint8Array(new Weird())).toThrow("Unexpected IPC response type: object / Weird");
    expect(() => toUint8Array("nope")).toThrow("Unexpected IPC response type: string / String");
  });
});
