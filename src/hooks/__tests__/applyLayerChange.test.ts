import { describe, it, expect, beforeEach, vi } from "vitest";

const mocks = vi.hoisted(() => ({ forget: vi.fn() }));
vi.mock("../usePhotometryStatus", () => ({ photometryStatusStore: { forget: mocks.forget } }));

import { fileStore } from "../useFileStore";
import { applyLayerChange } from "../applyLayerChange";
import { pixelSourceVersion } from "../../utils/pixelSource";
import type { PlaneInfo, ProcessResult } from "../../shared/types";

const PLANE: PlaneInfo = {
  kind: "hdu",
  index: 1,
  key: null,
  extname: "IMAGE",
  extver: null,
  is_dq: false,
  is_err: false,
  dq_ref: "C:/d/hsc.fits#hdu=2",
  err_ref: "C:/d/hsc.fits#hdu=3",
  source_path: "C:/d/hsc.fits",
  dq_table: "unknown",
  dq_origin: "EXTNAME MASK, HDU 2",
};

function added(path: string): string {
  fileStore.addFiles([{ name: "hsc.fits", path, size: 0 }]);
  return fileStore.getFileIds()[fileStore.getFileIds().length - 1];
}

describe("applyLayerChange", () => {
  beforeEach(() => {
    fileStore.reset();
    mocks.forget.mockClear();
  });

  it("puts the new plane into the result of the file and counts one reload", () => {
    const id = added("C:/d/hsc.fits");
    const stats = { min: 0, max: 1, mean: 0.5, sigma: 1, median: 0.5 };
    const header = { BUNIT: "nJy" };
    const result = { png_path: "C:/d/hsc.png", previewUrl: "asset://hsc.png", dimensions: [4, 4], elapsed_ms: 1, stats, header, plane: { ...PLANE, dq_ref: null } } as ProcessResult;
    fileStore.fileDone(id, result);

    applyLayerChange(fileStore.getFile(id)!, PLANE);

    const after = fileStore.getFile(id)!;
    expect(after.result!.plane).toEqual(PLANE);
    expect(after.result!.stats).toBe(stats);
    expect(after.result!.header).toBe(header);
    expect(after.result!.previewUrl).toBe("asset://hsc.png");
    expect(after.reloads).toBe(1);
    expect(pixelSourceVersion(after, null, 0)).toBe(1);

    applyLayerChange(fileStore.getFile(id)!, PLANE);
    expect(fileStore.getFile(id)!.reloads).toBe(2);
  });

  it("leaves a file without a result alone", () => {
    const id = added("C:/d/queued.fits");
    expect(() => applyLayerChange(fileStore.getFile(id)!, PLANE)).not.toThrow();
    expect(fileStore.getFile(id)!.result).toBeNull();
    expect(fileStore.getFile(id)!.reloads).toBeUndefined();
  });

  it("tells the photometry status line that the layers of the file changed", () => {
    const id = added("C:/d/hsc.fits");
    const stats = { min: 0, max: 1, mean: 0.5, sigma: 1, median: 0.5 };
    fileStore.fileDone(id, { png_path: "C:/d/hsc.png", previewUrl: "asset://hsc.png", dimensions: [4, 4], elapsed_ms: 1, stats, plane: PLANE } as ProcessResult);

    applyLayerChange(fileStore.getFile(id)!, PLANE);

    expect(mocks.forget).toHaveBeenCalledTimes(1);
    expect(mocks.forget).toHaveBeenCalledWith(fileStore.getFile(id)!.path);
    expect(fileStore.getFile(id)!.path).toBe("C:/d/hsc.fits");

    mocks.forget.mockClear();
    const queued = added("C:/d/queued.fits");
    applyLayerChange(fileStore.getFile(queued)!, PLANE);
    expect(mocks.forget).not.toHaveBeenCalled();
  });
});
