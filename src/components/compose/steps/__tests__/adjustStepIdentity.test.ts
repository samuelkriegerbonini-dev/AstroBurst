import { describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";

const STF = { shadow: 0.02, midtone: 0.2, highlight: 1 };

vi.mock("react", async (importOriginal) => (await import("../../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../../context/CompositeContext", () => ({
  useCompositeStf: () => ({ wizardStfR: STF, wizardStfG: STF, wizardStfB: STF, wizardStfLinked: true }),
}));
vi.mock("../../../../context/WizardRunGuardContext", async () => {
  const { UNGUARDED_RUNS } = await import("../../../../utils/wizardRunGuard");
  return { useWizardRunGuard: () => UNGUARDED_RUNS };
});

import { hooks } from "../../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../../test/elementTree";
import AdjustStep from "../AdjustStep";
import CurveEditor from "../../CurveEditor";
import { INITIAL_STATE, type WizardState } from "../../../../utils/wizard";
import type { CurvePoint } from "../../../../services/tone";

describe("wizard Adjust curves", () => {
  it("counts a curve as modified exactly when the tone service will send it", () => {
    hooks.mount();
    const state: WizardState = { ...INITIAL_STATE, compositeReady: true };
    const view = () => hooks.render(AdjustStep, { state, onResult: () => {} }) as ReactNode;
    const editor = elementsOf<{ onChange: (points: CurvePoint[]) => void }>(view()).find((el) => el.type === CurveEditor);
    editor!.props.onChange([{ x: 0, y: 0 }, { x: 1, y: 0.99995 }]);
    expect(textOf(view())).toContain("Modified");
  });
});
