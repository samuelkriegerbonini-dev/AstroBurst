import { beforeEach, describe, expect, it, vi } from "vitest";
import type { LineMeasurement } from "../../shared/types/spectral";
import type { Spectrum1d } from "../../shared/types/spectrum1d";
import type { SpectrumSetup } from "../../utils/spectrumSetup";

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

import { hooks } from "../../test/hookRuntime";
import {
  DEFAULT_SPECTRUM_VIEW,
  MAX_SPECTRUM_VIEWS,
  SpectrumViewStore,
  cubeViewKey,
  documentViewKey,
  effectiveFrameChoice,
  effectiveMode,
  spectrumViewStore,
  useSpectrumView,
  type SpectrumLineOverlay,
  type SpectrumViewState,
} from "../useSpectrumViewStore";

const DEFAULTS: SpectrumViewState = {
  mode: null,
  frameChoice: null,
  restUm: null,
  convention: "optical",
  redshift: 0,
  lineListVisible: true,
  lineFamilies: ["optical", "nir", "mir", "radio"],
  bandSpecies: [],
  pickedLineId: null,
  lineModel: "gaussian",
  useDq: true,
  range: null,
  windows: null,
  wavelengthUnit: "um",
  fluxUnit: "native",
  binFactor: 1,
  logY: false,
  yLimits: "full",
  xDomain: null,
};

function documentWithFrame(frame: Spectrum1d["wavelength_frame"]): Spectrum1d {
  return {
    source: { kind: "wcs1d", path: "C:/d/a.fits", hdu: 0 },
    axis: {
      kind: "awav",
      ctype: "AWAV",
      unit: "um",
      header_unit: "um",
      header_scale: 1,
      values: [0.6555, 0.6556],
      crval: 0.6555,
      cdelt: 1e-4,
      crpix: 1,
      rest_wavelength_um: null,
      rest_frequency_hz: null,
      specsys: null,
      velosys: null,
      notes: [],
    },
    wavelength_frame: frame,
    precision: "f64",
    flux: [1, 2],
    err: null,
    dq: null,
    dq_table: "unknown",
    flux_unit: null,
    n_rows: 2,
    cards: {},
    notes: [],
  };
}

type ViewHook = ReturnType<typeof useSpectrumView>;

const SETUP: SpectrumSetup = {
  fromKey: "1|C:/d/a.fits",
  fromLabel: "a.fits, 1D array in HDU 1",
  instrument: "",
  mode: "velocity",
  restUm: 0.656461,
  convention: "optical",
  redshift: 0.05,
  lineListVisible: true,
  lineFamilies: ["optical"],
  bandSpecies: [],
  pickedLineId: null,
  lineModel: "gaussian",
  wavelengthUnit: "um",
  rangeUm: [0.656, 0.6567],
  windowsUm: null,
};

function overlayOf(center: number): SpectrumLineOverlay {
  return { result: { center } as unknown as LineMeasurement, measurementAxis: [0.6555, 0.6556] };
}

function counted(store: SpectrumViewStore): { calls: () => number } {
  let calls = 0;
  store.subscribe(() => {
    calls++;
  });
  return { calls: () => calls };
}

