import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement } from "react";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

vi.mock("../CompositeContext", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../CompositeContext")>()),
  useCompositeActions: () => ({ clearComposite: vi.fn(async () => {}) }),
}));

vi.mock("../../infrastructure/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../infrastructure/tauri")>()),
  typedInvoke: invoke,
}));

import { hooks } from "../../test/hookRuntime";
import { ComposeWizardProvider, useComposeWizardContext } from "../ComposeWizardContext";
import { compositeChainReset } from "../../services/compositeChain";
import { compositeHistoryLines } from "../../utils/wizard";

type WizardContextValue = ReturnType<typeof useComposeWizardContext>;

function renderProvider(): WizardContextValue {
  const tree = hooks.render(ComposeWizardProvider, { children: null }) as ReactElement<{ value: WizardContextValue }>;
  return tree.props.value;
}

const DECONV = "Deconvolution RL: 20 iterations, PSF gaussian sigma 2 px size 15";

beforeEach(() => {
  hooks.mount();
  invoke.mockReset();
});

describe("Composite chain reset seen by the wizard HISTORY", () => {
  it("drops the chain lines when the reset restored the composite, and keeps them when it did not", async () => {
    const ctx = renderProvider();
    ctx.dispatch({ type: "RECORD_COMPOSITE_OP", op: { kind: "blend", preset: "hoo" } });
    ctx.dispatch({ type: "RECORD_COMPOSITE_OP", op: { kind: "chain", lines: [DECONV], restarted: false } });
    invoke.mockResolvedValueOnce({ restored: false, live_generation: 4 });
    await compositeChainReset();
    expect(compositeHistoryLines(ctx.getState().compositeHistory)).toEqual(["Blend: hoo", DECONV]);
    ctx.dispatch({ type: "RECORD_COMPOSITE_OP", op: { kind: "chain", lines: [DECONV], restarted: true } });
    invoke.mockResolvedValueOnce({ restored: true, live_generation: 5 });
    await compositeChainReset();
    expect(compositeHistoryLines(ctx.getState().compositeHistory)).toEqual(["Blend: hoo", DECONV]);
    ctx.dispatch({ type: "RECORD_COMPOSITE_OP", op: { kind: "chain", lines: [DECONV], restarted: false } });
    invoke.mockResolvedValueOnce({ restored: true, live_generation: 6 });
    await compositeChainReset();
    expect(compositeHistoryLines(ctx.getState().compositeHistory)).toEqual(["Blend: hoo", DECONV]);
  });
});
