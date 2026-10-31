import { describe, expect, it } from "vitest";
import {
  curvesInputLine,
  hasStretchBake,
  STRETCH_COMMIT_LINE,
  stretchCommitState,
} from "../stretchCommitState";

const SHOWN = {
  r: { shadow: 0.02, midtone: 0.2, highlight: 1 },
  g: { shadow: 0.03, midtone: 0.3, highlight: 1 },
  b: { shadow: 0.04, midtone: 0.4, highlight: 1 },
  linked: false,
};

const SLIDERS_ON_SCREEN = {
  r: { ...SHOWN.r },
  g: { ...SHOWN.g },
  b: { ...SHOWN.b },
  linked: false,
};

const MIDTONE_R_MOVED = { ...SLIDERS_ON_SCREEN, r: { ...SHOWN.r, midtone: 0.3 } };

describe("stretchCommitState", () => {
  it("is not applied while no bake exists and the sliders show the composite STF", () => {
    expect(stretchCommitState(SHOWN, SLIDERS_ON_SCREEN, false, false)).toBe("not_applied");
  });

  it("is applied when the bake holds the STF the sliders show", () => {
    expect(stretchCommitState(SHOWN, SLIDERS_ON_SCREEN, true, false)).toBe("applied");
  });

  it("is changed when a slider leaves the committed STF, with or without a bake", () => {
    expect(stretchCommitState(SHOWN, MIDTONE_R_MOVED, false, false)).toBe("changed");
    expect(stretchCommitState(SHOWN, MIDTONE_R_MOVED, true, false)).toBe("changed");
    expect(stretchCommitState(SHOWN, MIDTONE_R_MOVED, true, true)).toBe("changed");
  });

  it("is changed when the bake holds no STF, as after a GHS, masked or arcsinh stretch", () => {
    expect(stretchCommitState(null, SLIDERS_ON_SCREEN, true, false)).toBe("changed");
  });

  it("is changed when only the link mode differs from the committed STF", () => {
    expect(stretchCommitState(SHOWN, { ...SLIDERS_ON_SCREEN, linked: true }, true, false)).toBe("changed");
  });

  it("is adjusted when Adjust ran on the committed stretch, with or without a bake", () => {
    expect(stretchCommitState(SHOWN, SLIDERS_ON_SCREEN, true, true)).toBe("adjusted");
    expect(stretchCommitState(SHOWN, SLIDERS_ON_SCREEN, false, true)).toBe("adjusted");
  });

  it("gives each state one line that says what Adjust and Export use", () => {
    expect(STRETCH_COMMIT_LINE).toEqual({
      not_applied: "Not applied: Adjust and Export use the composite STF",
      applied: "Applied (STF shown): Adjust and Export use this stretch",
      changed: "Changed since applied: press Apply stretch",
      adjusted: "Adjusted: Export uses the Adjust result",
    });
  });
});

describe("hasStretchBake", () => {
  it("holds only for a completed Stretch on the blended composite", () => {
    expect(hasStretchBake({ compositeReady: true, completedSteps: { blend: true, stretch: true } })).toBe(true);
    expect(hasStretchBake({ compositeReady: true, completedSteps: { blend: true } })).toBe(false);
    expect(hasStretchBake({ compositeReady: false, completedSteps: { stretch: true } })).toBe(false);
  });
});

describe("curvesInputLine", () => {
  it("names the applied stretch when a bake exists, else the composite STF", () => {
    expect(curvesInputLine(true)).toBe("Curves apply to: the applied stretch");
    expect(curvesInputLine(false)).toBe("Curves apply to: the composite STF");
  });
});
