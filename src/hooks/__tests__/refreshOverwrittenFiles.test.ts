import { describe, it, expect, beforeEach, vi } from "vitest";

const { processFitsFullMock } = vi.hoisted(() => ({ processFitsFullMock: vi.fn() }));

vi.mock("../../services/fits", () => ({ processFitsFull: processFitsFullMock }));

import { fileStore } from "../useFileStore";
import { refreshOverwrittenFiles } from "../refreshOverwrittenFiles";
import { registerFileIngest } from "../useFileIngest";
import { fileKeyOf } from "../../context/PreviewContext";
import { pixelSourceVersion } from "../../utils/pixelSource";
import type { ProcessedFile, ProcessResult } from "../../shared/types";

function loaded(name: string, path: string, median: number): string {
  fileStore.addFiles([{ name, path, size: 0 }]);
  const id = fileStore.getFileIds()[fileStore.getFileIds().length - 1];
  const result = { png_path: `${path}.png`, previewUrl: `asset://${name}.png`, dimensions: [4, 4], elapsed_ms: 1, stats: { min: 0, max: 1, mean: median, sigma: 1, median } } as ProcessResult;
  fileStore.fileDone(id, result);
  return id;
}

describe("refreshOverwrittenFiles", () => {
  beforeEach(() => {
    fileStore.reset();
    processFitsFullMock.mockReset();
  });

  it("reloads a loaded file that a re-run step rewrote on disk, so its stats and preview describe the new data", async () => {
    const source = loaded("M42.fits", "C:/raw/M42.fits", 10);
    const output = loaded("M42_arcsinh.fits", "C:/out/M42_arcsinh.fits", 0.2);
    processFitsFullMock.mockResolvedValue({
      png_path: "C:/out/M42_arcsinh.png",
      previewUrl: "asset://M42_arcsinh.png",
      dimensions: [4, 4],
      elapsed_ms: 1,
      stats: { min: 0, max: 1, mean: 0.6, sigma: 1, median: 0.6 },
    });

    const refreshed = await refreshOverwrittenFiles({ fits_path: "C:/out/M42_arcsinh.fits", png_path: "C:/out/M42_arcsinh.png" });

    expect(refreshed).toEqual([output]);
    expect(processFitsFullMock).toHaveBeenCalledWith("C:/out/M42_arcsinh.fits");
    expect(fileStore.getFile(output)?.result?.stats?.median).toBe(0.6);
    expect(fileStore.getFile(output)?.result?.previewUrl).toMatch(/^asset:\/\/M42_arcsinh\.png\?v=\d+$/);
    expect(fileStore.getFile(source)?.result?.stats?.median).toBe(10);
  });

  it("gives a rewritten file that is open in the viewer a new pixel version, so the GPU pixels, scale limits and DQ mask are fetched again", async () => {
    const PATH = "C:/data/mast/JWST/2739/xc1_i2d.fits";
    const id = loaded("xc1_i2d.fits", PATH, 128);
    fileStore.selectFile(id);
    const before = fileStore.getFile(id) as ProcessedFile;
    processFitsFullMock.mockResolvedValue({
      png_path: "C:/out/xc1_i2d.png",
      previewUrl: "asset://xc1_i2d.png",
      dimensions: [256, 256],
      elapsed_ms: 1,
      stats: { min: 1000, max: 1510, mean: 1255, sigma: 147, median: 1255 },
    });

    expect(await refreshOverwrittenFiles({ output_path: PATH })).toEqual([id]);

    const after = fileStore.getFile(id) as ProcessedFile;
    expect(after.result?.stats?.mean).toBe(1255);
    expect(fileKeyOf(after)).toBe(fileKeyOf(before));
    expect(fileStore.getSelected()).toBe(id);
    expect(pixelSourceVersion(after, null, 0)).not.toBe(pixelSourceVersion(before, null, 0));
  });

  it("does nothing when the step wrote no loaded file", async () => {
    loaded("M42.fits", "C:/raw/M42.fits", 10);
    expect(await refreshOverwrittenFiles({ fits_path: "C:/out/M42_deconv.fits" })).toEqual([]);
    expect(processFitsFullMock).not.toHaveBeenCalled();
  });

  it("keeps the old state when the reload fails", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const output = loaded("M42_arcsinh.fits", "C:/out/M42_arcsinh.fits", 0.2);
    processFitsFullMock.mockRejectedValue(new Error("locked"));
    await refreshOverwrittenFiles({ fits_path: "C:/out/M42_arcsinh.fits" });
    expect(fileStore.getFile(output)?.result?.stats?.median).toBe(0.2);
    warn.mockRestore();
  });

  it("requeues a written output whose row errored and kicks the queue, without a second row", async () => {
    const kicks: string[][] = [];
    const unregister = registerFileIngest((files) => {
      kicks.push(files.map((f) => f.path));
      fileStore.addFiles(files);
    });
    try {
      fileStore.addFiles([{ name: "M42_pm_out.fits", path: "C:\\out\\M42_pm_out.fits", size: 0 }]);
      const [row] = fileStore.getFiles();
      fileStore.fileError(row.id, "No such file");

      expect(await refreshOverwrittenFiles({ fits_path: "C:/out/M42_pm_out.fits" })).toEqual([]);

      expect(fileStore.getFile(row.id)).toMatchObject({ status: "queued", error: null });
      expect(fileStore.getFiles()).toHaveLength(1);
      expect(fileStore.getStats()).toMatchObject({ total: 1, failed: 0 });
      expect(kicks).toEqual([["C:/out/M42_pm_out.fits"]]);
      expect(processFitsFullMock).not.toHaveBeenCalled();
    } finally {
      unregister();
    }
  });
});
