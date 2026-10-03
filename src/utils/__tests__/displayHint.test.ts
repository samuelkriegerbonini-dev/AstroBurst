import { describe, it, expect } from "vitest";
import {
  displayForRecordChange,
  initialHintedDisplay,
  patchHintedDisplay,
  persistableDisplay,
  type HintSource,
  type HintedDisplay,
} from "../displayHint";
import { displayHintFor } from "../lineFit";
import { DEFAULT_DISPLAY_SETTINGS, type DisplaySettings } from "../../shared/types/display";

const USER: DisplaySettings = {
  ...DEFAULT_DISPLAY_SETTINGS,
  stretch: "asinh",
  limits: "zscale",
  colormap: "magma",
  invert: true,
  percentileLow: 2,
  grid: true,
  symmetric: false,
  centre: 5,
};

const VELOCITY: HintSource = { displayHint: displayHintFor("velocity") };
const FLUX: HintSource = { displayHint: displayHintFor("flux") };
const MASK: HintSource = { displayHint: displayHintFor("mask") };
const MOMENT: HintSource = {};
const FRAME: HintSource = {};

interface Session {
  state: HintedDisplay;
  shown: HintSource | null;
  stored: DisplaySettings;
}

function startSession(stored: DisplaySettings): Session {
  return { state: initialHintedDisplay(stored), shown: null, stored };
}

function show(session: Session, record: HintSource | null): Session {
  return { ...session, state: displayForRecordChange(session.state, session.shown, record), shown: record };
}

function edit(session: Session, patch: Partial<DisplaySettings>): Session {
  const state = patchHintedDisplay(session.state, patch);
  const persisted = persistableDisplay(state);
  return { ...session, state, stored: persisted ?? session.stored };
}

