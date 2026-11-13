import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SpectrumRequest, SpectrumSeriesSet, SpectrumTableOption } from "../../services/spectrum1d";
import type { Spectrum1d } from "../../shared/types/spectrum1d";

const state = vi.hoisted(() => ({
  isCube: false,
  isSpectrum: false,
  ramp: null as unknown,
  file: null as Record<string, unknown> | null,
  csv: null as { path: string; over: string | null } | null,
  doc: { loading: false, spectrum: null as unknown, error: null as string | null },
  requests: [] as (SpectrumRequest | null)[],
}));
const lists = vi.hoisted(() => ({
  tables: vi.fn<(path: string) => Promise<unknown>>(),
  series: vi.fn<(paths: string[]) => Promise<unknown>>(),
}));

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../context/PreviewContext", () => ({
  fileKeyOf: (file: { id: string; path: string } | null) => (file ? `${file.id}|${file.path}` : null),
  useFileContext: () => ({ file: state.file }),
  useCubeContext: () => ({ isCube: state.isCube, isSpectrum: state.isSpectrum, ramp: state.ramp }),
}));
vi.mock("../../services/spectrum1d", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/spectrum1d")>()),
  listSpectrumTables: lists.tables,
  listSpectrumSeries: lists.series,
}));
vi.mock("../useSpectrum1dStore", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../useSpectrum1dStore")>()),
  useSpectrum1dRequest: (request: SpectrumRequest | null) => {
    state.requests.push(request);
    return request === null ? { loading: false, spectrum: null, error: null } : state.doc;
  },
  useOpenSpectrumCsv: () => state.csv,
}));

import { hooks } from "../../test/hookRuntime";
import { spectrumSelectionStore, useActiveSpectrumDocument, type ActiveSpectrumDocument } from "../useActiveSpectrumDocument";

const X1D_PATH = "C:/data/jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits";
const CSV_PATH = "C:/data/ha_spectrum.csv";
const A5_PATH = "C:/data/jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_x1dints.fits";
const SPECTRUM = { source: { kind: "x1d", path: X1D_PATH, hdu: 1, extver: 1 }, flux: [1, 2, 3] } as unknown as Spectrum1d;
const X1D_FILE = { id: "x1", path: X1D_PATH, status: "spectrum", spectrum: { kind: "x1d", hdu: 1 } };
const CUBE_FILE = { id: "f1", path: "C:/data/cube.fits" };
const IMAGE_FILE = { id: "img", path: "C:/data/m31.fits" };
const A5_KIND = { kind: "x1dints", layout: "rows", n_series: 2, n_integrations: 8, n_channels: 1271 };
const A5_SERIES: SpectrumSeriesSet = {
  set_id: "a5a5a5a5a5a5a5a5",
  series: [
    { key: { detector: "NRS1", sporder: -1 }, label: "NRS1", n_integrations: 8, n_channels: 1271, flux_unit: "Jy" },
    { key: { detector: "NRS2", sporder: -1 }, label: "NRS2", n_integrations: 8, n_channels: 2048, flux_unit: "Jy" },
  ],
  notes: [],
  warnings: [],
};
const NRS1_MEDIAN = { kind: "series", detector: "NRS1", sporder: -1, pick: { kind: "median" } } as const;
const NO_OPTIONS = { tables: null, series: null, loading: false, error: null };

const active = () => hooks.render(useActiveSpectrumDocument, undefined) as ActiveSpectrumDocument;
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const fileRequest = (path: string, hdu: number | null): SpectrumRequest => ({ kind: "file", path, hdu, wavelengthUnit: null });
const documentRequests = () => state.requests.filter((request): request is SpectrumRequest => request !== null);

function selectSpectrum(id: string, path: string, spectrum: Record<string, unknown>): string {
  state.isSpectrum = true;
  state.file = { id, path, status: "spectrum", spectrum };
  return `${id}|${path}`;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => {
    resolve = yes;
  });
  return { promise, resolve };
}

beforeEach(() => {
  hooks.mount();
  spectrumSelectionStore.reset();
  lists.tables.mockReset().mockResolvedValue([]);
  lists.series.mockReset().mockResolvedValue(A5_SERIES);
  state.isCube = false;
  state.isSpectrum = false;
  state.ramp = null;
  state.file = null;
  state.csv = null;
  state.doc = { loading: false, spectrum: null, error: null };
  state.requests = [];
});

