import { describe, it, expect } from "vitest";
import {
  bumpsCompositeVersion,
  compositeReducer,
  INITIAL_COMPOSITE_STATE,
  liveCompositeVersion,
  noteCompositeAction,
  type CompositeAction,
} from "../CompositeContext";

const R = { shadow: 0.02, midtone: 0.017, highlight: 1 };
const G = { shadow: 0.01, midtone: 0.05, highlight: 1 };
const B = { shadow: 0.03, midtone: 0.02, highlight: 1 };

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
      { type: "RESET" },
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
