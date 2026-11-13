import { describe, expect, it, vi } from "vitest";
import {
  EMPTY_IMAGE_PSF,
  ImagePsfStore,
  MAX_IMAGE_PSF_DOCS,
  RESOLUTION_STORAGE_KEY,
  ResolutionSettingsStore,
} from "../useResolutionSettings";
import { DEFAULT_RESOLUTION_SETTINGS } from "../../utils/resolutionText";
import type { ImagePsfFwhm, ResolutionSettings } from "../../shared/types/resolution";

function memoryStorage(initial: Record<string, string> = {}) {
  const items = new Map(Object.entries(initial));
  return {
    items,
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => {
      items.set(key, value);
    },
  };
}

const throwingGet = {
  getItem: (): string | null => {
    throw new Error("storage is locked");
  },
  setItem: () => undefined,
};

const throwingSet = {
  getItem: (): string | null => null,
  setItem: () => {
    throw new Error("quota exceeded");
  },
};

function psf(fwhmPx: number): ImagePsfFwhm {
  return {
    fwhm_px: fwhmPx,
    fwhm_px_scatter: 0.1,
    fwhm_arcsec: fwhmPx * 0.262,
    fwhm_arcsec_scatter: 0.03,
    half_light_radius_px: null,
    half_light_radius_arcsec: null,
    n_stars: 102,
    n_measured: 194,
    n_candidates: 200,
    n_detected: 8092,
    max_stars: 200,
    model: "Moffat fit",
    pixel_scale_arcsec: 0.262,
    pixels_per_fwhm: fwhmPx,
    undersampled: false,
    header_values: [],
    stars: [],
    notes: [],
    warnings: [],
    elapsed_ms: 10,
  };
}

interface Pending {
  path: string;
  resolve: (value: ImagePsfFwhm) => void;
  reject: (reason: unknown) => void;
}

function manualMeasure() {
  const calls: Pending[] = [];
  const measure = vi.fn(
    (path: string) =>
      new Promise<ImagePsfFwhm>((resolve, reject) => {
        calls.push({ path, resolve, reject });
      }),
  );
  return { calls, measure };
}

describe("ResolutionSettingsStore", () => {
  it("starts from the defaults and from a valid stored value", () => {
    expect(new ResolutionSettingsStore(memoryStorage()).getSnapshot()).toEqual(DEFAULT_RESOLUTION_SETTINGS);
    expect(new ResolutionSettingsStore(null).getSnapshot()).toEqual(DEFAULT_RESOLUTION_SETTINGS);
    const stored: ResolutionSettings = {
      redshift: 1.2,
      h0: 70,
      omegaM: 0.3,
      psfSource: "typed",
      psfFwhmArcsec: 0.9,
      eps: 0.05,
      qResolved: 2.5,
      minPixelsPerFwhm: 2,
      scaleOfInterestKpc: 1,
    };
    const store = new ResolutionSettingsStore(memoryStorage({ [RESOLUTION_STORAGE_KEY]: JSON.stringify(stored) }));
    expect(store.getSnapshot()).toEqual(stored);
    expect(RESOLUTION_STORAGE_KEY).toBe("ab.resolution.v1");
  });

  it("a broken or out-of-range stored value falls back member by member", () => {
    expect(new ResolutionSettingsStore(memoryStorage({ [RESOLUTION_STORAGE_KEY]: "not json" })).getSnapshot()).toEqual(
      DEFAULT_RESOLUTION_SETTINGS,
    );
    const partial = new ResolutionSettingsStore(memoryStorage({ [RESOLUTION_STORAGE_KEY]: '{"h0":5,"redshift":1}' })).getSnapshot();
    expect(partial).toEqual({ ...DEFAULT_RESOLUTION_SETTINGS, h0: 67.66, redshift: 1 });
    expect(new ResolutionSettingsStore(throwingGet).getSnapshot()).toEqual(DEFAULT_RESOLUTION_SETTINGS);
  });

  it("update merges, sanitizes, stores and notifies once", () => {
    const storage = memoryStorage();
    const store = new ResolutionSettingsStore(storage);
    const listener = vi.fn();
    store.subscribe(listener);
    const before = store.getSnapshot();
    expect(store.getSnapshot()).toBe(before);

    store.update({ redshift: 2 });
    const after = store.getSnapshot();
    expect(after.redshift).toBe(2);
    expect(after).not.toBe(before);
    expect(storage.items.get(RESOLUTION_STORAGE_KEY)).toBe(JSON.stringify(after));
    expect(listener).toHaveBeenCalledTimes(1);

    store.update({ redshift: 2 });
    expect(listener).toHaveBeenCalledTimes(1);
    expect(store.getSnapshot()).toBe(after);

    store.update({ h0: 500 });
    expect(store.getSnapshot().h0).toBe(after.h0);
    expect(listener).toHaveBeenCalledTimes(1);

    const failing = new ResolutionSettingsStore(throwingSet);
    expect(() => failing.update({ redshift: 3 })).not.toThrow();
    expect(failing.getSnapshot().redshift).toBe(3);
  });

  it("reset returns to the defaults", () => {
    const storage = memoryStorage();
    const store = new ResolutionSettingsStore(storage);
    const listener = vi.fn();
    store.subscribe(listener);
    store.update({ redshift: 1.5, psfSource: "typed", psfFwhmArcsec: 0.8 });
    store.reset();
    expect(store.getSnapshot()).toEqual(DEFAULT_RESOLUTION_SETTINGS);
    expect(JSON.parse(storage.items.get(RESOLUTION_STORAGE_KEY) ?? "null")).toEqual(DEFAULT_RESOLUTION_SETTINGS);
    expect(listener).toHaveBeenCalledTimes(2);
  });
});

