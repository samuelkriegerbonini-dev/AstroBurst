import { describe, it, expect, beforeEach, vi } from "vitest";

const { processFitsFullMock, getFitsExtensionsMock, releaseCubesMock, probeSpectrum1dMock } = vi.hoisted(() => ({
  processFitsFullMock: vi.fn(),
  getFitsExtensionsMock: vi.fn(),
  releaseCubesMock: vi.fn(async () => {}),
  probeSpectrum1dMock: vi.fn(),
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
vi.mock("../../services/spectrum1d", () => ({ probeSpectrum1d: probeSpectrum1dMock }));

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

const ONE_D_REJECTION =
  "Failed to load C:/d/ha_spectrum.fits: HDU 0 (SCI): 1D array, not an image; " +
  "no HDU in this file holds a 2D image. " +
  "HDUs: [0] EXTNAME=SCI XTENSION=PRIMARY NAXIS=1 [2048] -- 1D array, not an image; " +
  "[1] EXTNAME=ERR XTENSION=IMAGE NAXIS=1 [2048] -- 1D array, not an image";

function queued(name: string) {
  fileStore.addFiles([{ name, path: `C:/d/${name}`, size: 1 }]);
  const files = fileStore.getFiles();
  return files[files.length - 1];
}

describe("ingestOutcome", () => {
  it("routes the real x1d rejection to the spectrum status", () => {
    expect(ingestOutcome(X1D_REJECTION)).toBe("spectrum");
  });

  it("routes the 1D array verdict to the spectrum status", () => {
    expect(ingestOutcome(ONE_D_REJECTION)).toBe("spectrum");
    expect(ingestOutcome("HDU 0 (PRIMARY): 1D array, not an image; no HDU in this file holds a 2D image. HDUs: []")).toBe("spectrum");
  });

  it("reads the verdict past a semicolon in the file path", () => {
    expect(ingestOutcome(X1D_REJECTION.replace("C:/d/jw_x1d.fits", "C:/data/run;2/jw_x1d.fits"))).toBe("spectrum");
    expect(ingestOutcome(ONE_D_REJECTION.replace("C:/d/ha_spectrum.fits", "C:/data/run;2/ha_spectrum.fits"))).toBe("spectrum");
  });

  it("keeps the cube fallback when the EXTRACT1D table only appears in the HDU inventory", () => {
    expect(ingestOutcome(CUBE_WITH_TABLE_REJECTION)).toBe("cube-fallback");
  });

  it("keeps the cube fallback for an ASDF or plain error", () => {
    expect(ingestOutcome(PLAIN_REJECTION)).toBe("cube-fallback");
    expect(ingestOutcome("HDU 1 (ASDF): BINTABLE extension, holds no image pixels")).toBe("cube-fallback");
  });
});

describe("settleIngestFailure", () => {
  beforeEach(() => {
    fileStore.reset();
    processFitsFullMock.mockReset();
    getFitsExtensionsMock.mockReset();
    probeSpectrum1dMock.mockReset();
  });

  it("lists an x1d as a spectrum with its probe, without scanning extensions and without counting a failure", async () => {
    const file = queued("a_x1d.fits");
    probeSpectrum1dMock.mockResolvedValue({ kind: "x1d", hdu: 1 });
    await settleIngestFailure(file, X1D_REJECTION);
    expect(probeSpectrum1dMock).toHaveBeenCalledWith("C:/d/a_x1d.fits");
    expect(getFitsExtensionsMock).not.toHaveBeenCalled();
    expect(fileStore.getFile(file.id)).toMatchObject({ status: "spectrum", error: null, spectrum: { kind: "x1d", hdu: 1 } });
    expect(fileStore.getStats()).toMatchObject({ total: 1, done: 0, failed: 0, spectra: 1 });
  });

  it("lists a 1D array file as a wcs1d spectrum on the HDU the probe found", async () => {
    const file = queued("ha_spectrum.fits");
    probeSpectrum1dMock.mockResolvedValue({ kind: "wcs1d", hdu: 0 });
    await settleIngestFailure(file, ONE_D_REJECTION);
    expect(fileStore.getFile(file.id)).toMatchObject({ status: "spectrum", spectrum: { kind: "wcs1d", hdu: 0 } });
    expect(fileStore.getStats()).toMatchObject({ failed: 0, spectra: 1 });
  });

  it("reports a probe error as a file error carrying the ingest message and the probe error", async () => {
    const file = queued("ha_spectrum.fits");
    probeSpectrum1dMock.mockRejectedValue(new Error("Failed to open C:/d/ha_spectrum.fits"));
    await settleIngestFailure(file, ONE_D_REJECTION);
    expect(getFitsExtensionsMock).not.toHaveBeenCalled();
    expect(fileStore.getFile(file.id)).toMatchObject({
      status: "error",
      error:
        "Failed to load C:/d/ha_spectrum.fits: HDU 0 (SCI): 1D array, not an image; " +
        "opening it as a 1D spectrum also failed: Failed to open C:/d/ha_spectrum.fits; " +
        "no HDU in this file holds a 2D image. " +
        "HDUs: [0] EXTNAME=SCI XTENSION=PRIMARY NAXIS=1 [2048] -- 1D array, not an image; " +
        "[1] EXTNAME=ERR XTENSION=IMAGE NAXIS=1 [2048] -- 1D array, not an image",
    });
    expect(fileStore.getFile(file.id)?.spectrum).toBeUndefined();
    expect(fileStore.getStats()).toMatchObject({ failed: 1, spectra: 0 });
  });

  it("falls back to the cube path when the probe finds no spectral axis on the 1D array", async () => {
    const file = queued("profile.fits");
    probeSpectrum1dMock.mockResolvedValue(null);
    getFitsExtensionsMock.mockResolvedValue({ extensions: [] });
    await settleIngestFailure(file, ONE_D_REJECTION);
    expect(getFitsExtensionsMock).toHaveBeenCalledWith("C:/d/profile.fits");
    expect(fileStore.getFile(file.id)).toMatchObject({ status: "error", error: ONE_D_REJECTION });
    expect(fileStore.getStats()).toMatchObject({ failed: 1, spectra: 0 });
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
    expect(probeSpectrum1dMock).not.toHaveBeenCalled();
    expect(processFitsFullMock).toHaveBeenCalledWith(`${file.path}#hdu=1`);
    expect(fileStore.getFile(file.id)?.status).toBe("done");
    expect(fileStore.getStats()).toMatchObject({ done: 1, failed: 0, spectra: 0 });
  });

  it("reports a plain failure as an error when no cube plane exists", async () => {
    const file = queued("broken.fits");
    getFitsExtensionsMock.mockResolvedValue({ extensions: [] });
    await settleIngestFailure(file, PLAIN_REJECTION);
    expect(probeSpectrum1dMock).not.toHaveBeenCalled();
    expect(fileStore.getFile(file.id)).toMatchObject({ status: "error", error: PLAIN_REJECTION });
    expect(fileStore.getStats()).toMatchObject({ done: 0, failed: 1, spectra: 0 });
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
