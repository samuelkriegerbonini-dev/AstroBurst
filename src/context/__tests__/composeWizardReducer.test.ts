import { describe, it, expect, vi, beforeEach } from "vitest";
import { createElement } from "react";
import { prerender } from "react-dom/static";
import { CompositeProvider } from "../CompositeContext";
import { ComposeWizardProvider, useComposeWizardContext, wizardReducer, type WizardStackRun } from "../ComposeWizardContext";
import { INITIAL_STATE, type WizardState } from "../../utils/wizard";
import { cancelProgress } from "../../services/progress";
import { STACK_PROGRESS_EVENT } from "../../shared/types/stacking";

vi.mock("../../services/progress", () => ({ cancelProgress: vi.fn(() => Promise.resolve(true)) }));

const stretched = { path: "/out/r_stretched.fits", note: "stretched" };
const starless = { path: "/out/r_starless.fits", note: "starless" };

function composedState(): WizardState {
  return {
    ...INITIAL_STATE,
    alignedPaths: { r: "/out/r_aligned.fits" },
    compositeReady: true,
    channelResults: { r: { starless, stretched } },
    completedSteps: {
      channels: true,
      stack: true,
      align: true,
      blend: true,
      colorbalance: true,
      stretch: true,
      adjust: true,
      export: true,
    },
  };
}

describe("wizardReducer INVALIDATE_FROM stretch", () => {
  it("clears stretch, adjust and export and keeps the composite and the channel results", () => {
    const before = composedState();
    const after = wizardReducer(before, { type: "INVALIDATE_FROM", stepId: "stretch" });
    expect(after.completedSteps).toEqual({ channels: true, stack: true, align: true, blend: true, colorbalance: true });
    expect(after.compositeReady).toBe(true);
    expect(after.channelResults).toEqual({ r: { starless, stretched } });
    expect(after.alignedPaths).toEqual({ r: "/out/r_aligned.fits" });
  });
});

type WizardContextValue = ReturnType<typeof useComposeWizardContext>;

async function mountWizard(): Promise<WizardContextValue> {
  const seen: WizardContextValue[] = [];
  function Capture(): null {
    seen.push(useComposeWizardContext());
    return null;
  }
  await prerender(createElement(CompositeProvider, {
    children: createElement(ComposeWizardProvider, { children: createElement(Capture) }),
  }));
  return seen[0];
}

function run(token: number): WizardStackRun {
  return { binId: "r", startedAt: 0, batch: null, token, files: ["/a.fits", "/b.fits", "/c.fits"] };
}

describe("wizard provider stack run", () => {
  beforeEach(() => {
    vi.mocked(cancelProgress).mockClear();
  });

  it("bumps the stack generation on every RESET, read through getStackGeneration", async () => {
    const ctx = await mountWizard();
    expect(ctx.getStackGeneration()).toBe(0);
    ctx.dispatch({ type: "RESET" });
    expect(ctx.getStackGeneration()).toBe(1);
    ctx.dispatch({ type: "UPDATE", partial: {} });
    expect(ctx.getStackGeneration()).toBe(1);
    ctx.dispatch({ type: "RESET" });
    expect(ctx.getStackGeneration()).toBe(2);
  });

  it("cancels the stack on RESET only while one is running", async () => {
    const ctx = await mountWizard();
    ctx.dispatch({ type: "RESET" });
    expect(cancelProgress).not.toHaveBeenCalled();
    ctx.setStackRun(run(1));
    ctx.dispatch({ type: "RESET" });
    expect(cancelProgress).toHaveBeenCalledTimes(1);
    expect(cancelProgress).toHaveBeenCalledWith(STACK_PROGRESS_EVENT);
    ctx.dispatch({ type: "RESET" });
    expect(cancelProgress).toHaveBeenCalledTimes(1);
  });

  it("lets only the current run's finisher clear the stack run", async () => {
    const ctx = await mountWizard();
    ctx.setStackRun(run(2));
    ctx.finishStackRun(1);
    ctx.dispatch({ type: "RESET" });
    expect(cancelProgress).toHaveBeenCalledTimes(1);
    ctx.setStackRun(run(3));
    ctx.finishStackRun(3);
    ctx.dispatch({ type: "RESET" });
    expect(cancelProgress).toHaveBeenCalledTimes(1);
  });
});
