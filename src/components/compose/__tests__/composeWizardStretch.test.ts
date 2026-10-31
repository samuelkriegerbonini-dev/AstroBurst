import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ChannelStage, WizardState } from "../../../utils/wizard";

type Stf = { shadow: number; midtone: number; highlight: number };
type StfTriple = { r: Stf; g: Stf; b: Stf; linked: boolean };

interface CapturedStretchProps {
  onResult: (png: string | null, stf?: { r: Stf; g: Stf; b: Stf }, bakedStf?: StfTriple | null) => void;
  onChannelOutput: (binId: string, stage: "starless" | "stretched", value: ChannelStage) => void;
  onPreview?: (url: string, stf?: { r: Stf; g: Stf; b: Stf }) => void;
  onStarlessComposite?: (url: string | null, stf: StfTriple) => void;
  onStarRemoval?: () => void;
}

interface CapturedColorProps {
  onResult: (png: string | null, stf?: StfTriple) => void;
}

interface CapturedHistogramProps {
  channels: { key: string; color: string }[];
  symmetryPoint?: number;
}

const { captured, current, dispatchMock, actions, onScreen } = vi.hoisted(() => ({
  captured: {
    stretch: null as CapturedStretchProps | null,
    color: null as CapturedColorProps | null,
    histograms: [] as CapturedHistogramProps[],
  },
  current: { state: null as WizardState | null, activeStep: "stretch", realStretch: false },
  dispatchMock: vi.fn(),
  actions: {
    setCompositePreviewUrl: vi.fn(),
    setCompositeAutoStf: vi.fn(),
    setCompositeStf: vi.fn(),
    setCompositeStfLinked: vi.fn(),
  },
  onScreen: {
    stf: {
      r: { shadow: 0.02, midtone: 0.2, highlight: 1 },
      g: { shadow: 0.03, midtone: 0.3, highlight: 1 },
      b: { shadow: 0.04, midtone: 0.4, highlight: 1 },
    },
    autoStf: {
      r: { shadow: 0.0043, midtone: 0.12, highlight: 1 },
      g: { shadow: 0.005, midtone: 0.25, highlight: 1 },
      b: { shadow: 0.006, midtone: 0.35, highlight: 1 },
    },
    linked: false,
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
  useCompositeStf: () => ({
    compositeStfR: onScreen.stf.r,
    compositeStfG: onScreen.stf.g,
    compositeStfB: onScreen.stf.b,
    compositeStfLinked: onScreen.linked,
    compositeAutoStfR: onScreen.autoStf.r,
    compositeAutoStfG: onScreen.autoStf.g,
    compositeAutoStfB: onScreen.autoStf.b,
    wizardStfR: onScreen.stf.r,
    wizardStfG: onScreen.stf.g,
    wizardStfB: onScreen.stf.b,
    wizardStfLinked: onScreen.linked,
  }),
}));

vi.mock("../StfHistogram", () => ({
  default: (props: CapturedHistogramProps) => {
    captured.histograms.push(props);
    return null;
  },
}));

vi.mock("../../../context/ComposeWizardContext", () => ({
  useComposeWizardContext: () => ({
    state: current.state,
    dispatch: dispatchMock,
    activeStep: current.activeStep,
    setActiveStep: () => {},
    setOutputForgetter: () => {},
    setCompositeDims: () => {},
  }),
}));

vi.mock("../steps/ColorBalanceStep", () => ({
  default: (props: CapturedColorProps) => {
    captured.color = props;
    return null;
  },
}));

vi.mock("../steps/StretchStep", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../steps/StretchStep")>();
  return {
    default: (props: CapturedStretchProps) => {
      captured.stretch = props;
      return current.realStretch ? createElement(actual.default, props as Parameters<typeof actual.default>[0]) : null;
    },
  };
});

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

  it("adopts the auto STF recomputed for the starless composite as the auto, current and link state", async () => {
    const { props } = await render(stateWith({ channels: true, blend: true, stretch: true }, true));
    expect(typeof props.onStarlessComposite).toBe("function");
    props.onStarlessComposite?.("asset://out/rgb_starless.png", { ...STF, linked: true });
    expect(actions.setCompositePreviewUrl).toHaveBeenCalledWith("asset://out/rgb_starless.png");
    expect(actions.setCompositeAutoStf).toHaveBeenCalledWith(STF.r, STF.g, STF.b);
    expect(actions.setCompositeStf).toHaveBeenCalledWith(STF.r, STF.g, STF.b);
    expect(actions.setCompositeStfLinked).toHaveBeenCalledWith(true);
    expect(dispatched()).toEqual([]);
  });

  it("completes Stretch when the composite stretch run finishes, recording that no STF describes the bake", async () => {
    const { props } = await render(stateWith({ channels: true, blend: true }, true));
    props.onResult("asset://out/composite_ghs_1.png", STF, null);
    expect(actions.setCompositePreviewUrl).toHaveBeenCalledWith("asset://out/composite_ghs_1.png");
    expect(actions.setCompositeStf).toHaveBeenCalledWith(STF.r, STF.g, STF.b);
    expect(dispatched()).toEqual([
      { type: "UPDATE", partial: { bakedStf: null } },
      { type: "COMPLETE_STEP", stepId: "stretch" },
    ]);
  });

  it("records the STF and link mode that Apply stretch baked into the composite", async () => {
    const { props } = await render(stateWith({ channels: true, blend: true }, true));
    const baked = { ...STF, linked: false };
    props.onResult("asset://out/rgb_composite_3.png", STF, baked);
    expect(dispatched()).toEqual([
      { type: "UPDATE", partial: { bakedStf: baked } },
      { type: "COMPLETE_STEP", stepId: "stretch" },
    ]);
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

async function renderStretchStep(state: WizardState): Promise<string> {
  current.realStretch = true;
  captured.histograms = [];
  try {
    return (await render(state)).html;
  } finally {
    current.realStretch = false;
  }
}

function sliderValues(html: string, label: string): string[] {
  return [...html.matchAll(new RegExp(`aria-label="Edit ${label} value"[^>]*>([^<]*)</button>`, "g"))].map((m) => m[1]);
}

const onScreenTriple = () => ({ ...onScreen.stf, linked: onScreen.linked });

function autoStfState(completedSteps: Record<string, boolean>, bakedStf: StfTriple | null = null): WizardState {
  return { ...stateWith(completedSteps, true), stretchMode: "auto_stf", bakedStf };
}

describe("Compose wizard Stretch commit", () => {
  beforeEach(() => {
    dispatchMock.mockReset();
    for (const spy of Object.values(actions)) spy.mockReset();
  });

  it("the sliders start from the composite STF on screen, not the auto STF", async () => {
    const html = await renderStretchStep(autoStfState({ channels: true, blend: true }));
    expect(sliderValues(html, "Shadow")).toEqual(["0.0200", "0.0300", "0.0400"]);
    expect(sliderValues(html, "Midtone")).toEqual(["0.2000", "0.3000", "0.4000"]);
  });

  it("names the commit button Apply stretch, keeps Reset to Auto STF and says nothing is applied yet", async () => {
    const html = await renderStretchStep(autoStfState({ channels: true, blend: true }));
    expect(html).toContain(">Apply stretch</span>");
    expect(html).not.toContain("Re-stretch Composite");
    expect(html).toContain("Reset to Auto STF");
    expect(html).toContain("Not applied: Adjust and Export use the composite STF");
  });

  it("says the stretch is applied when the bake holds the STF on screen", async () => {
    const html = await renderStretchStep(autoStfState({ channels: true, blend: true, stretch: true }, onScreenTriple()));
    expect(html).toContain("Applied (STF shown): Adjust and Export use this stretch");
  });

  it("says the sliders changed when the bake is not the STF they show", async () => {
    const ghsBake = await renderStretchStep(autoStfState({ channels: true, blend: true, stretch: true }, null));
    expect(ghsBake).toContain("Changed since applied: press Apply stretch");
    const otherStf = { ...onScreenTriple(), r: { ...onScreen.stf.r, midtone: 0.3 } };
    const olderBake = await renderStretchStep(autoStfState({ channels: true, blend: true, stretch: true }, otherStf));
    expect(olderBake).toContain("Changed since applied: press Apply stretch");
  });

  it("with Adjust done, the discards notice names Adjust above the Apply stretch button", async () => {
    const html = await renderStretchStep(autoStfState({ channels: true, blend: true, stretch: true, adjust: true }, onScreenTriple()));
    const notice = html.indexOf("Apply stretch discards: Adjust");
    expect(notice).toBeGreaterThan(-1);
    expect(notice).toBeLessThan(html.indexOf(">Apply stretch</span>"));
    expect(html).toContain("Adjusted: Export uses the Adjust result");
  });

  it("shows no discards notice while Adjust has nothing to lose", async () => {
    const html = await renderStretchStep(autoStfState({ channels: true, blend: true, stretch: true }, onScreenTriple()));
    expect(html).not.toContain("discards");
  });

  it("GHS mode shows the composite histogram with an SP marker at the symmetry point", async () => {
    await renderStretchStep({ ...stateWith({ channels: true, blend: true }, true), stretchMode: "ghs" });
    const ghs = captured.histograms.filter((h) => h.symmetryPoint !== undefined);
    expect(ghs).toHaveLength(1);
    expect(ghs[0].symmetryPoint).toBe(0.01);
    expect(ghs[0].channels.map((c) => c.key)).toEqual(["__composite_r", "__composite_g", "__composite_b"]);
  });

  it("GHS mode on one channel reads the histogram of the file the stretch runs on", async () => {
    await renderStretchStep({ ...stateWith({ channels: true }), stretchMode: "ghs" });
    const ghs = captured.histograms.filter((h) => h.symmetryPoint !== undefined);
    expect(ghs).toHaveLength(1);
    expect(ghs[0].channels.map((c) => c.key)).toEqual(["/d/656nmos.fits"]);
  });
});

describe("Compose wizard Color Balance result", () => {
  beforeEach(() => {
    dispatchMock.mockReset();
    for (const spy of Object.values(actions)) spy.mockReset();
  });

  async function renderColor(): Promise<CapturedColorProps> {
    current.state = stateWith({ channels: true, blend: true }, true);
    current.activeStep = "colorbalance";
    captured.color = null;
    try {
      renderToStaticMarkup(createElement(ComposeWizard));
      await import("../steps/ColorBalanceStep");
      await new Promise((resolve) => setTimeout(resolve, 0));
      renderToStaticMarkup(createElement(ComposeWizard));
    } finally {
      current.activeStep = "stretch";
    }
    if (!captured.color) throw new Error("ColorBalanceStep was not rendered");
    return captured.color;
  }

  it("keeps the unlinked per-channel STF a neutral white balance returned", async () => {
    const props = await renderColor();
    props.onResult("asset://out/rgb_composite_2.png", { ...STF, linked: false });
    expect(actions.setCompositePreviewUrl).toHaveBeenCalledWith("asset://out/rgb_composite_2.png");
    expect(actions.setCompositeAutoStf).toHaveBeenCalledWith(STF.r, STF.g, STF.b);
    expect(actions.setCompositeStf).toHaveBeenCalledWith(STF.r, STF.g, STF.b);
    expect(actions.setCompositeStfLinked).toHaveBeenCalledWith(false);
    expect(dispatched()).toEqual([{ type: "COMPLETE_STEP", stepId: "colorbalance" }]);
  });
});
