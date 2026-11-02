import { describe, it, expect } from "vitest";
import { SpectrumStore } from "../useSpectrumStore";
import type { ContinuumWindows, RegionSpectrum } from "../../shared/types/cube";
import type { SpectrumSource } from "../../shared/types/spectral";

const extracted: RegionSpectrum = {
  sum: [1, 5, 1],
  mean: [0.1, 0.5, 0.1],
  npix: 28.27,
  n_bg: 0,
  bg_per_pixel: null,
  wavelengths: null,
  unit: "",
  bg_subtracted: false,
  flux_jy: null,
  err: null,
  err_scaled: null,
  mean_err: null,
  used_weight: [28.27, 28.27, 28.27],
  err_source: "none",
  bg_err_per_pixel: null,
  flux_jy_err: null,
  n_dq_excluded: null,
  noise_factor: null,
  notes: [],
  elapsed_ms: 1,
};
const smallCircle: SpectrumSource = { kind: "region", shape: { shape: "circle", x: 20, y: 20, r: 3 }, background: null };

describe("SpectrumStore region source", () => {
  it("keeps the shape the plotted region spectrum was extracted from", () => {
    const store = new SpectrumStore();
    store.commitRegion(extracted, smallCircle);
    expect(store.getSnapshot().region).toBe(extracted);
    expect(store.getSnapshot().regionSource).toEqual(smallCircle);
  });

  it("forgets the region source when a pixel is picked, a pixel spectrum lands or the region spectrum is cleared", () => {
    const store = new SpectrumStore();
    store.commitRegion(extracted, smallCircle);
    store.begin({ x: 1, y: 1 });
    expect(store.getSnapshot().regionSource).toBeNull();
    store.commitRegion(extracted, smallCircle);
    store.commit({ x: 1, y: 1, values: [1, 2, 3], wavelengths: [] }, 5);
    expect(store.getSnapshot().region).toBeNull();
    expect(store.getSnapshot().regionSource).toBeNull();
    store.commitRegion(extracted, smallCircle);
    store.clearRegion();
    expect(store.getSnapshot().regionSource).toBeNull();
    store.commitRegion(extracted, smallCircle);
    store.reset();
    expect(store.getSnapshot().regionSource).toBeNull();
  });

  it("keeps the continuum windows the region spectrum was asked with, and forgets them with the region", () => {
    const store = new SpectrumStore();
    const windows: ContinuumWindows = [
      [0, 4],
      [35, 39],
    ];
    expect(store.getSnapshot().regionContinuum).toBeNull();
    store.commitRegion(extracted, smallCircle, windows);
    expect(store.getSnapshot().regionContinuum).toEqual(windows);
    store.beginRegion();
    store.failRegion("boom");
    expect(store.getSnapshot().regionContinuum).toBeNull();
    store.commitRegion(extracted, smallCircle);
    expect(store.getSnapshot().regionContinuum).toBeNull();
    store.commitRegion(extracted, smallCircle, windows);
    store.begin({ x: 1, y: 1 });
    expect(store.getSnapshot().regionContinuum).toBeNull();
    store.commitRegion(extracted, smallCircle, windows);
    store.commit({ x: 1, y: 1, values: [1, 2, 3], wavelengths: [] }, 5);
    expect(store.getSnapshot().regionContinuum).toBeNull();
    store.commitRegion(extracted, smallCircle, windows);
    store.clearRegion();
    expect(store.getSnapshot().regionContinuum).toBeNull();
  });

  it("clears the source of the plotted spectrum while a new region extraction is running", () => {
    const store = new SpectrumStore();
    store.commitRegion(extracted, smallCircle);
    store.beginRegion();
    expect(store.getSnapshot().regionSource).toBeNull();
    store.failRegion("boom");
    expect(store.getSnapshot().region).toBeNull();
    expect(store.getSnapshot().regionSource).toBeNull();
  });
});

describe("SpectrumStore pixel errors", () => {
  it("keeps the ERR values of a committed pixel spectrum and drops them on reset", () => {
    const store = new SpectrumStore();
    store.commit({ x: 1, y: 2, values: [1, 2], wavelengths: [], err: [0.1, 0.2] }, 3);
    expect(store.getSnapshot().err).toEqual([0.1, 0.2]);
    store.commit({ x: 1, y: 2, values: [1, 2], wavelengths: [] }, 3);
    expect(store.getSnapshot().err).toBeNull();
    store.commit({ x: 1, y: 2, values: [1, 2], wavelengths: [], err: [0.1, 0.2] }, 3);
    store.reset();
    expect(store.getSnapshot().err).toBeNull();
  });
});

