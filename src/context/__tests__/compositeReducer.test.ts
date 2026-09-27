import { describe, it, expect } from "vitest";
import {
  bumpsCompositeVersion,
  canShowParkedComposite,
  compositeReducer,
  INITIAL_COMPOSITE_STATE,
  liveCompositeVersion,
  noteCompositeAction,
  type CompositeAction,
  type CompositeState,
  type ParkedComposite,
} from "../CompositeContext";

const R = { shadow: 0.02, midtone: 0.017, highlight: 1 };
const G = { shadow: 0.01, midtone: 0.05, highlight: 1 };
const B = { shadow: 0.03, midtone: 0.02, highlight: 1 };
const PARKED: ParkedComposite = { previewUrl: "blend", stf: { r: R, g: G, b: B }, autoStf: { r: R, g: G, b: B }, linked: false };
const STEP_STF = { r: B, g: B, b: B, linked: true };
const DECONV_URL = "asset://composite_chain_deconv_1.png";

function wizardComposite(url = "asset://rgb_composite_1.png"): CompositeState {
  let s = compositeReducer(INITIAL_COMPOSITE_STATE, { type: "SET_PREVIEW_URL", url });
  s = compositeReducer(s, { type: "SET_AUTO_STF", r: R, g: G, b: B });
  s = compositeReducer(s, { type: "SET_STF", r: G, g: B, b: R });
  return s;
}

describe("compositeReducer", () => {
  it("INIT_RGB from per-channel ingest STF selects per-channel normalisation", () => {
    const next = compositeReducer(INITIAL_COMPOSITE_STATE, { type: "INIT_RGB", previewUrl: "u", stfR: R, stfG: G, stfB: B });
    expect(INITIAL_COMPOSITE_STATE.linked).toBe(true);
    expect(next.linked).toBe(false);
    expect(next.stf).toEqual({ r: R, g: G, b: B });
  });

  it("a linked auto STF triple (wizard Blend) switches back to linked normalisation", () => {
    const rgb = compositeReducer(INITIAL_COMPOSITE_STATE, { type: "INIT_RGB", previewUrl: "u", stfR: R, stfG: G, stfB: B });
    const blended = compositeReducer(rgb, { type: "SET_AUTO_STF", r: G, g: G, b: G });
    expect(blended.linked).toBe(true);
    const perChannel = compositeReducer(INITIAL_COMPOSITE_STATE, { type: "SET_AUTO_STF", r: R, g: G, b: B });
    expect(perChannel.linked).toBe(false);
  });

  it("bumps the version on every preview URL, init and reset, even for the same URL", () => {
    let s = INITIAL_COMPOSITE_STATE;
    s = compositeReducer(s, { type: "SET_PREVIEW_URL", url: "a" });
    const v1 = s.version;
    s = compositeReducer(s, { type: "SET_PREVIEW_URL", url: "a" });
    expect(s.version).toBeGreaterThan(v1);
    const v2 = s.version;
    s = compositeReducer(s, { type: "INIT_RGB", previewUrl: "a", stfR: R, stfG: G, stfB: B });
    expect(s.version).toBeGreaterThan(v2);
    const v3 = s.version;
    s = compositeReducer(s, { type: "RESET" });
    expect(s.version).toBeGreaterThan(v3);
    expect(s.previewUrl).toBeNull();
    expect(s.linked).toBe(true);
  });

  it("hiding the composite with SET_PREVIEW_URL null keeps the Blend STF that Export and Stretch read", () => {
    let blended = compositeReducer(INITIAL_COMPOSITE_STATE, { type: "SET_PREVIEW_URL", url: "blend" });
    blended = compositeReducer(blended, { type: "SET_AUTO_STF", r: R, g: G, b: B });
    blended = compositeReducer(blended, { type: "SET_STF", r: R, g: G, b: B });
    const hidden = compositeReducer(blended, { type: "SET_PREVIEW_URL", url: null });
    expect(hidden.previewUrl).toBeNull();
    expect(hidden.version).toBeGreaterThan(blended.version);
    expect(hidden.stf).toEqual({ r: R, g: G, b: B });
    expect(hidden.autoStf).toEqual({ r: R, g: G, b: B });
    expect(hidden.linked).toBe(false);
  });

  it("RESET discards the composite STF, so it must not serve as a display-only hide", () => {
    let blended = compositeReducer(INITIAL_COMPOSITE_STATE, { type: "SET_AUTO_STF", r: R, g: G, b: B });
    blended = compositeReducer(blended, { type: "SET_STF", r: R, g: G, b: B });
    const reset = compositeReducer(blended, { type: "RESET" });
    expect(reset.stf).toEqual(INITIAL_COMPOSITE_STATE.stf);
    expect(reset.autoStf).toEqual({ r: null, g: null, b: null });
  });

  it("live STF edits do not bump the version", () => {
    const s = compositeReducer(INITIAL_COMPOSITE_STATE, { type: "SET_STF", r: R, g: R, b: R });
    expect(s.version).toBe(INITIAL_COMPOSITE_STATE.version);
  });

  it("the live version store moves exactly when the reducer bumps the version", () => {
    const actions: CompositeAction[] = [
      { type: "SET_PREVIEW_URL", url: "a" },
      { type: "SET_STF", r: R, g: G, b: B },
      { type: "SET_AUTO_STF", r: R, g: G, b: B },
      { type: "SET_LINKED", linked: false },
      { type: "INIT_RGB", previewUrl: "a", stfR: R, stfG: G, stfB: B },
      { type: "PARK" },
      { type: "SHOW_PARKED", parked: PARKED, url: "b" },
      { type: "DROP_PARKED", parked: PARKED },
      { type: "REPLACE_PARKED", previewUrl: "c", stf: STEP_STF },
      { type: "RESET" },
      { type: "CLEAR" },
    ];
    for (const action of actions) {
      const reducerBumped = compositeReducer(INITIAL_COMPOSITE_STATE, action).version !== INITIAL_COMPOSITE_STATE.version;
      const before = liveCompositeVersion();
      noteCompositeAction(action);
      expect(bumpsCompositeVersion(action)).toBe(reducerBumped);
      expect(liveCompositeVersion() !== before).toBe(reducerBumped);
    }
  });
});

