import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ChannelStage, WizardState } from "../../../utils/wizard";

type Stf = { shadow: number; midtone: number; highlight: number };

interface CapturedStretchProps {
  onResult: (png: string | null, stf?: { r: Stf; g: Stf; b: Stf }) => void;
  onChannelOutput: (binId: string, stage: "starless" | "stretched", value: ChannelStage) => void;
  onPreview?: (url: string, stf?: { r: Stf; g: Stf; b: Stf }) => void;
  onStarRemoval?: () => void;
}

const { captured, current, dispatchMock, actions } = vi.hoisted(() => ({
  captured: { stretch: null as CapturedStretchProps | null },
  current: { state: null as WizardState | null },
  dispatchMock: vi.fn(),
  actions: {
    setCompositePreviewUrl: vi.fn(),
    setCompositeAutoStf: vi.fn(),
    setCompositeStf: vi.fn(),
    setCompositeStfLinked: vi.fn(),
  },
}));

vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({ file: null }),
  useHistContext: () => ({ histData: null, stfParams: { shadow: 0, midtone: 0.5, highlight: 1 } }),
  useDoneFilesContext: () => ({ doneFiles: [] }),
  useNarrowbandContext: () => ({ narrowbandPalette: null, narrowbandFilters: [] }),
  useRenderActions: () => ({ forgetOutputs: () => {} }),
}));

vi.mock("../../../context/CompositeContext", () => ({
  useCompositeActions: () => actions,
}));

vi.mock("../../../context/ComposeWizardContext", () => ({
  useComposeWizardContext: () => ({
    state: current.state,
    dispatch: dispatchMock,
    activeStep: "stretch",
    setActiveStep: () => {},
    setOutputForgetter: () => {},
    setCompositeDims: () => {},
  }),
}));

vi.mock("../steps/StretchStep", () => ({
  default: (props: CapturedStretchProps) => {
    captured.stretch = props;
    return null;
  },
}));

import ComposeWizard from "../ComposeWizard";
import { INITIAL_STATE, STEPS } from "../../../utils/wizard";

const HA_FILES = { ha: ["/d/656nmos.fits"] };

function stateWith(completedSteps: Record<string, boolean>, compositeReady = false): WizardState {
  return {
    ...INITIAL_STATE,
    bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: HA_FILES[b.id as keyof typeof HA_FILES] ?? [] })),
    completedSteps,
    compositeReady,
  };
}

async function render(state: WizardState): Promise<{ html: string; props: CapturedStretchProps }> {
  current.state = state;
  captured.stretch = null;
  let html = renderToStaticMarkup(createElement(ComposeWizard));
  if (!captured.stretch) {
    await import("../steps/StretchStep");
    await new Promise((resolve) => setTimeout(resolve, 0));
    html = renderToStaticMarkup(createElement(ComposeWizard));
  }
  const props = captured.stretch as CapturedStretchProps | null;
  if (!props) throw new Error("StretchStep was not rendered");
  return { html, props };
}

const dispatched = () => dispatchMock.mock.calls.map((call) => call[0]);

function stepTab(html: string, id: string): string {
  const tag = html.match(new RegExp(`<button[^>]*data-testid="step-${id}"[^>]*>`));
  expect(tag, `step-${id} tab`).not.toBeNull();
  return tag?.[0] ?? "";
}

const STF = {
  r: { shadow: 0.01, midtone: 0.2, highlight: 1 },
  g: { shadow: 0.02, midtone: 0.3, highlight: 1 },
  b: { shadow: 0.03, midtone: 0.4, highlight: 1 },
};

describe("Compose wizard Stretch completion", () => {
  beforeEach(() => {
    dispatchMock.mockReset();
    for (const spy of Object.values(actions)) spy.mockReset();
  });

  it("stores a channel's starless output, then invalidates Stretch instead of completing it", async () => {
    const { props } = await render(stateWith({ channels: true, stretch: true }));
    const value = { path: "/out/656nmos_starless.fits", note: "star removal 4.0 sigma, growth 3.00x FWHM" };
    props.onChannelOutput("ha", "starless", value);
    expect(dispatched()).toEqual([
      { type: "SET_CHANNEL_STAGE", binId: "ha", stage: "starless", value },
      { type: "INVALIDATE_FROM", stepId: "stretch" },
    ]);
  });

  it("stores a channel's stretched output, then completes Stretch", async () => {
    const { props } = await render(stateWith({ channels: true }));
    const value = { path: "/out/656nmos_stretched.fits", note: "arcsinh stretch (factor 50)" };
    props.onChannelOutput("ha", "stretched", value);
    expect(dispatched()).toEqual([
      { type: "SET_CHANNEL_STAGE", binId: "ha", stage: "stretched", value },
      { type: "COMPLETE_STEP", stepId: "stretch" },
    ]);
  });

  it("shows the composite star-removal preview without completing Stretch, then invalidates it", async () => {
    const { props } = await render(stateWith({ channels: true, blend: true, stretch: true }, true));
    expect(typeof props.onPreview).toBe("function");
    expect(typeof props.onStarRemoval).toBe("function");
    props.onPreview?.("asset://out/rgb_starless.png", STF);
    expect(actions.setCompositePreviewUrl).toHaveBeenCalledWith("asset://out/rgb_starless.png");
    expect(actions.setCompositeStf).toHaveBeenCalledWith(STF.r, STF.g, STF.b);
    expect(dispatched()).toEqual([]);
    props.onStarRemoval?.();
    expect(dispatched()).toEqual([{ type: "INVALIDATE_FROM", stepId: "stretch" }]);
  });

  it("completes Stretch when the composite stretch run finishes", async () => {
    const { props } = await render(stateWith({ channels: true, blend: true }, true));
    props.onResult("asset://out/rgb_stretched.png", STF);
    expect(actions.setCompositePreviewUrl).toHaveBeenCalledWith("asset://out/rgb_stretched.png");
    expect(actions.setCompositeStf).toHaveBeenCalledWith(STF.r, STF.g, STF.b);
    expect(dispatched()).toEqual([{ type: "COMPLETE_STEP", stepId: "stretch" }]);
  });

  it("gives every step tab a step id and marks only completed steps", async () => {
    const done = await render(stateWith({ channels: true, blend: true, stretch: true }, true));
    for (const step of STEPS) stepTab(done.html, step.id);
    expect(stepTab(done.html, "stretch")).toContain('data-complete="true"');
    expect(stepTab(done.html, "blend")).toContain('data-complete="true"');
    expect(stepTab(done.html, "adjust")).not.toContain("data-complete");
    const invalidated = await render(stateWith({ channels: true, blend: true }, true));
    expect(stepTab(invalidated.html, "stretch")).not.toContain("data-complete");
    expect(stepTab(invalidated.html, "blend")).toContain('data-complete="true"');
  });
});
