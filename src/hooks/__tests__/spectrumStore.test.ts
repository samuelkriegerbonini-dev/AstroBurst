import { describe, it, expect } from "vitest";
import { SpectrumStore } from "../useSpectrumStore";
import type { RegionSpectrum } from "../../shared/types/cube";
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

  it("keeps the source of the plotted spectrum while a new region extraction is running", () => {
    const store = new SpectrumStore();
    store.commitRegion(extracted, smallCircle);
    store.beginRegion();
    expect(store.getSnapshot().regionSource).toEqual(smallCircle);
    store.failRegion("boom");
    expect(store.getSnapshot().region).toBe(extracted);
    expect(store.getSnapshot().regionSource).toEqual(smallCircle);
  });
});
