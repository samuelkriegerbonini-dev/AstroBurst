import { describe, it, expect, vi } from "vitest";
import {
  EMPTY_SPECTRUM1D_DOC,
  MAX_SPECTRUM1D_DOCS,
  SPECTRUM_CSV_FILTER,
  Spectrum1dStore,
  SpectrumCsvStore,
  openSpectrumCsv,
  pickSpectrumCsvFile,
} from "../useSpectrum1dStore";

const dialog = vi.hoisted(() => ({ open: vi.fn<(options: unknown) => Promise<string | string[] | null>>() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: dialog.open }));
const reader = vi.hoisted(() => ({ document: vi.fn<(path: string, hdu: number | null) => Promise<unknown>>() }));
vi.mock("../../services/spectrum1d", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/spectrum1d")>()),
  readSpectrumDocument: reader.document,
}));
import type { Spectrum1d } from "../../shared/types/spectrum1d";

function spectrumAt(path: string, hdu: number): Spectrum1d {
  return {
    source: { kind: "wcs1d", path, hdu },
    axis: {
      kind: "awav",
      ctype: "LINEAR",
      unit: "um",
      header_unit: "Angstrom",
      header_scale: 1e-4,
      values: [0.65, 0.6501],
      crval: 0.65,
      cdelt: 0.0001,
      crpix: 1,
      rest_wavelength_um: null,
      rest_frequency_hz: null,
      specsys: null,
      velosys: null,
      notes: [],
    },
    wavelength_frame: "air",
    precision: "f64",
    flux: [1, Number.NaN],
    err: null,
    dq: null,
    dq_table: "unknown",
    flux_unit: null,
    n_rows: 1,
    cards: {},
    notes: [],
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("Spectrum1dStore", () => {
  it("starts empty, marks the document loading, then keeps the spectrum the reader returned", async () => {
    const pending = deferred<Spectrum1d>();
    const read = vi.fn(() => pending.promise);
    const store = new Spectrum1dStore(read);
    const listener = vi.fn();
    store.subscribe(listener);

    expect(store.getDoc("C:/d/ha.fits", 0)).toBe(EMPTY_SPECTRUM1D_DOC);
    const loading = store.load("C:/d/ha.fits", 0);
    expect(read).toHaveBeenCalledWith("C:/d/ha.fits", 0);
    expect(store.getDoc("C:/d/ha.fits", 0)).toEqual({ loading: true, spectrum: null, error: null });

    const spectrum = spectrumAt("C:/d/ha.fits", 0);
    pending.resolve(spectrum);
    await loading;
    expect(store.getDoc("C:/d/ha.fits", 0)).toEqual({ loading: false, spectrum, error: null });
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it("reads through the document reader by default, so an opened CSV reaches the CSV command", async () => {
    const csv = { ...spectrumAt("C:/d/exported_spectrum.csv", 0), source: { kind: "csv" as const, path: "C:/d/exported_spectrum.csv" } };
    reader.document.mockReset();
    reader.document.mockResolvedValueOnce(csv);
    const store = new Spectrum1dStore();
    await store.load("C:/d/exported_spectrum.csv", null);
    expect(reader.document).toHaveBeenCalledWith("C:/d/exported_spectrum.csv", null);
    expect(store.getDoc("C:/d/exported_spectrum.csv", null).spectrum).toBe(csv);
  });

  it("reads a document once while it loads and never again once it is ready", async () => {
    const read = vi.fn(async (path: string, hdu: number | null) => spectrumAt(path, hdu ?? 0));
    const store = new Spectrum1dStore(read);
    await Promise.all([store.load("C:/d/ha.fits", 0), store.load("C:/d/ha.fits", 0)]);
    await store.load("C:/d/ha.fits", 0);
    expect(read).toHaveBeenCalledTimes(1);
  });

  it("keys documents by path and HDU", async () => {
    const read = vi.fn(async (path: string, hdu: number | null) => spectrumAt(path, hdu ?? 0));
    const store = new Spectrum1dStore(read);
    await store.load("C:/d/x1d.fits", 1);
    await store.load("C:/d/x1d.fits", 2);
    expect(read).toHaveBeenCalledTimes(2);
    expect(store.getDoc("C:/d/x1d.fits", 1).spectrum?.source).toEqual({ kind: "wcs1d", path: "C:/d/x1d.fits", hdu: 1 });
    expect(store.getDoc("C:/d/x1d.fits", 2).spectrum?.source).toEqual({ kind: "wcs1d", path: "C:/d/x1d.fits", hdu: 2 });
    expect(store.getDoc("C:/d/x1d.fits", null)).toBe(EMPTY_SPECTRUM1D_DOC);
  });

  it("keeps the backend refusal verbatim and reads again on the next load", async () => {
    const refusal = "CTYPE1 = 'WAVE-LOG': log-lambda dispersion is not supported";
    const read = vi.fn<(path: string, hdu: number | null) => Promise<Spectrum1d>>();
    read.mockRejectedValueOnce(new Error(refusal));
    read.mockRejectedValueOnce("plain string rejection");
    const store = new Spectrum1dStore(read);

    await store.load("C:/d/log.fits", 0);
    expect(store.getDoc("C:/d/log.fits", 0)).toEqual({ loading: false, spectrum: null, error: refusal });

    await store.load("C:/d/log.fits", 0);
    expect(read).toHaveBeenCalledTimes(2);
    expect(store.getDoc("C:/d/log.fits", 0).error).toBe("plain string rejection");
  });

  it("evicts the least recently loaded document beyond the cap", async () => {
    const read = vi.fn(async (path: string, hdu: number | null) => spectrumAt(path, hdu ?? 0));
    const store = new Spectrum1dStore(read);
    for (let i = 0; i <= MAX_SPECTRUM1D_DOCS; i++) await store.load(`C:/d/s${i}.fits`, 0);
    expect(store.getDoc("C:/d/s0.fits", 0)).toBe(EMPTY_SPECTRUM1D_DOC);
    expect(store.getDoc("C:/d/s1.fits", 0).spectrum).not.toBeNull();
    expect(store.getDoc(`C:/d/s${MAX_SPECTRUM1D_DOCS}.fits`, 0).spectrum).not.toBeNull();
  });

  it("keeps a retained document past the cap and lets it go once released", async () => {
    const read = vi.fn(async (path: string, hdu: number | null) => spectrumAt(path, hdu ?? 0));
    const store = new Spectrum1dStore(read);
    const release = store.retain("C:/d/s0.fits", 0);
    for (let i = 0; i <= MAX_SPECTRUM1D_DOCS; i++) await store.load(`C:/d/s${i}.fits`, 0);
    expect(store.getDoc("C:/d/s0.fits", 0).spectrum).not.toBeNull();
    expect(store.getDoc("C:/d/s1.fits", 0)).toBe(EMPTY_SPECTRUM1D_DOC);
    release();
    await store.load("C:/d/extra.fits", 0);
    expect(store.getDoc("C:/d/s0.fits", 0)).toBe(EMPTY_SPECTRUM1D_DOC);
    expect(read).toHaveBeenCalledTimes(MAX_SPECTRUM1D_DOCS + 2);
  });

  it("holds a retained document while a second holder still has it", async () => {
    const read = vi.fn(async (path: string, hdu: number | null) => spectrumAt(path, hdu ?? 0));
    const store = new Spectrum1dStore(read);
    const first = store.retain("C:/d/s0.fits", 0);
    const second = store.retain("C:/d/s0.fits", 0);
    await store.load("C:/d/s0.fits", 0);
    first();
    first();
    for (let i = 1; i <= MAX_SPECTRUM1D_DOCS; i++) await store.load(`C:/d/s${i}.fits`, 0);
    expect(store.getDoc("C:/d/s0.fits", 0).spectrum).not.toBeNull();
    second();
  });

  it("reads a cached document from disk again when it is reread, at once while it is shown", async () => {
    let version = 0;
    const read = vi.fn(async (path: string, hdu: number | null) => ({ ...spectrumAt(path, hdu ?? 0), n_rows: ++version }));
    const store = new Spectrum1dStore(read);
    const path = "C:/d/x_spectrum.csv";
    await store.load(path, null);
    expect(store.getDoc(path, null).spectrum?.n_rows).toBe(1);
    await store.reread(path, null);
    expect(store.getDoc(path, null)).toBe(EMPTY_SPECTRUM1D_DOC);
    await store.load(path, null);
    expect(store.getDoc(path, null).spectrum?.n_rows).toBe(2);
    const release = store.retain(path, null);
    const shown = store.reread(path, null);
    expect(store.getDoc(path, null).loading).toBe(true);
    await shown;
    expect(store.getDoc(path, null).spectrum?.n_rows).toBe(3);
    release();
    expect(read).toHaveBeenCalledTimes(3);
  });

  it("leaves a document that is still loading to its own read when it is reread", async () => {
    const slow = deferred<Spectrum1d>();
    const read = vi.fn(() => slow.promise);
    const store = new Spectrum1dStore(read);
    const loading = store.load("C:/d/slow.csv", null);
    await store.reread("C:/d/slow.csv", null);
    slow.resolve(spectrumAt("C:/d/slow.csv", 0));
    await loading;
    expect(read).toHaveBeenCalledTimes(1);
    expect(store.getDoc("C:/d/slow.csv", null).spectrum).not.toBeNull();
  });

  it("never evicts a document while it loads, so it is read once", async () => {
    const slow = deferred<Spectrum1d>();
    const read = vi.fn(async (path: string, hdu: number | null) => (path === "C:/d/slow.fits" ? slow.promise : spectrumAt(path, hdu ?? 0)));
    const store = new Spectrum1dStore(read);
    const loading = store.load("C:/d/slow.fits", 0);
    for (let i = 0; i < MAX_SPECTRUM1D_DOCS; i++) await store.load(`C:/d/s${i}.fits`, 0);
    expect(store.getDoc("C:/d/slow.fits", 0).loading).toBe(true);
    void store.load("C:/d/slow.fits", 0);
    slow.resolve(spectrumAt("C:/d/slow.fits", 0));
    await loading;
    expect(read.mock.calls.filter(([path]) => path === "C:/d/slow.fits")).toHaveLength(1);
    expect(store.getDoc("C:/d/slow.fits", 0).spectrum).not.toBeNull();
  });
});

describe("SpectrumCsvStore", () => {
  it("opens a CSV over the selected file key, tells its listeners, and closes it", () => {
    const store = new SpectrumCsvStore();
    const listener = vi.fn();
    store.subscribe(listener);
    expect(store.get()).toBeNull();
    store.open("C:/d/ha_spectrum.csv", "cube|C:/d/cube.fits");
    expect(store.get()).toEqual({ path: "C:/d/ha_spectrum.csv", over: "cube|C:/d/cube.fits" });
    store.close();
    expect(store.get()).toBeNull();
    store.close();
    expect(listener).toHaveBeenCalledTimes(2);
  });
});

describe("openSpectrumCsv", () => {
  it("opens the picked CSV over the file selected at that moment", async () => {
    const store = new SpectrumCsvStore();
    await expect(openSpectrumCsv("img|C:/d/m31.fits", async () => "C:/d/ha_spectrum.csv", store)).resolves.toBe(true);
    expect(store.get()).toEqual({ path: "C:/d/ha_spectrum.csv", over: "img|C:/d/m31.fits" });
  });

  it("reads the picked CSV from disk again even when an earlier read of that path is cached", async () => {
    const read = vi.fn(async (path: string) => spectrumAt(path, 0));
    const documents = new Spectrum1dStore(read);
    await documents.load("C:/d/ha_spectrum.csv", null);
    const store = new SpectrumCsvStore();
    await expect(openSpectrumCsv(null, async () => "C:/d/ha_spectrum.csv", store, documents)).resolves.toBe(true);
    expect(documents.getDoc("C:/d/ha_spectrum.csv", null)).toBe(EMPTY_SPECTRUM1D_DOC);
    await documents.load("C:/d/ha_spectrum.csv", null);
    expect(read).toHaveBeenCalledTimes(2);
    expect(store.get()).toEqual({ path: "C:/d/ha_spectrum.csv", over: null });
  });

  it("keeps the CSV already open when the dialog is cancelled", async () => {
    const store = new SpectrumCsvStore();
    store.open("C:/d/a.csv", null);
    await expect(openSpectrumCsv("img|C:/d/m31.fits", async () => null, store)).resolves.toBe(false);
    expect(store.get()).toEqual({ path: "C:/d/a.csv", over: null });
  });

  it("reports a dialog failure on the console and opens nothing", async () => {
    const store = new SpectrumCsvStore();
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    await expect(
      openSpectrumCsv(null, async () => Promise.reject(new Error("dialog plugin unavailable")), store),
    ).resolves.toBe(false);
    expect(store.get()).toBeNull();
    expect(logged).toHaveBeenCalledWith("[AstroBurst] Spectrum CSV dialog error:", expect.any(Error));
    logged.mockRestore();
  });

  it("asks the dialog for one CSV file and ignores anything but a single path", async () => {
    dialog.open.mockResolvedValueOnce("C:/d/ha_spectrum.csv");
    await expect(pickSpectrumCsvFile()).resolves.toBe("C:/d/ha_spectrum.csv");
    expect(dialog.open).toHaveBeenCalledWith(expect.objectContaining({ multiple: false, directory: false, filters: [SPECTRUM_CSV_FILTER] }));
    expect(SPECTRUM_CSV_FILTER.extensions).toEqual(["csv"]);
    dialog.open.mockResolvedValueOnce(null);
    await expect(pickSpectrumCsvFile()).resolves.toBeNull();
    dialog.open.mockResolvedValueOnce(["C:/d/a.csv", "C:/d/b.csv"]);
    await expect(pickSpectrumCsvFile()).resolves.toBeNull();
  });
});