describe("SpectrumStore extraction invalidation", () => {
  const pixel = { x: 1, y: 2, values: [10, 20], wavelengths: [2.16, 2.17], err: [0.1, 0.2] };

  it("clears the old pixel values and errors as soon as another spaxel is selected", () => {
    const store = new SpectrumStore();
    store.commit(pixel, 5);
    store.begin({ x: 3, y: 4 });
    expect(store.getSnapshot()).toMatchObject({
      spectrum: [],
      err: null,
      wavelengths: null,
      coord: { x: 3, y: 4 },
      elapsed: 0,
      loading: true,
    });
    store.fail("unreadable spaxel");
    expect(store.getSnapshot().spectrum).toEqual([]);
  });

  it("clears both the previous region and pixel when another region extraction starts", () => {
    const store = new SpectrumStore();
    store.commit(pixel, 5);
    store.commitRegion(extracted, smallCircle, [[0, 4], [35, 39]]);
    store.beginRegion();
    expect(store.getSnapshot()).toMatchObject({
      spectrum: [],
      err: null,
      wavelengths: null,
      coord: null,
      region: null,
      regionSource: null,
      regionContinuum: null,
      regionLoading: true,
      elapsed: 0,
    });
    store.failRegion("unreadable region");
    expect(store.getSnapshot().region).toBeNull();
  });

  it("drops a late pixel response and error after a newer spaxel starts loading", () => {
    const store = new SpectrumStore();
    const oldRequest = store.begin({ x: 1, y: 2 });
    const currentRequest = store.begin({ x: 3, y: 4 });
    const loading = store.getSnapshot();
    store.commit(pixel, 8, oldRequest);
    store.fail("old failure", oldRequest);
    expect(store.getSnapshot()).toBe(loading);
    store.commit({ ...pixel, x: 3, y: 4, values: [30, 40] }, 2, currentRequest);
    expect(store.getSnapshot()).toMatchObject({ coord: { x: 3, y: 4 }, spectrum: [30, 40], loading: false, error: null });
  });

  it("drops a late region extraction after a pixel is selected", () => {
    const store = new SpectrumStore();
    const regionRequest = store.beginRegion();
    const pixelRequest = store.begin({ x: 1, y: 2 });
    store.commit(pixel, 2, pixelRequest);
    const current = store.getSnapshot();
    store.commitRegion(extracted, smallCircle, null, regionRequest);
    store.failRegion("old region failure", regionRequest);
    expect(store.getSnapshot()).toBe(current);
    expect(current.regionLoading).toBe(false);
  });

  it("drops a late pixel extraction after a region is selected", () => {
    const store = new SpectrumStore();
    const pixelRequest = store.begin({ x: 1, y: 2 });
    const regionRequest = store.beginRegion();
    store.commitRegion(extracted, smallCircle, null, regionRequest);
    const current = store.getSnapshot();
    store.commit(pixel, 5, pixelRequest);
    store.fail("old pixel failure", pixelRequest);
    expect(store.getSnapshot()).toBe(current);
    expect(current.loading).toBe(false);
  });

  it("drops responses from a cube that has been reset", () => {
    const store = new SpectrumStore();
    const pixelRequest = store.begin({ x: 1, y: 2 });
    store.reset();
    const empty = store.getSnapshot();
    store.commit(pixel, 5, pixelRequest);
    store.fail("old pixel failure", pixelRequest);
    expect(store.getSnapshot()).toBe(empty);
    const regionRequest = store.beginRegion();
    store.reset();
    store.commitRegion(extracted, smallCircle, null, regionRequest);
    store.failRegion("old region failure", regionRequest);
    expect(store.getSnapshot()).toBe(empty);
  });

  it("does not revive a region cleared while its extraction was pending", () => {
    const store = new SpectrumStore();
    const request = store.beginRegion();
    store.clearRegion();
    const current = store.getSnapshot();
    store.commitRegion(extracted, smallCircle, null, request);
    store.failRegion("old region failure", request);
    expect(store.getSnapshot()).toBe(current);
  });

  it("keeps a pending pixel request valid when clearing an absent region", () => {
    const store = new SpectrumStore();
    const request = store.begin({ x: 1, y: 2 });
    store.clearRegion();
    store.commit(pixel, 5, request);
    expect(store.getSnapshot().spectrum).toEqual(pixel.values);
  });
});
