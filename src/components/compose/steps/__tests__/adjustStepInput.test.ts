import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const STF = { shadow: 0.02, midtone: 0.2, highlight: 1 };

vi.mock("../../../../context/CompositeContext", () => ({
  useCompositeStf: () => ({ wizardStfR: STF, wizardStfG: STF, wizardStfB: STF, wizardStfLinked: true }),
}));

import AdjustStep from "../AdjustStep";
import { INITIAL_STATE, type WizardState } from "../../../../utils/wizard";

function render(completedSteps: Record<string, boolean>, compositeReady: boolean): string {
  const state: WizardState = { ...INITIAL_STATE, completedSteps, compositeReady };
  return renderToStaticMarkup(createElement(AdjustStep, { state, onResult: () => {} }));
}

describe("wizard Adjust input line", () => {
  it("says the curves apply to the applied stretch once Stretch baked the composite", () => {
    expect(render({ channels: true, blend: true, stretch: true }, true)).toContain("Curves apply to: the applied stretch");
  });

  it("says the curves apply to the composite STF while nothing is applied", () => {
    const html = render({ channels: true, blend: true }, true);
    expect(html).toContain("Curves apply to: the composite STF");
    expect(html).not.toContain("the applied stretch");
  });

  it("names no input before Blend", () => {
    expect(render({ channels: true }, false)).not.toContain("Curves apply to");
  });
});
