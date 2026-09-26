import { describe, it, expect } from "vitest";
import { parseFftGrid } from "../fftHeader";

function header(fields: { cols: number; rows: number; paddedCols: number; paddedRows: number; flags: number }): Uint8Array {
  const bytes = new Uint8Array(40 + fields.cols * fields.rows);
  const view = new DataView(bytes.buffer);
  view.setUint32(0, fields.cols, true);
  view.setUint32(4, fields.rows, true);
  view.setFloat32(8, 12.5, true);
  view.setFloat32(12, 20, true);
  view.setUint32(16, 42, true);
  view.setUint32(20, fields.paddedCols, true);
  view.setUint32(24, fields.paddedRows, true);
  view.setUint32(28, fields.flags, true);
  return bytes;
}

describe("parseFftGrid", () => {
  it("reads the padded grid per axis and the downsampled flag", () => {
    const grid = parseFftGrid(header({ cols: 16, rows: 8, paddedCols: 8192, paddedRows: 4096, flags: 3 }));
    expect(grid).toEqual({ grid_width: 8192, grid_height: 4096, windowed: true, downsampled: true });
  });

  it("reports a tall grid that was not downsampled", () => {
    const grid = parseFftGrid(header({ cols: 32, rows: 512, paddedCols: 32, paddedRows: 512, flags: 1 }));
    expect(grid).toEqual({ grid_width: 32, grid_height: 512, windowed: true, downsampled: false });
  });

  it("reads each flag bit on its own", () => {
    expect(parseFftGrid(header({ cols: 1, rows: 1, paddedCols: 1, paddedRows: 1, flags: 0 }))).toMatchObject({
      windowed: false,
      downsampled: false,
    });
    expect(parseFftGrid(header({ cols: 1, rows: 1, paddedCols: 1, paddedRows: 1, flags: 2 }))).toMatchObject({
      windowed: false,
      downsampled: true,
    });
  });

  it("honours the byte offset of a view into a larger buffer", () => {
    const inner = header({ cols: 2, rows: 2, paddedCols: 64, paddedRows: 128, flags: 1 });
    const outer = new Uint8Array(inner.length + 5);
    outer.set(inner, 5);
    expect(parseFftGrid(outer.subarray(5))).toMatchObject({ grid_width: 64, grid_height: 128 });
  });

  it("rejects a response shorter than the header", () => {
    expect(() => parseFftGrid(new Uint8Array(39))).toThrow(/too small \(39 bytes\)/);
  });
});
