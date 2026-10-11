import { describe, it, expect, beforeEach, vi } from "vitest";

const { processFitsFullMock, getFitsExtensionsMock, releaseCubesMock } = vi.hoisted(() => ({
  processFitsFullMock: vi.fn(),
  getFitsExtensionsMock: vi.fn(),
  releaseCubesMock: vi.fn(async () => {}),
}));

vi.mock("../../services/fits", () => ({
  processFitsFull: processFitsFullMock,
  processFits: vi.fn(),
  resampleFits: vi.fn(),
}));
vi.mock("../../services/header", () => ({
  getHeader: vi.fn(),
  getFitsExtensions: getFitsExtensionsMock,
}));
vi.mock("../../services/cube", () => ({ releaseCubes: releaseCubesMock }));

import { fileStore } from "../useFileStore";
import { ingestOutcome, pathToRelease, releaseCubesOnSwitch, settleIngestFailure } from "../useFileQueue";

const X1D_REJECTION =
  "Failed to load C:/d/jw_x1d.fits: HDU 1 (EXTRACT1D): BINTABLE extension, holds no image pixels; " +
  "no HDU in this file holds a 2D image. " +
  "HDUs: [0] EXTNAME=(none) XTENSION=PRIMARY NAXIS=0 -- header only, no pixel data; " +
  "[1] EXTNAME=EXTRACT1D XTENSION=BINTABLE NAXIS=2 [232x1024] -- BINTABLE extension, holds no image pixels";

const CUBE_WITH_TABLE_REJECTION =
  "Failed to load C:/d/odd_s3d.fits: HDU 1 (SCI): 3D cube of 40 planes, not a single 2D image; " +
  "no HDU in this file holds a 2D image. " +
  "HDUs: [0] EXTNAME=(none) XTENSION=PRIMARY NAXIS=0 -- header only, no pixel data; " +
  "[1] EXTNAME=SCI XTENSION=IMAGE NAXIS=3 [50x60x40] -- 3D cube of 40 planes, not a single 2D image; " +
  "[2] EXTNAME=EXTRACT1D XTENSION=BINTABLE NAXIS=2 [232x1024] -- BINTABLE extension, holds no image pixels";

const PLAIN_REJECTION = "Failed to load C:/d/broken.fits: No such file or directory (os error 2)";

function queued(name: string) {
  fileStore.addFiles([{ name, path: `C:/d/${name}`, size: 1 }]);
  const files = fileStore.getFiles();
  return files[files.length - 1];
}

describe("ingestOutcome", () => {
  it("routes the real x1d rejection to the table status", () => {
    expect(ingestOutcome(X1D_REJECTION)).toBe("table");
  });

  it("keeps the cube fallback when the EXTRACT1D table only appears in the HDU inventory", () => {
    expect(ingestOutcome(CUBE_WITH_TABLE_REJECTION)).toBe("cube-fallback");
  });

  it("keeps the cube fallback for a non-table error", () => {
    expect(ingestOutcome(PLAIN_REJECTION)).toBe("cube-fallback");
    expect(ingestOutcome("HDU 1 (ASDF): BINTABLE extension, holds no image pixels")).toBe("cube-fallback");
  });
});