describe("display hint transitions", () => {
  it("non-hinted -> hinted -> hinted -> non-hinted restores the original settings exactly", () => {
    let s = startSession(USER);
    s = show(s, MOMENT);
    expect(s.state.display).toBe(USER);
    s = show(s, VELOCITY);
    expect(s.state.display).toMatchObject({ colormap: "rdbu", invert: true, symmetric: true, centre: 0, stretch: "linear", limits: "percentile" });
    expect(s.state.display.grid).toBe(true);
    expect(s.state.snapshot).toBe(USER);
    s = show(s, FLUX);
    expect(s.state.display).toMatchObject({ colormap: "viridis", invert: false, symmetric: false, stretch: "linear", limits: "percentile" });
    expect(s.state.snapshot).toBe(USER);
    s = show(s, MASK);
    expect(s.state.display).toMatchObject({ colormap: "gray", limits: "minmax" });
    s = show(s, FRAME);
    expect(s.state.display).toBe(USER);
    expect(s.state.snapshot).toBeNull();
  });

  it("applies the hint without persisting it", () => {
    let s = startSession(USER);
    s = show(s, VELOCITY);
    expect(persistableDisplay(s.state)).toBeNull();
    expect(s.stored).toBe(USER);
  });

  it("shows a manual change made during a hint, does not persist it, and drops it on restore", () => {
    let s = startSession(USER);
    s = show(s, VELOCITY);
    s = edit(s, { colormap: "inferno", percentileHigh: 98 });
    expect(s.state.display.colormap).toBe("inferno");
    expect(s.state.display.percentileHigh).toBe(98);
    expect(s.stored).toBe(USER);
    s = show(s, FLUX);
    expect(s.state.display.colormap).toBe("viridis");
    expect(s.state.display.percentileHigh).toBe(98);
    s = show(s, MOMENT);
    expect(s.state.display).toBe(USER);
    expect(s.stored).toBe(USER);
  });

  it("restores on reset (no processed record) and re-snapshots the user's later settings on the next hint", () => {
    let s = startSession(USER);
    s = show(s, VELOCITY);
    s = show(s, null);
    expect(s.state).toEqual({ display: USER, snapshot: null });
    s = edit(s, { colormap: "cividis" });
    expect(s.stored.colormap).toBe("cividis");
    s = show(s, VELOCITY);
    expect(s.state.snapshot?.colormap).toBe("cividis");
    s = show(s, null);
    expect(s.state.display).toEqual({ ...USER, colormap: "cividis" });
  });

  it("follows file switches: hinted A -> B without a record restores, B -> hinted A applies the hint again", () => {
    const fileA: HintSource = { displayHint: displayHintFor("velocity") };
    const fileBMoment: HintSource = {};
    let s = startSession(USER);
    s = show(s, fileA);
    s = show(s, null);
    expect(s.state.display).toBe(USER);
    s = show(s, fileBMoment);
    expect(s.state.display).toBe(USER);
    s = edit(s, { stretch: "sqrt" });
    expect(s.stored.stretch).toBe("sqrt");
    s = show(s, fileA);
    expect(s.state.display.colormap).toBe("rdbu");
    expect(s.state.snapshot?.stretch).toBe("sqrt");
    s = show(s, fileBMoment);
    expect(s.state.display).toEqual({ ...USER, stretch: "sqrt" });
  });

  it("switching between two hinted files keeps the first snapshot", () => {
    const fileA: HintSource = { displayHint: displayHintFor("velocity") };
    const fileB: HintSource = { displayHint: displayHintFor("flux") };
    let s = startSession(USER);
    s = show(s, fileA);
    s = show(s, fileB);
    expect(s.state.snapshot).toBe(USER);
    expect(s.state.display.colormap).toBe("viridis");
    s = show(s, null);
    expect(s.state.display).toBe(USER);
  });

  it("does nothing when the displayed record object is unchanged (chain update on the same result)", () => {
    let s = startSession(USER);
    s = show(s, VELOCITY);
    s = edit(s, { colormap: "inferno" });
    const before = s.state;
    s = show(s, VELOCITY);
    expect(s.state).toBe(before);
    expect(s.state.display.colormap).toBe("inferno");
  });

  it("re-applies the hint when the same plane is published again as a new record", () => {
    let s = startSession(USER);
    s = show(s, VELOCITY);
    s = edit(s, { colormap: "inferno" });
    s = show(s, { displayHint: displayHintFor("velocity") });
    expect(s.state.display.colormap).toBe("rdbu");
    expect(s.state.snapshot).toBe(USER);
  });

  it("keeps non-hinted results identical to before: the state object is untouched and manual changes persist", () => {
    let s = startSession(USER);
    const initial = s.state;
    s = show(s, MOMENT);
    s = show(s, FRAME);
    s = show(s, null);
    expect(s.state).toBe(initial);
    s = edit(s, { symmetric: true });
    expect(s.state.display).toEqual({ ...USER, symmetric: true });
    expect(s.stored).toEqual({ ...USER, symmetric: true });
  });

  it("reconciles a manual patch the same way outside and inside a hint", () => {
    const mtfUser: DisplaySettings = { ...USER, stretch: "mtf" };
    const outside = patchHintedDisplay(initialHintedDisplay(mtfUser), { symmetric: true });
    expect(outside.display.stretch).toBe("linear");
    expect(outside.display.symmetric).toBe(true);
    const hinted = displayForRecordChange(initialHintedDisplay(mtfUser), null, FLUX);
    const inside = patchHintedDisplay(hinted, { stretch: "mtf" });
    expect(inside.display.stretch).toBe("mtf");
    expect(inside.display.symmetric).toBe(false);
    expect(inside.snapshot).toBe(mtfUser);
  });

  it("never leaves hinted settings in storage, so a reload starts from the user's own settings", () => {
    let s = startSession(USER);
    s = edit(s, { colormap: "plasma" });
    s = show(s, VELOCITY);
    s = edit(s, { invert: false, grid: false });
    s = show(s, MASK);
    s = edit(s, { colorbar: false });
    const reloaded = startSession(s.stored);
    expect(reloaded.state.snapshot).toBeNull();
    expect(reloaded.state.display).toEqual({ ...USER, colormap: "plasma" });
  });
});
