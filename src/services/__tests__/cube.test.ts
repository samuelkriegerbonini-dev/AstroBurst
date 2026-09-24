import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { releaseCubes } from "../cube";

describe("releaseCubes", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
  });

  it("releases each source file once, whatever plane of it was loaded", async () => {
    typedInvokeMock.mockResolvedValue(undefined);
    await releaseCubes(["C:/d/cube_s3d.fits", "C:/d/cube_s3d.fits#hdu=1", "C:/d/other.fits"]);
    expect(typedInvokeMock.mock.calls).toEqual([
      ["release_cube_cmd", { path: "C:/d/cube_s3d.fits" }],
      ["release_cube_cmd", { path: "C:/d/other.fits" }],
    ]);
  });

  it("keeps releasing the other files when one release fails", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    typedInvokeMock.mockRejectedValueOnce(new Error("gone")).mockResolvedValueOnce(undefined);
    await expect(releaseCubes(["a.fits", "b.fits"])).resolves.toBeUndefined();
    expect(typedInvokeMock).toHaveBeenCalledTimes(2);
    warn.mockRestore();
  });
});
