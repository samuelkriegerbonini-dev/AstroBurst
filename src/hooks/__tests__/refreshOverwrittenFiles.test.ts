import { describe, it, expect, beforeEach, vi } from "vitest";

const { processFitsFullMock } = vi.hoisted(() => ({ processFitsFullMock: vi.fn() }));

vi.mock("../../services/fits", () => ({ processFitsFull: processFitsFullMock }));

import { fileStore } from "../useFileStore";
import { refreshOverwrittenFiles } from "../refreshOverwrittenFiles";
import type { ProcessResult } from "../../shared/types";

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
});