describe("SpectrumViewStore", () => {
  it("starts every document from the defaults", () => {
    const store = new SpectrumViewStore();
    expect(store.get("1|C:/a.fits")).toEqual(DEFAULTS);
    expect(DEFAULT_SPECTRUM_VIEW).toEqual(DEFAULTS);
    expect(MAX_SPECTRUM_VIEWS).toBe(16);
    expect(store.keys()).toEqual([]);
  });

  it("keeps what was patched, per key", () => {
    const store = new SpectrumViewStore();
    const listener = counted(store);
    store.patch("A", { range: { z0: 5, z1: 12 }, restUm: 0.656461 });
    expect(listener.calls()).toBe(1);
    store.patch("B", { redshift: 0.5 });
    expect(listener.calls()).toBe(2);
    expect(store.get("A")).toEqual({ ...DEFAULTS, range: { z0: 5, z1: 12 }, restUm: 0.656461 });
    expect(store.get("A").redshift).toBe(0);
    expect(store.get("B")).toEqual({ ...DEFAULTS, redshift: 0.5 });
    expect(store.get("B").range).toBeNull();
  });

  it("does not create an entry for a patch that changes nothing", () => {
    const store = new SpectrumViewStore();
    const listener = counted(store);
    store.patch("k", { restUm: null });
    store.patch("k", { lineFamilies: ["optical", "nir", "mir", "radio"] });
    store.patch("k", { range: null });
    expect(store.keys()).toEqual([]);
    expect(listener.calls()).toBe(0);
  });

  it("does not notify when a patch repeats the stored values", () => {
    const store = new SpectrumViewStore();
    store.patch("k", { redshift: 0.5 });
    const stored = store.get("k");
    const listener = counted(store);
    store.patch("k", { redshift: 0.5 });
    store.patch("k", { range: null });
    expect(store.get("k")).toBe(stored);
    expect(listener.calls()).toBe(0);
  });

  it("drops the zoom when the displayed axis changes", () => {
    const store = new SpectrumViewStore();
    const domain: [number, number] = [1, 2];
    const axisPatches: Partial<SpectrumViewState>[] = [
      { mode: "frequency" },
      { frameChoice: "air" },
      { wavelengthUnit: "nm" },
      { convention: "radio" },
    ];
    for (const axisPatch of axisPatches) {
      store.patch("k", { xDomain: domain });
      expect(store.get("k").xDomain).toEqual([1, 2]);
      store.patch("k", axisPatch);
      expect(store.get("k").xDomain, JSON.stringify(axisPatch)).toBeNull();
    }
    store.reset();
    store.patch("k", { xDomain: domain });
    store.patch("k", { restUm: 0.65 });
    expect(store.get("k").xDomain).toEqual([1, 2]);
    store.patch("k", { mode: "velocity" });
    store.patch("k", { xDomain: domain });
    store.patch("k", { restUm: 0.6563 });
    expect(store.get("k").xDomain).toBeNull();
    store.patch("k", { xDomain: domain });
    store.patch("k", { range: { z0: 1, z1: 3 } });
    store.patch("k", { logY: true });
    store.patch("k", { redshift: 1 });
    expect(store.get("k").xDomain).toEqual([1, 2]);
  });

  it("keeps the sixteen most recently used documents", () => {
    const store = new SpectrumViewStore();
    for (let i = 0; i <= 16; i++) store.patch(`k${i}`, { redshift: i + 1 });
    expect(store.keys()).toHaveLength(16);
    expect(store.keys()).not.toContain("k0");
    expect(store.keys()).toContain("k1");

    const touched = new SpectrumViewStore();
    for (let i = 0; i <= 15; i++) touched.patch(`k${i}`, { redshift: i + 1 });
    touched.get("k0");
    touched.patch("k16", { redshift: 17 });
    expect(touched.keys()).toHaveLength(16);
    expect(touched.keys()).toContain("k0");
    expect(touched.keys()).not.toContain("k1");
  });

  it("holds the line overlay beside the state and forgets it with the document", () => {
    const store = new SpectrumViewStore();
    const o = overlayOf(0.6563);
    const listener = counted(store);
    store.setOverlay("k", o);
    expect(store.overlay("k")).toBe(o);
    expect(listener.calls()).toBe(1);
    store.setOverlay("k", null);
    expect(store.overlay("k")).toBeNull();
    expect(listener.calls()).toBe(2);

    store.patch("k0", { redshift: 1 });
    store.setOverlay("k0", overlayOf(1));
    for (let i = 1; i <= 16; i++) store.patch(`k${i}`, { redshift: i + 1 });
    expect(store.keys()).not.toContain("k0");
    expect(store.overlay("k0")).toBeNull();

    store.setOverlay("k5", o);
    store.reset();
    expect(store.overlay("k5")).toBeNull();
    expect(store.keys()).toEqual([]);
    expect(store.get("k5")).toEqual(DEFAULTS);
  });

  it("follows the file until the user chooses a frame or an axis", () => {
    const air = documentWithFrame("air");
    expect(effectiveFrameChoice(DEFAULT_SPECTRUM_VIEW, air)).toBe("air");
    expect(effectiveFrameChoice({ ...DEFAULTS, frameChoice: "vacuum" }, air)).toBe("vacuum");
    expect(effectiveFrameChoice({ ...DEFAULTS, frameChoice: "air" }, documentWithFrame("not_wavelength"))).toBeNull();
    expect(effectiveFrameChoice({ ...DEFAULTS, frameChoice: "air" }, null)).toBe("air");
    expect(effectiveFrameChoice(DEFAULT_SPECTRUM_VIEW, null)).toBeNull();
  });

  it("opens a spectrum on the axis of its frame until the user picks one", () => {
    const air = documentWithFrame("air");
    const vacuum = documentWithFrame("vacuum");
    expect(effectiveMode(DEFAULT_SPECTRUM_VIEW, air)).toBe("wavelength_air");
    expect(effectiveMode({ ...DEFAULTS, frameChoice: "vacuum" }, air)).toBe("wavelength_vac");
    expect(effectiveMode(DEFAULT_SPECTRUM_VIEW, vacuum)).toBe("wavelength_vac");
    expect(effectiveMode({ ...DEFAULTS, frameChoice: "air" }, vacuum)).toBe("wavelength_air");
    expect(effectiveMode(DEFAULT_SPECTRUM_VIEW, documentWithFrame("not_wavelength"))).toBe("wavelength_vac");
    expect(effectiveMode({ ...DEFAULTS, frameChoice: "air" }, documentWithFrame("not_wavelength"))).toBe("wavelength_vac");
    expect(effectiveMode(DEFAULT_SPECTRUM_VIEW, null)).toBe("wavelength_vac");
    expect(effectiveMode({ ...DEFAULTS, frameChoice: "air" }, null)).toBe("wavelength_vac");
    expect(effectiveMode({ ...DEFAULTS, mode: "velocity" }, air)).toBe("velocity");
    expect(effectiveMode({ ...DEFAULTS, mode: "wavelength_vac" }, air)).toBe("wavelength_vac");
  });

  it("keeps the setup apart from the view states", () => {
    const store = new SpectrumViewStore();
    store.rememberSetup(SETUP);
    expect(store.keys()).toEqual([]);
    expect(store.get(SETUP.fromKey)).toEqual(DEFAULTS);
    const fresh = new SpectrumViewStore();
    fresh.patch("k", { redshift: 0.5 });
    expect(fresh.lastSetup()).toBeNull();
  });

  it("keeps the last setup, the seed labels and the dismissed keys without notifying anyone", () => {
    const store = new SpectrumViewStore();
    const listener = counted(store);
    expect(store.lastSetup()).toBeNull();
    store.rememberSetup(SETUP);
    expect(store.lastSetup()).toBe(SETUP);
    store.markSeeded("k", "a.fits");
    expect(store.seededFrom("k")).toBe("a.fits");
    expect(store.setupDismissed("k")).toBe(false);
    store.dismissSetup("k");
    expect(store.seededFrom("k")).toBeNull();
    expect(store.setupDismissed("k")).toBe(true);
    expect(store.setupDismissed("j")).toBe(false);
    store.markSeeded("j", "b.fits");
    expect(listener.calls()).toBe(0);
    store.reset();
    expect(store.lastSetup()).toBeNull();
    expect(store.seededFrom("j")).toBeNull();
    expect(store.setupDismissed("k")).toBe(false);

    const evicting = new SpectrumViewStore();
    evicting.patch("k0", { redshift: 1 });
    evicting.markSeeded("k0", "a.fits");
    for (let i = 1; i <= 15; i++) evicting.patch(`k${i}`, { redshift: i + 1 });
    expect(evicting.seededFrom("k0")).toBe("a.fits");
    evicting.patch("k16", { redshift: 17 });
    expect(evicting.keys()).not.toContain("k0");
    expect(evicting.seededFrom("k0")).toBeNull();
  });

  it("forgets the last setup when a carried setup is dismissed, so it is carried no further", () => {
    const store = new SpectrumViewStore();
    store.rememberSetup(SETUP);
    store.markSeeded("k", SETUP.fromLabel);
    store.dismissSetup("k");
    expect(store.lastSetup()).toBeNull();
    store.rememberSetup(SETUP);
    expect(store.lastSetup()).toBe(SETUP);
  });

  it("builds its keys from the path and the selection", () => {
    expect(documentViewKey("C:/d/a.fits", "1")).toBe("1|C:/d/a.fits");
    expect(documentViewKey("C:/d/s.csv", "")).toBe("|C:/d/s.csv");
    expect(cubeViewKey("C:/d/c.fits")).toBe("cube|C:/d/c.fits");
  });
});

describe("useSpectrumView", () => {
  beforeEach(() => {
    hooks.mount();
    spectrumViewStore.reset();
  });

  it("reads the state of its key and patches it", () => {
    const Probe = () => useSpectrumView("k");
    const [first, patch] = hooks.render(Probe, undefined) as ViewHook;
    expect(first).toEqual(DEFAULTS);
    patch({ redshift: 0.5 });
    const [second, samePatch] = hooks.render(Probe, undefined) as ViewHook;
    expect(second.redshift).toBe(0.5);
    expect(samePatch).toBe(patch);
    expect(spectrumViewStore.keys()).toEqual(["k"]);
  });

  it("gives the defaults and a patch that does nothing without a key", () => {
    const [state, patch] = hooks.render(() => useSpectrumView(null), undefined) as ViewHook;
    expect(state).toEqual(DEFAULTS);
    patch({ redshift: 2 });
    expect(spectrumViewStore.keys()).toEqual([]);
  });
});
