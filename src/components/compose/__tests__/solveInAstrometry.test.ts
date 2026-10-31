import { beforeEach, describe, expect, it, vi } from "vitest";

const { files, selectFile, dispatch } = vi.hoisted(() => ({
  files: { current: [] as { id: string; path: string; sourcePath: string }[] },
  selectFile: vi.fn(),
  dispatch: vi.fn(),
}));

vi.mock("../../../hooks/useFileStore", () => ({
  fileStore: { getFiles: () => files.current, selectFile },
}));

vi.mock("../../../hooks/useDockLayout", () => ({
  dockStore: { dispatch },
}));

import { solveInAstrometry } from "../solveInAstrometry";

describe("Solve R channel", () => {
  beforeEach(() => {
    selectFile.mockReset();
    dispatch.mockReset();
    files.current = [
      { id: "g-1", path: "C:\\data\\frame_0002.fits", sourcePath: "C:\\data\\frame_0002.fits" },
      { id: "r-1", path: "C:\\data\\frame_0001.fits", sourcePath: "C:\\data\\frame_0001.fits" },
    ];
  });

  it("selects the R file in Files and opens the Astrometry tool", () => {
    expect(solveInAstrometry("c:/data/FRAME_0001.fits")).toBe(true);
    expect(selectFile).toHaveBeenCalledWith("r-1");
    expect(dispatch).toHaveBeenCalledWith({ type: "open", tool: "astrometry" });
  });

  it("finds a file whose view switched to another image ref by its source path", () => {
    files.current = [{ id: "r-2", path: "C:\\data\\frame_0001.fits#hdu=1", sourcePath: "C:\\data\\frame_0001.fits" }];
    expect(solveInAstrometry("C:\\data\\frame_0001.fits")).toBe(true);
    expect(selectFile).toHaveBeenCalledWith("r-2");
  });

  it("opens nothing when the R file is not in Files, so Astrometry is not left on another image", () => {
    expect(solveInAstrometry("C:\\data\\gone.fits")).toBe(false);
    expect(selectFile).not.toHaveBeenCalled();
    expect(dispatch).not.toHaveBeenCalled();
  });
});
