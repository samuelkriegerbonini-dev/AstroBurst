import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement } from "react";

const { clearComposite } = vi.hoisted(() => ({ clearComposite: vi.fn(async () => {}) }));

vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react")>();
  const runtime = await import("../../test/hookRuntime");
  const faked = runtime.reactWithFakeHooks(actual) as typeof actual;
  function useReducer(reducer: (state: unknown, action: unknown) => unknown, init: unknown): [unknown, (action: unknown) => void] {
    if (!runtime.hooks.run.active) return (actual.useReducer as unknown as typeof useReducer)(reducer, init);
    const [state, setState] = faked.useState(init);
    return [state, (action) => setState((current: unknown) => reducer(current, action))];
  }
  return { ...faked, useReducer };
});

vi.mock("../CompositeContext", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../CompositeContext")>()),
  useCompositeActions: () => ({ clearComposite }),
}));

vi.mock("../../services/progress", () => ({ cancelProgress: vi.fn(() => Promise.resolve(true)) }));

import { hooks } from "../../test/hookRuntime";
import { ComposeWizardProvider, useComposeWizardContext } from "../ComposeWizardContext";
import { cancelProgress } from "../../services/progress";
import { STACK_PROGRESS_EVENT } from "../../shared/types/stacking";

type WizardContextValue = ReturnType<typeof useComposeWizardContext>;

function renderProvider(): WizardContextValue {
  const tree = hooks.render(ComposeWizardProvider, { children: null }) as ReactElement<{ value: WizardContextValue }>;
  return tree.props.value;
}

beforeEach(() => {
  hooks.mount();
  vi.mocked(cancelProgress).mockClear();
});

describe("Compose wizard RESET with a stack running", () => {
  it("clears the stack run, bumps the stack generation and cancels the live stack once", () => {
    let ctx = renderProvider();
    ctx.setStackRun({ binId: "r", startedAt: 0, batch: null, token: 1, files: ["a"] });
    ctx = renderProvider();
    expect(ctx.stackRun).toMatchObject({ binId: "r", token: 1 });
    expect(ctx.stackGeneration).toBe(0);

    ctx.dispatch({ type: "RESET" });
    ctx = renderProvider();
    expect(ctx.stackRun).toBeNull();
    expect(ctx.stackGeneration).toBe(1);
    expect(ctx.getStackGeneration()).toBe(1);
    expect(cancelProgress).toHaveBeenCalledTimes(1);
    expect(cancelProgress).toHaveBeenCalledWith(STACK_PROGRESS_EVENT);
  });
});