describe("ImagePsfStore", () => {
  it("the image PSF store keeps one measurement per path", async () => {
    const { calls, measure } = manualMeasure();
    const store = new ImagePsfStore(measure);
    const listener = vi.fn();
    store.subscribe(listener);
    expect(store.getSnapshot(null)).toBe(EMPTY_IMAGE_PSF);
    expect(EMPTY_IMAGE_PSF).toEqual({ psf: null, loading: false, error: null });
    expect(Object.isFrozen(EMPTY_IMAGE_PSF)).toBe(true);

    const running = store.measure("a");
    expect(measure).toHaveBeenCalledWith("a");
    expect(store.getSnapshot("a")).toEqual({ psf: null, loading: true, error: null });
    calls[0].resolve(psf(3.5));
    await running;
    expect(store.getSnapshot("a")).toEqual({ psf: psf(3.5), loading: false, error: null });
    expect(store.getSnapshot("b")).toBe(EMPTY_IMAGE_PSF);

    const again = store.measure("a");
    expect(store.getSnapshot("a")).toEqual({ psf: psf(3.5), loading: true, error: null });
    calls[1].reject(new Error("No star was detected in this image"));
    await again;
    expect(store.getSnapshot("a")).toEqual({ psf: null, loading: false, error: "No star was detected in this image" });

    const plain = store.measure("c");
    calls[2].reject("refused");
    await plain;
    expect(store.getSnapshot("c").error).toBe("refused");
    expect(listener).toHaveBeenCalledTimes(6);
  });

  it("a stale measurement is dropped and the cache holds eight paths", async () => {
    const { calls, measure } = manualMeasure();
    const store = new ImagePsfStore(measure);

    const first = store.measure("a");
    const second = store.measure("a");
    calls[1].resolve(psf(4));
    await second;
    calls[0].resolve(psf(3));
    await first;
    expect(store.getSnapshot("a").psf).toEqual(psf(4));

    const late = store.measure("a");
    store.forget("a");
    expect(store.getSnapshot("a")).toBe(EMPTY_IMAGE_PSF);
    calls[2].resolve(psf(5));
    await late;
    expect(store.getSnapshot("a")).toBe(EMPTY_IMAGE_PSF);

    expect(MAX_IMAGE_PSF_DOCS).toBe(8);
    const paths = ["p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8", "p9"];
    for (const path of paths) {
      const running = store.measure(path);
      calls[calls.length - 1].resolve(psf(3));
      await running;
    }
    expect(store.getSnapshot("p1")).toBe(EMPTY_IMAGE_PSF);
    for (const path of paths.slice(1)) expect(store.getSnapshot(path).psf).toEqual(psf(3));
  });
});
