import { describe, it, expect, expectTypeOf } from "vitest";
import {
  EMPTY_COMPARISON_DOC,
  MAX_COMPARISON_DOCS,
  SpectrumComparisonStore,
  type ComparisonPatch,
} from "../useSpectrumComparisonStore";
import type { ComparisonEntry } from "../../utils/spectrumCompare";
import type { X1dSpectrum } from "../../shared/types/spectral";

function x1dAt(path: string, hdu = 1): X1dSpectrum {
  return {
    path,
    hdu,
    extver: 1,
    n_rows: 2,
    wavelength_um: [1, 2],
    wavelength_unit: "um",
    flux: [1, 2],
    flux_error: null,
    flux_unit: "Jy",
    surf_bright: null,
    surf_bright_unit: null,
    background: null,
    npixels: null,
    dq: null,
    dq_table: "jwst",
    dq_flagged_rows: 0,
    srctype: null,
    grating: null,
    filter: null,
    detector: null,
    instrument: null,
    target: null,
    other_tables: [],
    notes: [],
  };
}

const entry: ComparisonEntry = {
  id: "r1",
  kind: "region",
  label: "R1",
  color: "#7dd3fc",
  source: { kind: "region", shape: { shape: "circle", x: 20, y: 20, r: 3 }, background: null },
  regionText: null,
  backgroundId: null,
  region: {
    sum: [1, 5, 1],
    mean: [0.1, 0.5, 0.1],
    npix: 28.27,
    n_bg: 0,
    bg_per_pixel: null,
    wavelengths: null,
    unit: "um",
    bg_subtracted: false,
    flux_jy: null,
    elapsed_ms: 1,
  },
  pixel: null,
  table: null,
  error: null,
};

const path = "C:/d/cube.fits";