describe("settleIngestFailure", () => {
  beforeEach(() => {
    fileStore.reset();
    processFitsFullMock.mockReset();
    getFitsExtensionsMock.mockReset();
  });

  it("lists an x1d as a table without scanning extensions and without counting a failure", async () => {
    const file = queued("a_x1d.fits");
    await settleIngestFailure(file, X1D_REJECTION);
    expect(getFitsExtensionsMock).not.toHaveBeenCalled();
    expect(fileStore.getFile(file.id)).toMatchObject({ status: "table", error: X1D_REJECTION });
    expect(fileStore.getStats()).toMatchObject({ total: 1, done: 0, failed: 0, tables: 1 });
  });

  it("still loads the first cube plane of a cube that also carries an EXTRACT1D table", async () => {
    const file = queued("odd_s3d.fits");
    getFitsExtensionsMock.mockResolvedValue({
      extensions: [
        { index: 0, extname: null, naxis: 0, naxis3: 0, has_data: false, ref: file.path },
        { index: 1, extname: "SCI", naxis: 3, naxis3: 40, has_data: true, ref: `${file.path}#hdu=1` },
        { index: 2, extname: "EXTRACT1D", naxis: 2, naxis3: 0, has_data: true, ref: `${file.path}#hdu=2` },
      ],
    });
    processFitsFullMock.mockResolvedValue({ dimensions: [50, 60], elapsed_ms: 1 });
    await settleIngestFailure(file, CUBE_WITH_TABLE_REJECTION);
    expect(processFitsFullMock).toHaveBeenCalledWith(`${file.path}#hdu=1`);
    expect(fileStore.getFile(file.id)?.status).toBe("done");
    expect(fileStore.getStats()).toMatchObject({ done: 1, failed: 0, tables: 0 });
  });

  it("reports a plain failure as an error when no cube plane exists", async () => {
    const file = queued("broken.fits");
    getFitsExtensionsMock.mockResolvedValue({ extensions: [] });
    await settleIngestFailure(file, PLAIN_REJECTION);
    expect(fileStore.getFile(file.id)).toMatchObject({ status: "error", error: PLAIN_REJECTION });
    expect(fileStore.getStats()).toMatchObject({ done: 0, failed: 1, tables: 0 });
  });
});

const CUBE = { path: "C:/d/cube_s3d.fits#hdu=1", sourcePath: "C:/d/cube_s3d.fits" };
const IMAGE = { path: "C:/d/502nmos.fits", sourcePath: "C:/d/502nmos.fits" };
const PLANE_RESULT = { png_path: "C:/o/cube.png", previewUrl: "asset://localhost/C%3A%2Fo%2Fcube.png", dimensions: [50, 60] as [number, number], elapsed_ms: 1 };

function settled(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

describe("pathToRelease", () => {
  it("releases the source file of the previous selection when another file is selected", () => {
    expect(pathToRelease(CUBE, IMAGE)).toBe("C:/d/cube_s3d.fits");
  });

  it("keeps the cube mapped when only its HDU or plane changes", () => {
    expect(pathToRelease(CUBE, { path: "C:/d/cube_s3d.fits#hdu=2", sourcePath: CUBE.sourcePath })).toBeNull();
    expect(pathToRelease({ path: CUBE.sourcePath, sourcePath: CUBE.sourcePath }, CUBE)).toBeNull();
  });

  it("falls back to the path without a source path and releases nothing without a previous selection", () => {
    expect(pathToRelease({ path: "C:/d/a.fits", sourcePath: "" }, null)).toBe("C:/d/a.fits");
    expect(pathToRelease(null, CUBE)).toBeNull();
  });
});

describe("releaseCubesOnSwitch", () => {
  beforeEach(async () => {
    fileStore.reset();
    await settled();
    releaseCubesMock.mockClear();
  });

  it("releases the previous cube on a file switch and not on a plane switch of the same cube", async () => {
    const cube = queued("cube_s3d.fits");
    const image = queued("502nmos.fits");
    fileStore.selectFile(cube.id);
    await settled();
    const stop = releaseCubesOnSwitch();

    fileStore.switchImageRef(cube.id, `${cube.path}#hdu=1`, PLANE_RESULT);
    await settled();
    expect(releaseCubesMock).not.toHaveBeenCalled();

    fileStore.selectFile(image.id);
    await settled();
    expect(releaseCubesMock).toHaveBeenCalledTimes(1);
    expect(releaseCubesMock).toHaveBeenCalledWith(["C:/d/cube_s3d.fits"]);

    stop();
    fileStore.selectFile(cube.id);
    await settled();
    expect(releaseCubesMock).toHaveBeenCalledTimes(1);
  });
});