describe("compositeReducer parked composite", () => {
  it("PARK hides the composite and keeps the live STF exactly as it was", () => {
    const live = wizardComposite();
    const parked = compositeReducer(live, { type: "PARK" });
    expect(parked.previewUrl).toBeNull();
    expect(parked.version).toBeGreaterThan(live.version);
    expect(parked.stf).toBe(live.stf);
    expect(parked.autoStf).toBe(live.autoStf);
    expect(parked.linked).toBe(live.linked);
    expect(parked.parked).toEqual({
      previewUrl: "asset://rgb_composite_1.png",
      stf: { r: G, g: B, b: R },
      autoStf: { r: R, g: G, b: B },
      linked: false,
    });
  });

  it("SHOW_PARKED restores the parked URL, STF, auto STF and link flag exactly and empties the slot", () => {
    const live = wizardComposite();
    let s = compositeReducer(live, { type: "PARK" });
    s = compositeReducer(s, { type: "INIT_RGB", previewUrl: "asset://rgb.png", stfR: B, stfG: B, stfB: B });
    const slot = s.parked!;
    const shown = compositeReducer(s, { type: "SHOW_PARKED", parked: slot, url: "asset://rgb_composite_2.png" });
    expect(shown.previewUrl).toBe("asset://rgb_composite_2.png");
    expect(shown.stf).toEqual(live.stf);
    expect(shown.autoStf).toEqual(live.autoStf);
    expect(shown.linked).toBe(live.linked);
    expect(shown.parked).toBeNull();
    expect(shown.rgbFileView).toBe(false);
    expect(shown.version).toBeGreaterThan(s.version);
  });

  it("a partial auto STF parks as null and comes back as all-null", () => {
    let s = compositeReducer(INITIAL_COMPOSITE_STATE, { type: "SET_PREVIEW_URL", url: "lrgb" });
    s = compositeReducer(s, { type: "SET_STF", r: R, g: R, b: R });
    s = compositeReducer(s, { type: "PARK" });
    expect(s.parked?.autoStf).toBeNull();
    const shown = compositeReducer(s, { type: "SHOW_PARKED", parked: s.parked!, url: "lrgb" });
    expect(shown.autoStf).toEqual({ r: null, g: null, b: null });
    expect(shown.stf).toEqual({ r: R, g: R, b: R });
  });

  it("RESET keeps the parked slot and keeps its STF live instead of dropping to identity", () => {
    const parked = compositeReducer(wizardComposite(), { type: "PARK" });
    const reset = compositeReducer(parked, { type: "RESET" });
    expect(reset.parked).toBe(parked.parked);
    expect(reset.previewUrl).toBeNull();
    expect(reset.stf).toEqual({ r: G, g: B, b: R });
    expect(reset.autoStf).toEqual({ r: R, g: G, b: B });
    expect(reset.linked).toBe(false);
  });

  it("INIT_RGB keeps the parked slot, and a null SET_PREVIEW_URL keeps it too", () => {
    const parked = compositeReducer(wizardComposite(), { type: "PARK" });
    const rgb = compositeReducer(parked, { type: "INIT_RGB", previewUrl: "asset://rgb.png", stfR: R, stfG: G, stfB: B });
    expect(rgb.parked).toBe(parked.parked);
    expect(compositeReducer(parked, { type: "SET_PREVIEW_URL", url: null }).parked).toBe(parked.parked);
  });

  it("a new composite on screen (non-null SET_PREVIEW_URL) and clearComposite (CLEAR) drop the parked slot", () => {
    const parked = compositeReducer(wizardComposite(), { type: "PARK" });
    expect(compositeReducer(parked, { type: "SET_PREVIEW_URL", url: "asset://composite_masked_1.png" }).parked).toBeNull();
    const cleared = compositeReducer(parked, { type: "CLEAR" });
    expect(cleared.parked).toBeNull();
    expect(cleared.stf).toEqual(INITIAL_COMPOSITE_STATE.stf);
    expect(cleared.version).toBeGreaterThan(parked.version);
  });

  it("PARK on an RGB file view hides it without replacing the parked wizard composite", () => {
    const parked = compositeReducer(wizardComposite(), { type: "PARK" });
    const fileView = compositeReducer(parked, { type: "INIT_RGB", previewUrl: "asset://rgb.png", stfR: R, stfG: G, stfB: B });
    const again = compositeReducer(fileView, { type: "PARK" });
    expect(again.parked).toBe(parked.parked);
    expect(again.previewUrl).toBeNull();
    const fresh = compositeReducer(INITIAL_COMPOSITE_STATE, { type: "INIT_RGB", previewUrl: "asset://rgb.png", stfR: R, stfG: G, stfB: B });
    expect(compositeReducer(fresh, { type: "PARK" }).parked).toBeNull();
  });

  it("SHOW_PARKED and DROP_PARKED for a slot that was replaced meanwhile change nothing but the version", () => {
    const parked = compositeReducer(wizardComposite(), { type: "PARK" });
    const newer = compositeReducer(parked, { type: "SET_PREVIEW_URL", url: "asset://rgb_composite_9.png" });
    const staleShow = compositeReducer(newer, { type: "SHOW_PARKED", parked: parked.parked!, url: "asset://old.png" });
    expect(staleShow.previewUrl).toBe("asset://rgb_composite_9.png");
    expect(staleShow.version).toBeGreaterThan(newer.version);
    const reparked = compositeReducer(newer, { type: "PARK" });
    expect(compositeReducer(reparked, { type: "DROP_PARKED", parked: parked.parked! })).toBe(reparked);
    expect(compositeReducer(reparked, { type: "DROP_PARKED", parked: reparked.parked! }).parked).toBeNull();
  });

  it("park, then a step completes, then Show composite: the step's PNG, STF and link flag come back", () => {
    const live = wizardComposite();
    const parked = compositeReducer(live, { type: "PARK" });
    const completed = compositeReducer(parked, { type: "REPLACE_PARKED", previewUrl: DECONV_URL, stf: STEP_STF });
    expect(completed.version).toBe(parked.version);
    expect(completed.previewUrl).toBeNull();
    expect(completed.parked).toEqual({ previewUrl: DECONV_URL, stf: { r: B, g: B, b: B }, autoStf: { r: B, g: B, b: B }, linked: true });
    const shown = compositeReducer(completed, { type: "SHOW_PARKED", parked: completed.parked!, url: completed.parked!.previewUrl });
    expect(shown.previewUrl).toBe(DECONV_URL);
    expect(shown.stf).toEqual({ r: B, g: B, b: B });
    expect(shown.autoStf).toEqual({ r: B, g: B, b: B });
    expect(shown.linked).toBe(true);
    expect(shown.parked).toBeNull();
  });

  it("with nothing on screen the live STF follows the replaced slot, so a later RESET keeps the step's STF", () => {
    const parked = compositeReducer(wizardComposite(), { type: "PARK" });
    const completed = compositeReducer(parked, { type: "REPLACE_PARKED", previewUrl: DECONV_URL, stf: STEP_STF });
    expect(completed.stf).toEqual({ r: B, g: B, b: B });
    expect(completed.autoStf).toEqual({ r: B, g: B, b: B });
    expect(completed.linked).toBe(true);
    const reset = compositeReducer(completed, { type: "RESET" });
    expect(reset.stf).toEqual({ r: B, g: B, b: B });
    expect(reset.parked).toBe(completed.parked);
  });

  it("a step without a display STF (stretched or toned output) swaps only the parked PNG", () => {
    const parked = compositeReducer(wizardComposite(), { type: "PARK" });
    const completed = compositeReducer(parked, { type: "REPLACE_PARKED", previewUrl: "asset://composite_chain_localContrast_1.png", stf: null });
    expect(completed.parked).toEqual({ ...parked.parked, previewUrl: "asset://composite_chain_localContrast_1.png" });
    expect(completed.stf).toBe(parked.stf);
    expect(completed.linked).toBe(parked.linked);
  });

  it("REPLACE_PARKED leaves an RGB file view's own STF alone and does nothing without a slot", () => {
    const parked = compositeReducer(wizardComposite(), { type: "PARK" });
    const fileView = compositeReducer(parked, { type: "INIT_RGB", previewUrl: "asset://rgb.png", stfR: R, stfG: G, stfB: B });
    const completed = compositeReducer(fileView, { type: "REPLACE_PARKED", previewUrl: DECONV_URL, stf: STEP_STF });
    expect(completed.parked?.previewUrl).toBe(DECONV_URL);
    expect(completed.previewUrl).toBe("asset://rgb.png");
    expect(completed.stf).toBe(fileView.stf);
    expect(completed.linked).toBe(false);
    const live = wizardComposite();
    expect(compositeReducer(live, { type: "REPLACE_PARKED", previewUrl: DECONV_URL, stf: STEP_STF })).toBe(live);
  });

  it("canShowParked holds while the slot is full and the screen shows no composite or an RGB file view", () => {
    const live = wizardComposite();
    expect(canShowParkedComposite(live)).toBe(false);
    const parked = compositeReducer(live, { type: "PARK" });
    expect(canShowParkedComposite(parked)).toBe(true);
    const fileView = compositeReducer(parked, { type: "INIT_RGB", previewUrl: "asset://rgb.png", stfR: R, stfG: G, stfB: B });
    expect(canShowParkedComposite(fileView)).toBe(true);
    const shown = compositeReducer(fileView, { type: "SHOW_PARKED", parked: fileView.parked!, url: "u" });
    expect(canShowParkedComposite(shown)).toBe(false);
    expect(canShowParkedComposite(compositeReducer(parked, { type: "CLEAR" }))).toBe(false);
  });
});
