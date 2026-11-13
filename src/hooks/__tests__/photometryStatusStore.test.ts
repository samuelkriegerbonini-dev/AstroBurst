import { describe, it, expect, vi } from "vitest";

const serviceRead = vi.hoisted(() => vi.fn<(...args: unknown[]) => Promise<unknown>>());
vi.mock("../../services/photometryStatus", () => ({ photometryStatus: serviceRead }));

import {
  EMPTY_PHOTOMETRY_STATUS,
  MAX_PHOTOMETRY_STATUS_DOCS,
  PhotometryStatusStore,
  photometryStatusKey,
  type PhotometryStatusRequest,
} from "../usePhotometryStatus";
import type { PhotometryStatus } from "../../shared/types/photometryStatus";

const STATUS: PhotometryStatus = {
  photcal: null,
  calibration_origin: null,
  missing_reason: "no photometric calibration in the header (BUNIT 'adu')",
  bunit: "adu",
  summed_unit: "adu",
  err: null,
  dq: null,
  pixel_scale_arcsec: null,
  pixel_scale_axes_arcsec: null,
  filter: null,
  warnings: [],
};

const request = (path: string, measureKey: string | null = null, excludeDq = false): PhotometryStatusRequest => ({ path, excludeDq, measureKey });

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("PhotometryStatusStore", () => {
  it("reads a status once per key and serves it from the store afterwards", async () => {
    const read = vi.fn(async (_request: PhotometryStatusRequest) => STATUS);
    const store = new PhotometryStatusStore(read);
    const req = request("C:/d/a.fits");
    await store.load(req);
    await store.load(req);
    expect(read).toHaveBeenCalledTimes(1);
    expect(store.getDoc(req)).toEqual({ loading: false, status: STATUS, error: null });
  });

  it("keys the status by path, DQ exclusion and measure key", async () => {
    expect(photometryStatusKey({ path: "C:/d/a.fits", excludeDq: true, measureKey: "C:/d/a.fits@4" })).toBe("1|C:/d/a.fits@4|C:/d/a.fits");
    expect(photometryStatusKey({ path: "C:/d/a.fits", excludeDq: false, measureKey: null })).toBe("0||C:/d/a.fits");
    const read = vi.fn(async (_request: PhotometryStatusRequest) => STATUS);
    const store = new PhotometryStatusStore(read);
    await store.load(request("C:/d/a.fits", "C:/d/a.fits@4"));
    await store.load(request("C:/d/a.fits", "C:/d/a.fits@5"));
    expect(read).toHaveBeenCalledTimes(2);
    expect(read).toHaveBeenLastCalledWith(request("C:/d/a.fits", "C:/d/a.fits@5"));
  });

  it("reports loading, then the message of a failed read, and never rejects", async () => {
    const read = vi.fn((_request: PhotometryStatusRequest) => Promise.reject<PhotometryStatus>(new Error("no such file")));
    const store = new PhotometryStatusStore(read);
    const req = request("C:/d/missing.fits");
    const loading = store.load(req);
    expect(store.getDoc(req)).toEqual({ loading: true, status: null, error: null });
    await expect(loading).resolves.toBeUndefined();
    expect(store.getDoc(req)).toEqual({ loading: false, status: null, error: "no such file" });
    await store.load(req);
    expect(read).toHaveBeenCalledTimes(2);
  });

  it("notifies subscribers on every change and stops after unsubscribe", async () => {
    const store = new PhotometryStatusStore(async () => STATUS);
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);
    await store.load(request("C:/d/a.fits"));
    expect(listener).toHaveBeenCalledTimes(2);
    unsubscribe();
    await store.load(request("C:/d/b.fits"));
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it("keeps at most eight documents and never drops a retained or loading one", async () => {
    expect(MAX_PHOTOMETRY_STATUS_DOCS).toBe(8);
    const store = new PhotometryStatusStore(async () => STATUS);
    const keys = Array.from({ length: MAX_PHOTOMETRY_STATUS_DOCS + 1 }, (_, i) => request(`C:/d/f${i}.fits`));
    store.retain(keys[0]);
    for (const key of keys) await store.load(key);
    expect(store.getDoc(keys[0]).status).toBe(STATUS);
    expect(store.getDoc(keys[1])).toBe(EMPTY_PHOTOMETRY_STATUS);
    expect(store.getDoc(keys[MAX_PHOTOMETRY_STATUS_DOCS]).status).toBe(STATUS);

    const pending = new PhotometryStatusStore((r) => (r.path === "C:/d/slow.fits" ? new Promise<PhotometryStatus>(() => {}) : Promise.resolve(STATUS)));
    const slow = request("C:/d/slow.fits");
    void pending.load(slow);
    for (const key of keys) await pending.load(key);
    expect(pending.getDoc(slow).loading).toBe(true);
  });

  it("forget drops the documents of a path and reloads the retained ones", async () => {
    const read = vi.fn(async (_request: PhotometryStatusRequest) => STATUS);
    const store = new PhotometryStatusStore(read);
    const kept = request("A", "A@1");
    const released = request("A", "A@2");
    const other = request("B");
    store.retain(kept);
    await store.load(kept);
    await store.load(released);
    await store.load(other);
    const otherDoc = store.getDoc(other);
    read.mockClear();
    store.forget("A");
    await flush();
    expect(read).toHaveBeenCalledTimes(1);
    expect(read).toHaveBeenCalledWith(kept);
    expect(store.getDoc(kept)).toEqual({ loading: false, status: STATUS, error: null });
    expect(store.getDoc(other)).toBe(otherDoc);
    expect(store.getDoc(released)).toBe(EMPTY_PHOTOMETRY_STATUS);
  });

  it("drops the answer of a read that a forget made stale", async () => {
    let answer!: (status: PhotometryStatus) => void;
    const store = new PhotometryStatusStore(() => new Promise<PhotometryStatus>((resolve) => (answer = resolve)));
    const req = request("A");
    void store.load(req);
    store.forget("A");
    answer({ ...STATUS, bunit: "stale" });
    await flush();
    expect(store.getDoc(req)).toBe(EMPTY_PHOTOMETRY_STATUS);
  });

  it("keys the status by the typed zero point too", async () => {
    const base = { path: "C:/d/a.fits", excludeDq: true, measureKey: "C:/d/a.fits@4" };
    const typed = { ...base, zeroPoint: { zp: 25, system: "ab" as const, per_second: true } };
    expect(photometryStatusKey(typed)).toBe("1|C:/d/a.fits@4|C:/d/a.fits|25,ab,true");
    expect(photometryStatusKey({ ...base, zeroPoint: null })).toBe("1|C:/d/a.fits@4|C:/d/a.fits");
    expect(photometryStatusKey(base)).toBe("1|C:/d/a.fits@4|C:/d/a.fits");
    expect(photometryStatusKey({ ...base, zeroPoint: { zp: 21.432, system: "vega", per_second: false } })).toBe(
      "1|C:/d/a.fits@4|C:/d/a.fits|21.432,vega,false",
    );

    const read = vi.fn(async (_request: PhotometryStatusRequest) => STATUS);
    const store = new PhotometryStatusStore(read);
    await store.load(typed);
    await store.load({ ...typed, zeroPoint: { zp: 25, system: "vega", per_second: true } });
    await store.load({ ...typed });
    expect(read).toHaveBeenCalledTimes(2);

    serviceRead.mockReset().mockResolvedValue(STATUS);
    const defaultStore = new PhotometryStatusStore();
    await defaultStore.load(typed);
    await defaultStore.load(base);
    expect(serviceRead.mock.calls).toEqual([
      ["C:/d/a.fits", { excludeDq: true, zeroPoint: { zp: 25, system: "ab", per_second: true } }],
      ["C:/d/a.fits", { excludeDq: true, zeroPoint: null }],
    ]);
  });
});