describe("useActiveSpectrumDocument", () => {
  it("follows the selected spectrum file", () => {
    state.isSpectrum = true;
    state.file = X1D_FILE;
    state.doc = { loading: false, spectrum: SPECTRUM, error: null };
    const first = active();
    expect(first).toMatchObject({ content: "spectrum", fileKey: `x1|${X1D_PATH}`, path: X1D_PATH, hdu: 1, viewKey: "1|" + X1D_PATH });
    expect(state.requests).toContainEqual(fileRequest(X1D_PATH, 1));
    expect(first.doc).toBe(state.doc);
    expect(first.document).toEqual({ source: { kind: "file", hdu: 1 }, spectrum: SPECTRUM });
    const second = active();
    expect(second.document).toBe(first.document);
  });

  it("has no document for a cube or an image", () => {
    state.isCube = true;
    state.file = CUBE_FILE;
    expect(active()).toMatchObject({ content: "cube", path: null, hdu: null, viewKey: null, document: null });
    expect(state.requests).toContainEqual(null);

    state.isCube = false;
    state.file = IMAGE_FILE;
    expect(active()).toMatchObject({ content: "empty", path: null, viewKey: null, document: null });

    state.file = null;
    expect(active()).toMatchObject({ content: "empty", fileKey: null, path: null, viewKey: null, document: null });
    expect(state.requests.every((request) => request === null)).toBe(true);
  });

  it("shows a CSV opened over the selected file, and over nothing", () => {
    state.isSpectrum = true;
    state.file = X1D_FILE;
    state.csv = { path: CSV_PATH, over: "x1|" + X1D_PATH };
    expect(active()).toMatchObject({ content: "csv", path: CSV_PATH, hdu: null, viewKey: "|" + CSV_PATH });
    expect(state.requests).toContainEqual(fileRequest(CSV_PATH, null));

    state.isSpectrum = false;
    state.file = IMAGE_FILE;
    expect(active()).toMatchObject({ content: "csv", path: CSV_PATH, viewKey: "|" + CSV_PATH });

    state.isCube = true;
    state.file = CUBE_FILE;
    expect(active()).toMatchObject({ content: "cube", path: null, viewKey: null });
  });

  it("reaches the other tables of an x1d", async () => {
    const path = "C:/data/jw01345-o063_t001_nirspec_g395m-f290lp_x1d.fits";
    const fileKey = selectSpectrum("m1", path, { kind: "x1d", hdu: 2, n_tables: 3 });
    const tables = [2, 3, 4].map(
      (hdu): SpectrumTableOption => ({
        hdu,
        extver: hdu - 1,
        n_rows: 3400,
        n_channels: 3400,
        sporder: null,
        detector: null,
        slit_name: `S${hdu}`,
        source_id: null,
        source_name: null,
        int_num: null,
        srctype: null,
        label: `slit S${hdu}`,
      }),
    );
    lists.tables.mockResolvedValueOnce(tables);
    state.doc = { loading: false, spectrum: SPECTRUM, error: null };
    const first = active();
    expect(documentRequests()).toEqual([fileRequest(path, 2)]);

    spectrumSelectionStore.set(fileKey, { kind: "table", hdu: 4, wavelengthUnit: null });
    const second = active();
    expect(documentRequests().at(-1)).toEqual(fileRequest(path, 4));
    expect(second.viewKey).toBe("4|" + path);
    expect(second.document?.source).toEqual({ kind: "file", hdu: 4 });
    expect(second).toMatchObject({ hdu: 4, request: fileRequest(path, 4), selection: { kind: "table", hdu: 4, wavelengthUnit: null } });
    expect(first).toMatchObject({ hdu: 2, request: fileRequest(path, 2), viewKey: "2|" + path, selection: { kind: "table", hdu: 2, wavelengthUnit: null } });
    expect(first.kind).toEqual({ kind: "x1d", hdu: 2, n_tables: 3 });

    await settle();
    expect(active().options).toEqual({ tables, series: null, loading: false, error: null });
    expect(lists.tables).toHaveBeenCalledTimes(1);
    expect(lists.tables).toHaveBeenCalledWith(path);
    expect(lists.series).not.toHaveBeenCalled();
  });

  it("opens a time-series file on the median of its first series", async () => {
    const fileKey = selectSpectrum("a5", A5_PATH, A5_KIND);
    const listed = deferred<SpectrumSeriesSet>();
    lists.series.mockReturnValueOnce(listed.promise);
    const before = active();
    expect(documentRequests()).toEqual([]);
    expect(before.request).toBeNull();
    expect(before.doc).toEqual({ loading: true, spectrum: null, error: null });
    expect(before).toMatchObject({ selection: null, hdu: null, viewKey: "|" + A5_PATH, document: null, kind: A5_KIND });
    expect(before.options).toMatchObject({ loading: true, series: null, error: null });
    expect(lists.series).toHaveBeenCalledWith([A5_PATH]);

    listed.resolve(A5_SERIES);
    await settle();
    state.doc = { loading: false, spectrum: SPECTRUM, error: null };
    const after = active();
    const median = { kind: "series", paths: [A5_PATH], detector: "NRS1", sporder: -1, pick: { kind: "median" } };
    expect(spectrumSelectionStore.get(fileKey)).toEqual(NRS1_MEDIAN);
    expect(after.request).toEqual(median);
    expect(documentRequests().at(-1)).toEqual(median);
    expect(after.viewKey).toBe("NRS1:-1:median|" + A5_PATH);
    expect(after.document?.source).toEqual({ kind: "file", hdu: null });
    expect(after).toMatchObject({ hdu: null, selection: NRS1_MEDIAN, options: { series: A5_SERIES, loading: false, error: null } });
    expect(active().selection).toBe(spectrumSelectionStore.get(fileKey));
    expect(lists.series).toHaveBeenCalledTimes(1);
    expect(lists.tables).not.toHaveBeenCalled();

    const emptyPath = "C:/data/jw09999-o001_t001_nirspec_clear-prism-s1600a1-sub512_x1dints.fits";
    const emptyKey = selectSpectrum("e1", emptyPath, A5_KIND);
    lists.series.mockResolvedValueOnce({ set_id: "0000000000000000", series: [], notes: [], warnings: [] });
    state.requests = [];
    active();
    await settle();
    const empty = active();
    expect(empty.doc).toEqual({ loading: false, spectrum: null, error: "this file holds no series of spectra" });
    expect(empty).toMatchObject({ request: null, selection: null, viewKey: "|" + emptyPath });
    expect(spectrumSelectionStore.get(emptyKey)).toBeNull();
    expect(documentRequests()).toEqual([]);
  });

  it("says why a time-series file has no document when its series cannot be listed", async () => {
    const path = "C:/data/jw01366-o001_t001_nirspec_clear-prism-s1600a1-sub512_s3d.fits";
    const fileKey = selectSpectrum("c3", path, A5_KIND);
    lists.series.mockRejectedValueOnce(new Error("DATAMODL 'CubeModel' is not a time series of spectra"));
    active();
    await settle();
    const failed = active();
    expect(failed.doc).toEqual({ loading: false, spectrum: null, error: "DATAMODL 'CubeModel' is not a time series of spectra" });
    expect(failed.options).toEqual({ ...NO_OPTIONS, error: "DATAMODL 'CubeModel' is not a time series of spectra" });
    expect(failed).toMatchObject({ request: null, selection: null, document: null });
    expect(spectrumSelectionStore.get(fileKey)).toBeNull();
    expect(documentRequests()).toEqual([]);
  });

  it("lists tables only when there may be more than one", async () => {
    const one = "C:/data/jw02072-o001_t001_miri_p750l_x1d.fits";
    selectSpectrum("t1", one, { kind: "x1d", hdu: 1, n_tables: 1 });
    expect(active().options).toEqual(NO_OPTIONS);
    expect(lists.tables).not.toHaveBeenCalled();

    const unknown = "C:/data/jw02072-o002_t001_miri_p750l_x1d.fits";
    selectSpectrum("t2", unknown, { kind: "x1d", hdu: 1 });
    active();
    active();
    expect(lists.tables).toHaveBeenCalledTimes(1);
    expect(lists.tables).toHaveBeenCalledWith(unknown);

    selectSpectrum("w1", "C:/data/ha_bess.fits", { kind: "wcs1d", hdu: 0 });
    expect(active().options).toEqual(NO_OPTIONS);
    expect(lists.tables).toHaveBeenCalledTimes(1);

    const broken = "C:/data/jw02072-o003_t001_miri_p750l_x1d.fits";
    selectSpectrum("t3", broken, { kind: "x1d", hdu: 1, n_tables: 2 });
    lists.tables.mockRejectedValueOnce(new Error("Failed to open C:/data/jw02072-o003_t001_miri_p750l_x1d.fits"));
    state.doc = { loading: false, spectrum: SPECTRUM, error: null };
    active();
    await settle();
    const failed = active();
    expect(lists.tables).toHaveBeenCalledTimes(2);
    expect(failed.options).toEqual({ ...NO_OPTIONS, error: "Failed to open C:/data/jw02072-o003_t001_miri_p750l_x1d.fits" });
    expect(failed.request).toEqual(fileRequest(broken, 1));
    expect(failed.doc).toBe(state.doc);
    expect(failed.document?.source).toEqual({ kind: "file", hdu: 1 });

    state.isSpectrum = false;
    state.file = IMAGE_FILE;
    state.csv = { path: CSV_PATH, over: null };
    const csv = active();
    expect(csv).toMatchObject({ content: "csv", kind: null, selection: null, viewKey: "|" + CSV_PATH, request: fileRequest(CSV_PATH, null) });
    expect(csv.options).toEqual(NO_OPTIONS);
    expect(lists.tables).toHaveBeenCalledTimes(2);
    expect(lists.series).not.toHaveBeenCalled();
  });

  it("re-reads a 1D array with the stated unit", () => {
    const path = "C:/data/bare_linear_array.fits";
    const fileKey = selectSpectrum("w2", path, { kind: "wcs1d", hdu: 0 });
    spectrumSelectionStore.set(fileKey, { kind: "table", hdu: 0, wavelengthUnit: "nm" });
    const shown = active();
    expect(shown.request).toEqual({ kind: "file", path, hdu: 0, wavelengthUnit: "nm" });
    expect(documentRequests()).toEqual([{ kind: "file", path, hdu: 0, wavelengthUnit: "nm" }]);
    expect(shown).toMatchObject({ hdu: 0, viewKey: "0@nm|" + path });
    expect(lists.tables).not.toHaveBeenCalled();
  });

  it("replaces a stored series that the file does not have", async () => {
    const path = "C:/data/jw03375001001_03101_00001-seg001_nrs1_x1dints.fits";
    const fileKey = selectSpectrum("a5a", path, A5_KIND);
    spectrumSelectionStore.set(fileKey, { kind: "series", detector: "NRS9", sporder: -1, pick: { kind: "median" } });
    expect(active().request).toBeNull();
    await settle();
    const replaced = active();
    expect(spectrumSelectionStore.get(fileKey)).toEqual(NRS1_MEDIAN);
    expect(replaced.viewKey).toBe("NRS1:-1:median|" + path);
    expect(documentRequests().some((request) => request.kind === "series" && request.detector === "NRS9")).toBe(false);

    const third = { kind: "series", detector: "NRS2", sporder: -1, pick: { kind: "integration", index: 2 } } as const;
    spectrumSelectionStore.set(fileKey, third);
    const kept = active();
    expect(kept.request).toEqual({ kind: "series", paths: [path], detector: "NRS2", sporder: -1, pick: { kind: "integration", index: 2 } });
    expect(kept.viewKey).toBe("NRS2:-1:int2|" + path);
    expect(spectrumSelectionStore.get(fileKey)).toBe(third);
  });

  it("keeps the lists of the last sixteen files", async () => {
    const paths = Array.from({ length: 17 }, (_, i) => `C:/data/tso/jw02327001001_04101_00001-seg${String(i + 1).padStart(3, "0")}_nrs1_x1dints.fits`);
    const select = async (path: string) => {
      selectSpectrum(path, path, A5_KIND);
      active();
      await settle();
    };
    for (const path of paths) await select(path);
    await select(paths[0]);
    await select(paths[16]);
    const callsFor = (path: string) => lists.series.mock.calls.filter(([listed]) => listed[0] === path).length;
    expect(callsFor(paths[0])).toBe(2);
    expect(callsFor(paths[16])).toBe(1);
    expect(lists.series).toHaveBeenCalledTimes(18);
  });
});