describe("SpectrumComparisonStore", () => {
  it("begin increments the seq and commit is ignored for a stale seq", () => {
    const store = new SpectrumComparisonStore();
    let notified = 0;
    store.subscribe(() => notified++);
    const first = store.begin(path, 1, "3,4");
    expect(first).toBe(1);
    expect(store.getDoc(path).loading).toBe(true);
    expect(store.getDoc(path).pendingRegionVersion).toBe(1);
    expect(store.getDoc(path).pendingPixelKey).toBe("3,4");
    const second = store.begin(path, 1, "3,4");
    expect(second).toBe(2);
    expect(store.commit(path, first, [entry])).toBe(false);
    expect(store.getDoc(path).entries).toEqual([]);
    expect(store.getDoc(path).loading).toBe(true);
    expect(store.commit(path, second, [entry])).toBe(true);
    const doc = store.getDoc(path);
    expect(doc.entries).toEqual([entry]);
    expect(doc.loading).toBe(false);
    expect(doc.regionVersion).toBe(1);
    expect(doc.pixelKey).toBe("3,4");
    expect(doc.pendingRegionVersion).toBeNull();
    expect(doc.pendingPixelKey).toBeNull();
    expect(doc.seq).toBe(2);
    expect(notified).toBe(3);
  });

  it("docs are kept per file path and patch keeps the other fields", () => {
    const store = new SpectrumComparisonStore();
    expect(store.getDoc("unknown")).toBe(EMPTY_COMPARISON_DOC);
    expect(EMPTY_COMPARISON_DOC).toEqual({
      enabled: false,
      entries: [],
      loading: false,
      regionVersion: null,
      pixelKey: null,
      pendingRegionVersion: null,
      pendingPixelKey: null,
      normalise: "none",
      offsetStep: null,
      includePixel: true,
      hidden: [],
      seq: 0,
      tablePath: null,
      tableHdu: null,
      table: null,
      tableError: null,
      tableLoading: false,
    });
    const before = store.getDoc(path);
    store.patch(path, { enabled: true, normalise: "peak" });
    store.patch("C:/d/other.fits", { includePixel: false, hidden: ["r1"] });
    const a = store.getDoc(path);
    expect(a).not.toBe(before);
    expect(a.enabled).toBe(true);
    expect(a.normalise).toBe("peak");
    expect(a.includePixel).toBe(true);
    expect(a.hidden).toEqual([]);
    const b = store.getDoc("C:/d/other.fits");
    expect(b.enabled).toBe(false);
    expect(b.normalise).toBe("none");
    expect(b.includePixel).toBe(false);
    expect(b.hidden).toEqual(["r1"]);
    expect(store.getDoc(path)).toBe(a);
    store.forget(path);
    expect(store.getDoc(path)).toBe(EMPTY_COMPARISON_DOC);
    expect(store.getDoc("C:/d/other.fits")).toBe(b);
  });

  it("invalidate drops an in-flight batch and clears loading", () => {
    const store = new SpectrumComparisonStore();
    store.patch(path, { enabled: true });
    const seq = store.begin(path, 4, null);
    store.invalidate(path);
    const doc = store.getDoc(path);
    expect(doc.loading).toBe(false);
    expect(doc.seq).toBe(seq + 1);
    expect(doc.pendingRegionVersion).toBeNull();
    expect(doc.pendingPixelKey).toBeNull();
    expect(doc.enabled).toBe(true);
    expect(store.commit(path, seq, [entry])).toBe(false);
    expect(store.getDoc(path).entries).toEqual([]);
    store.invalidate("never-seen");
    expect(store.getDoc("never-seen")).toBe(EMPTY_COMPARISON_DOC);
  });

  it("an in-flight batch begun at region version 1 is dropped by an invalidation for version 2", () => {
    const store = new SpectrumComparisonStore();
    const stale = store.begin(path, 1, null);
    store.invalidate(path);
    expect(store.commit(path, stale, [entry])).toBe(false);
    expect(store.getDoc(path).entries).toEqual([]);
    expect(store.getDoc(path).regionVersion).toBeNull();
    const fresh = store.begin(path, 2, null);
    expect(store.getDoc(path).pendingRegionVersion).toBe(2);
    expect(store.commit(path, fresh, [entry])).toBe(true);
    const doc = store.getDoc(path);
    expect(doc.regionVersion).toBe(2);
    expect(doc.pixelKey).toBeNull();
    expect(doc.entries).toEqual([entry]);
    expect(doc.pendingRegionVersion).toBeNull();
    expect(doc.pendingPixelKey).toBeNull();
  });

  it("the store keeps at most MAX_COMPARISON_DOCS files", () => {
    const store = new SpectrumComparisonStore();
    expect(MAX_COMPARISON_DOCS).toBe(8);
    const paths = Array.from({ length: MAX_COMPARISON_DOCS + 1 }, (_, i) => `C:/d/cube${i}.fits`);
    for (const p of paths) store.patch(p, { enabled: true });
    expect(store.getDoc(paths[0])).toBe(EMPTY_COMPARISON_DOC);
    for (const p of paths.slice(1)) expect(store.getDoc(p).enabled).toBe(true);
    store.patch(paths[1], { normalise: "median" });
    store.patch("C:/d/late.fits", { enabled: true });
    expect(store.getDoc(paths[2])).toBe(EMPTY_COMPARISON_DOC);
    expect(store.getDoc(paths[1]).normalise).toBe("median");
  });

  it("requestTable enables the comparison and records the request, and begin/commit/fail/clear move the table state", () => {
    const store = new SpectrumComparisonStore();
    let notified = 0;
    store.subscribe(() => notified++);
    store.patch(path, { normalise: "peak" });
    store.requestTable(path, "C:/d/cube_x1d.fits");
    let doc = store.getDoc(path);
    expect(doc.enabled).toBe(true);
    expect(doc.normalise).toBe("peak");
    expect(doc.tablePath).toBe("C:/d/cube_x1d.fits");
    expect(doc.tableHdu).toBeNull();
    expect(doc.table).toBeNull();
    expect(doc.tableError).toBeNull();
    expect(doc.tableLoading).toBe(false);

    const request = store.beginTable(path);
    expect(request).toEqual({ path: "C:/d/cube_x1d.fits", hdu: null });
    expect(store.getDoc(path).tableLoading).toBe(true);
    const x1d = x1dAt("C:/d/cube_x1d.fits");
    expect(store.commitTable(path, request!, x1d)).toBe(true);
    doc = store.getDoc(path);
    expect(doc.table).toBe(x1d);
    expect(doc.tableLoading).toBe(false);
    expect(doc.tableError).toBeNull();

    store.requestTable(path, x1d.path, 3);
    doc = store.getDoc(path);
    expect(doc.table).toBeNull();
    expect(doc.tableHdu).toBe(3);
    const second = store.beginTable(path);
    expect(second).toEqual({ path: x1d.path, hdu: 3 });
    expect(store.failTable(path, second!, "HDU 3 (ASDF) is not an EXTRACT1D table")).toBe(true);
    doc = store.getDoc(path);
    expect(doc.tableError).toBe("HDU 3 (ASDF) is not an EXTRACT1D table");
    expect(doc.tableLoading).toBe(false);
    expect(doc.table).toBeNull();

    store.requestTable(path, x1d.path);
    expect(store.getDoc(path).tableError).toBeNull();
    store.clearTable(path);
    doc = store.getDoc(path);
    expect(doc.tablePath).toBeNull();
    expect(doc.tableHdu).toBeNull();
    expect(doc.table).toBeNull();
    expect(doc.tableError).toBeNull();
    expect(doc.tableLoading).toBe(false);
    expect(doc.enabled).toBe(true);
    expect(store.beginTable(path)).toBeNull();
    expect(notified).toBe(9);
  });

  it("a table reply for a superseded or cleared request is dropped", () => {
    const store = new SpectrumComparisonStore();
    store.requestTable(path, "C:/d/a_x1d.fits");
    const stale = store.beginTable(path)!;
    store.requestTable(path, "C:/d/b_x1d.fits");
    expect(store.getDoc(path).tableLoading).toBe(false);
    const fresh = store.beginTable(path)!;
    expect(store.commitTable(path, stale, x1dAt("C:/d/a_x1d.fits"))).toBe(false);
    expect(store.failTable(path, stale, "late")).toBe(false);
    expect(store.getDoc(path).table).toBeNull();
    expect(store.getDoc(path).tableError).toBeNull();
    expect(store.getDoc(path).tableLoading).toBe(true);
    const b = x1dAt("C:/d/b_x1d.fits");
    expect(store.commitTable(path, fresh, b)).toBe(true);
    expect(store.getDoc(path).table).toBe(b);
    expect(store.commitTable(path, fresh, x1dAt("C:/d/b_x1d.fits"))).toBe(false);
    store.clearTable(path);
    expect(store.commitTable(path, fresh, b)).toBe(false);
    expect(store.getDoc(path).table).toBeNull();
    expect(store.commitTable("C:/d/never.fits", fresh, b)).toBe(false);
    expect(store.getDoc("C:/d/never.fits")).toBe(EMPTY_COMPARISON_DOC);
  });

  it("a table load that never settles can be cancelled with clearTable or restarted with requestTable", () => {
    const store = new SpectrumComparisonStore();
    store.requestTable(path, "C:/d/a_x1d.fits");
    const stuck = store.beginTable(path)!;
    expect(store.getDoc(path).tableLoading).toBe(true);
    store.clearTable(path);
    expect(store.getDoc(path).tableLoading).toBe(false);
    expect(store.failTable(path, stuck, "late")).toBe(false);
    expect(store.getDoc(path).tableError).toBeNull();
    store.requestTable(path, "C:/d/a_x1d.fits");
    const first = store.beginTable(path)!;
    store.requestTable(path, "C:/d/a_x1d.fits");
    expect(store.getDoc(path).tableLoading).toBe(false);
    const retry = store.beginTable(path)!;
    expect(retry).toEqual(first);
    expect(store.getDoc(path).tableLoading).toBe(true);
  });

  it("patch cannot set the table state owned by the loader", () => {
    expectTypeOf<ComparisonPatch>().not.toHaveProperty("table");
    expectTypeOf<ComparisonPatch>().not.toHaveProperty("tableError");
    expectTypeOf<ComparisonPatch>().not.toHaveProperty("tableLoading");
    expectTypeOf<ComparisonPatch>().toHaveProperty("tablePath");
    expectTypeOf<ComparisonPatch>().toHaveProperty("tableHdu");
  });
});
