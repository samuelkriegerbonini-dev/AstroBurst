import { describe, it, expect, beforeEach, vi } from "vitest";

const { processFitsFullMock, getFitsExtensionsMock, releaseCubesMock, probeSpectrum1dMock, probeTableMock } = vi.hoisted(() => ({
  processFitsFullMock: vi.fn(),
  getFitsExtensionsMock: vi.fn(),
  releaseCubesMock: vi.fn(async () => {}),
  probeSpectrum1dMock: vi.fn(),
  probeTableMock: vi.fn(),
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
vi.mock("../../services/tables", () => ({ probeTable: probeTableMock }));

import { fileStore } from "../useFileStore";
import { alreadyInFilesMessage, ingestOutcome, ingestRoute, pathToRelease, releaseCubesOnSwitch, settleIngestFailure, settleTextTable } from "../useFileQueue";
import type { TableProbe, TableProbeColumn } from "../../shared/types/tables";

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

const CATALOG_REJECTION =
  "Failed to load C:/d/cat.fits: HDU 1 (CATALOG): BINTABLE extension, holds no image pixels; " +
  "no HDU in this file holds a 2D image. " +
  "HDUs: [0] EXTNAME=(none) XTENSION=PRIMARY NAXIS=0 -- header only, no pixel data; " +
  "[1] EXTNAME=CATALOG XTENSION=BINTABLE NAXIS=2 [64x500] -- BINTABLE extension, holds no image pixels";

function probeColumns(names: string[]): TableProbeColumn[] {
  return names.map((name) => ({ name, unit: null, datatype: "float64", numeric: true }));
}

function tableProbe(overrides: Partial<TableProbe>): TableProbe {
  return {
    format: "csv",
    n_rows: 10,
    columns: probeColumns(["a", "b"]),
    tables: null,
    meta: null,
    comments: [],
    guess: { kind: "unknown", reason: "no column set of a spectrum, a light curve or a catalogue" },
    notes: [],
    ...overrides,
  };
}

const CATALOG_PROBE = tableProbe({
  format: "fits_bintable",
  n_rows: 500,
  columns: probeColumns(["RA", "DEC", "FLUX_G", "FLUX_R", "FLUX_Z", "TYPE", "MASKBITS", "SHAPE_R"]),
  tables: [{ hdu: 1, extname: "CATALOG", n_rows: 500, n_columns: 8 }],
  guess: { kind: "catalogue", reason: "ra and dec columns" },
});

describe("ingestRoute", () => {
  it("routes text tables away from the FITS reader", () => {
    expect(ingestRoute("C:/d/C1_ls_dr10_box_A.csv")).toBe("text-table");
    expect(ingestRoute("C:/d/x_whtlt.ecsv")).toBe("text-table");
    expect(ingestRoute("C:/D/X_WHTLT.ECSV")).toBe("text-table");
    expect(ingestRoute("C:/d/m42.fits")).toBe("fits");
    expect(ingestRoute("C:/d/frame-r-001462-4-0165.fits.bz2")).toBe("fits");
  });
});

describe("settleTextTable", () => {
  beforeEach(() => {
    fileStore.reset();
    processFitsFullMock.mockReset();
    probeTableMock.mockReset();
  });

  it("lists the app's CSV spectrum as a spectrum and every other text table as a table", async () => {
    const csvSpectrum = queued("spec.csv");
    const spectrumProbe = tableProbe({ columns: probeColumns(["wavelength_vacuum_um", "flux"]), guess: { kind: "spectrum", reason: "columns wavelength_vacuum_um and flux: a spectrum exported by AstroBurst" } });
    probeTableMock.mockResolvedValueOnce(spectrumProbe);
    await settleTextTable(csvSpectrum);
    expect(probeTableMock).toHaveBeenLastCalledWith("C:/d/spec.csv");
    expect(fileStore.getFile(csvSpectrum.id)).toMatchObject({ status: "spectrum", error: null, table: spectrumProbe });
    expect(fileStore.getFile(csvSpectrum.id)?.spectrum).toBeUndefined();

    const cases: [string, Partial<TableProbe>][] = [
      ["x_whtlt.ecsv", { format: "ecsv", guess: { kind: "light_curve", reason: "time column MJD_UTC and value column whitelight_flux" } }],
      ["C1_ls_dr10_box_A.csv", { format: "csv", guess: { kind: "catalogue", reason: "sky position columns ra and dec" } }],
      ["exported.ecsv", { format: "ecsv", guess: { kind: "spectrum", reason: "columns wavelength_vacuum_um and flux: a spectrum exported by AstroBurst" } }],
    ];
    for (const [name, overrides] of cases) {
      const file = queued(name);
      const probe = tableProbe(overrides);
      probeTableMock.mockResolvedValueOnce(probe);
      await settleTextTable(file);
      expect(fileStore.getFile(file.id), name).toMatchObject({ status: "table", error: null, table: probe });
    }

    const broken = queued("broken.csv");
    probeTableMock.mockRejectedValueOnce(new Error("the CSV header row is missing"));
    await settleTextTable(broken);
    expect(fileStore.getFile(broken.id)).toMatchObject({ status: "error", error: "the CSV header row is missing" });
    expect(fileStore.getStats()).toMatchObject({ total: 5, done: 0, failed: 1, spectra: 1, tables: 3 });
    expect(processFitsFullMock).not.toHaveBeenCalled();
  });
});

describe("settleIngestFailure", () => {
  beforeEach(() => {
    fileStore.reset();
    processFitsFullMock.mockReset();
    getFitsExtensionsMock.mockReset();
    probeSpectrum1dMock.mockReset();
    probeTableMock.mockReset();
    probeTableMock.mockRejectedValue(new Error("not a table"));
  });

  it("turns a FITS file that holds only tables into a table row", async () => {
    const file = queued("cat.fits");
    getFitsExtensionsMock.mockResolvedValue({ extensions: [] });
    probeTableMock.mockResolvedValue(CATALOG_PROBE);
    await settleIngestFailure(file, CATALOG_REJECTION);
    expect(probeTableMock).toHaveBeenCalledWith("C:/d/cat.fits");
    expect(probeSpectrum1dMock).not.toHaveBeenCalled();
    expect(fileStore.getFile(file.id)).toMatchObject({ status: "table", error: null, table: CATALOG_PROBE });
    expect(fileStore.getStats()).toMatchObject({ total: 1, done: 0, failed: 0, spectra: 0, tables: 1 });
  });

  it("keeps the FITS rejection when the table probe lists no binary table or fails", async () => {
    getFitsExtensionsMock.mockResolvedValue({ extensions: [] });
    const empty = queued("cat.fits");
    probeTableMock.mockResolvedValueOnce({ ...CATALOG_PROBE, tables: [] });
    await settleIngestFailure(empty, CATALOG_REJECTION);
    expect(fileStore.getFile(empty.id)).toMatchObject({ status: "error", error: CATALOG_REJECTION });

    fileStore.reset();
    const refused = queued("cat.fits");
    probeTableMock.mockRejectedValueOnce(new Error("C:/d/cat.fits holds no binary table"));
    await settleIngestFailure(refused, CATALOG_REJECTION);
    expect(fileStore.getFile(refused.id)).toMatchObject({ status: "error", error: CATALOG_REJECTION });
    expect(fileStore.getStats()).toMatchObject({ failed: 1, tables: 0 });
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

  it("keeps the time-series probe on the row as the backend gave it", async () => {
    const file = queued("jw02358_x1dints.fits");
    const series = { kind: "x1dints", layout: "rows", n_series: 1, n_integrations: 240, n_channels: 431 };
    probeSpectrum1dMock.mockResolvedValue(series);
    expect(ingestOutcome(X1D_REJECTION)).toBe("spectrum");
    await settleIngestFailure(file, X1D_REJECTION);
    expect(fileStore.getFile(file.id)).toMatchObject({ status: "spectrum", error: null });
    expect(fileStore.getFile(file.id)?.spectrum).toEqual(series);
    expect(fileStore.getStats()).toMatchObject({ spectra: 1, failed: 0 });
    expect(probeTableMock).not.toHaveBeenCalled();
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
    expect(probeTableMock).not.toHaveBeenCalled();
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

describe("alreadyInFilesMessage", () => {
  it("names the file a repeated open found in Files, counts several, and says nothing when every file was new", () => {
    expect(alreadyInFilesMessage([{ name: "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_whtlt.ecsv" }])).toBe(
      "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_whtlt.ecsv is already in Files: selected it instead of adding it again",
    );
    expect(alreadyInFilesMessage([{ name: "a.fits" }, { name: "b.fits" }, { name: "c.csv" }])).toBe(
      "3 files are already in Files: selected the first instead of adding them again",
    );
    expect(alreadyInFilesMessage([])).toBeNull();
  });
});
