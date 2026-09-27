import { describe, it, expect, beforeEach, vi } from "vitest";

const { restretchMock } = vi.hoisted(() => ({ restretchMock: vi.fn() }));

vi.mock("../../services/compose", () => ({
  restretchComposite: restretchMock,
  clearCompositeCache: vi.fn(async () => {}),
}));

vi.mock("../../infrastructure/tauri", () => ({
  getOutputDir: vi.fn(async () => "C:/out"),
  getPreviewUrl: vi.fn(async (path: string) => `asset://${path}`),
}));

import {
  compositeReducer,
  INITIAL_COMPOSITE_STATE,
  isCompositeGoneError,
  isRestretchCompositeUrl,
  parkedCompositeUrl,
  restoreParkedComposite,
  type CompositeAction,
  type CompositeState,
  type ParkedComposite,
} from "../CompositeContext";

const R = { shadow: 0.02, midtone: 0.017, highlight: 1 };
const G = { shadow: 0.01, midtone: 0.05, highlight: 1 };
const B = { shadow: 0.03, midtone: 0.02, highlight: 1 };

function parkedAt(previewUrl: string, linked = false): ParkedComposite {
  return { previewUrl, stf: { r: R, g: G, b: B }, autoStf: { r: R, g: G, b: B }, linked };
}

const TAURI_RESTRETCH_URL = "http://asset.localhost/C%3A%5Cout%2Frgb_composite_1727361234567.png?v=4";

describe("isRestretchCompositeUrl", () => {
  it("matches the restretch PNG that the backend sweeps, in plain and in encoded asset URLs", () => {
    expect(isRestretchCompositeUrl("C:/out/rgb_composite_1727361234567.png")).toBe(true);
    expect(isRestretchCompositeUrl(TAURI_RESTRETCH_URL)).toBe(true);
    expect(isRestretchCompositeUrl("C:\\out\\rgb_composite_5.png")).toBe(true);
  });

  it("does not match the other composite PNGs, which the backend keeps", () => {
    for (const name of ["composite_masked_1.png", "composite_lhe_1.png", "composite_chain_masked_1.png", "composite_lrgb_1.png", "rgb_composite_1.fits"]) {
      expect(isRestretchCompositeUrl(`http://asset.localhost/C%3A%5Cout%2F${name}`)).toBe(false);
    }
  });
});

describe("parkedCompositeUrl", () => {
  beforeEach(() => {
    restretchMock.mockReset();
    restretchMock.mockResolvedValue({ png_path: "C:/out/rgb_composite_2000.png", elapsed_ms: 1 });
  });

  it("regenerates an rgb_composite PNG with the parked STF and link flag, uncached", async () => {
    const url = await parkedCompositeUrl(parkedAt(TAURI_RESTRETCH_URL, true));
    expect(restretchMock).toHaveBeenCalledTimes(1);
    expect(restretchMock).toHaveBeenCalledWith("C:/out", R, G, B, undefined, false, true);
    expect(url).toBe("asset://C:/out/rgb_composite_2000.png");
  });

  it("reuses any other composite PNG as is", async () => {
    const other = "http://asset.localhost/C%3A%5Cout%2Fcomposite_masked_1727.png";
    expect(await parkedCompositeUrl(parkedAt(other))).toBe(other);
    expect(await parkedCompositeUrl(parkedAt("asset://composite_lrgb_9.png"))).toBe("asset://composite_lrgb_9.png");
    expect(restretchMock).not.toHaveBeenCalled();
  });
});

describe("restoreParkedComposite", () => {
  beforeEach(() => {
    restretchMock.mockReset();
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  it("shows the regenerated URL for the parked slot", async () => {
    restretchMock.mockResolvedValue({ png_path: "C:/out/rgb_composite_3000.png", elapsed_ms: 1 });
    const parked = parkedAt(TAURI_RESTRETCH_URL);
    const actions: CompositeAction[] = [];
    await restoreParkedComposite(parked, () => true, (a) => actions.push(a));
    expect(actions).toEqual([{ type: "SHOW_PARKED", parked, url: "asset://C:/out/rgb_composite_3000.png" }]);
  });

  it("drops the slot with only a console error when the backend composite is gone", async () => {
    restretchMock.mockRejectedValue(new Error("The colour composite is no longer in memory; run Blend again."));
    const parked = parkedAt(TAURI_RESTRETCH_URL);
    const actions: CompositeAction[] = [];
    await restoreParkedComposite(parked, () => true, (a) => actions.push(a));
    expect(actions).toEqual([{ type: "DROP_PARKED", parked }]);
    expect(console.error).toHaveBeenCalled();
  });

  it("keeps the slot on any other failure, and does nothing if the slot changed while regenerating", async () => {
    restretchMock.mockRejectedValue(new Error("disk full"));
    const actions: CompositeAction[] = [];
    await restoreParkedComposite(parkedAt(TAURI_RESTRETCH_URL), () => true, (a) => actions.push(a));
    expect(actions).toEqual([]);
    restretchMock.mockResolvedValue({ png_path: "C:/out/rgb_composite_4000.png", elapsed_ms: 1 });
    await restoreParkedComposite(parkedAt(TAURI_RESTRETCH_URL), () => false, (a) => actions.push(a));
    expect(actions).toEqual([]);
  });

  it("park, then a composite step completes, then Show composite: shows the step output as is instead of re-rendering the pre-step restretch", async () => {
    let state: CompositeState = compositeReducer(INITIAL_COMPOSITE_STATE, { type: "SET_PREVIEW_URL", url: TAURI_RESTRETCH_URL });
    state = compositeReducer(state, { type: "SET_AUTO_STF", r: R, g: G, b: B });
    state = compositeReducer(state, { type: "PARK" });
    const stepUrl = "http://asset.localhost/C%3A%5Cout%2Fcomposite_chain_deconv_1727361299999.png";
    state = compositeReducer(state, { type: "REPLACE_PARKED", previewUrl: stepUrl, stf: { r: B, g: B, b: B, linked: true } });
    const dispatch = (a: CompositeAction) => {
      state = compositeReducer(state, a);
    };
    const slot = state.parked!;
    await restoreParkedComposite(slot, () => state.parked === slot, dispatch);
    expect(restretchMock).not.toHaveBeenCalled();
    expect(state.previewUrl).toBe(stepUrl);
    expect(state.stf).toEqual({ r: B, g: B, b: B });
    expect(state.linked).toBe(true);
    expect(state.parked).toBeNull();
  });

  it("recognises the backend's gone message in thrown strings too", () => {
    expect(isCompositeGoneError("The colour composite is no longer in memory; run Blend again.")).toBe(true);
    expect(isCompositeGoneError(new Error("disk full"))).toBe(false);
  });
});
