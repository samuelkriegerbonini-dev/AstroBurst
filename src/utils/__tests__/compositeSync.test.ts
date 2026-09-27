import { describe, it, expect } from "vitest";
import { wizardStepStaleAfterCompositeWrite } from "../compositeSync";
import { INITIAL_STATE, invalidateDownstream, type WizardState } from "../wizard";

describe("wizard state after a composite write", () => {
  it("reopens Stretch, Adjust and Export of a blended wizard, whose stretched and toned tiers the write dropped", () => {
    const done: WizardState = {
      ...INITIAL_STATE,
      compositeReady: true,
      completedSteps: { channels: true, blend: true, colorbalance: true, stretch: true, adjust: true, export: true },
    };
    const step = wizardStepStaleAfterCompositeWrite(done.compositeReady);
    expect(step).toBe("stretch");
    const after = { ...done, ...invalidateDownstream(done, step!) };
    expect(after.completedSteps).toEqual({ channels: true, blend: true, colorbalance: true });
    expect(after.compositeReady).toBe(true);
  });

  it("leaves a wizard without a blended composite alone", () => {
    expect(wizardStepStaleAfterCompositeWrite(false)).toBeNull();
  });
});
