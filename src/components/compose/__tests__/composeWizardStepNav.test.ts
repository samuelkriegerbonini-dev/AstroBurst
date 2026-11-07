import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { WizardState } from "../../../utils/wizard";

const { current, mocks } = vi.hoisted(() => ({
  current: { state: null as WizardState | null, activeStep: "channels" },
  mocks: { dispatch: vi.fn(), setActiveStep: vi.fn() },
}));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({ file: null }),
  useHistContext: () => ({ histData: null, stfParams: { shadow: 0, midtone: 0.5, highlight: 1 } }),
  useDoneFilesContext: () => ({ doneFiles: [] }),
  useNarrowbandContext: () => ({ narrowbandPalette: null, narrowbandFilters: [] }),
  useRenderActions: () => ({ forgetOutputs: () => {} }),
}));

vi.mock("../../../context/CompositeContext", () => ({
  useCompositeActions: () => ({
    setCompositePreviewUrl: () => {},
    setCompositeAutoStf: () => {},
    setCompositeStf: () => {},
    setCompositeStfLinked: () => {},
  }),
}));

vi.mock("../../../context/ComposeWizardContext", () => ({
  useComposeWizardContext: () => ({
    state: current.state,
    dispatch: mocks.dispatch,
    activeStep: current.activeStep,
    setActiveStep: mocks.setActiveStep,
    setOutputForgetter: () => {},
    setCompositeDims: () => {},
  }),
}));

vi.mock("../steps/ChannelStep", () => ({ default: () => null }));
vi.mock("../steps/StackStep", () => ({ default: () => null }));
vi.mock("../steps/AlignStep", () => ({ default: () => null }));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import ComposeWizard, { StepNavFooter } from "../ComposeWizard";
import { INITIAL_STATE } from "../../../utils/wizard";

const M16 = { r: ["/m16/673nmos.fits"], g: ["/m16/656nmos.fits"], b: ["/m16/502nmos.fits"] };
const M16_ALIGNED = { r: "__wizard_ch_tk1_r_aligned", g: "__wizard_ch_tk1_g_aligned", b: "__wizard_ch_tk1_b_aligned" };

function stateWith(files: Record<string, string[]>, extra: Partial<WizardState> = {}): WizardState {
  return {
    ...INITIAL_STATE,
    bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: files[b.id] ?? [] })),
    ...extra,
  };
}

function renderWizard(state: WizardState, activeStep: string): string {
  current.state = state;
  current.activeStep = activeStep;
  return renderToStaticMarkup(createElement(ComposeWizard));
}

function footerHtml(html: string): string {
  const start = html.indexOf('data-testid="wizard-step-nav"');
  expect(start, "step footer").toBeGreaterThan(-1);
  return html.slice(start, html.indexOf("</nav>", start));
}

function footerButtons(html: string): { text: string; disabled: boolean }[] {
  return [...footerHtml(html).matchAll(/<button([^>]*)>(.*?)<\/button>/g)].map((m) => ({
    text: m[2].replace(/<[^>]+>/g, ""),
    disabled: /\sdisabled=""/.test(m[1]) || /\saria-disabled="true"/.test(m[1]),
  }));
}

function nextButtonTag(html: string): string {
  const tag = /<button[^>]*data-testid="wizard-step-next"[^>]*>/.exec(footerHtml(html))?.[0];
  expect(tag, "Next button").toBeDefined();
  return tag ?? "";
}

interface Node {
  type: unknown;
  props: Record<string, unknown> & { children?: ReactNode };
}

function footerOfWizard(state: WizardState, activeStep: string): Node[] {
  current.state = state;
  current.activeStep = activeStep;
  hooks.mount();
  const wizard = elementsOf(hooks.render(ComposeWizard, undefined));
  const footer = wizard.find((n) => n.type === StepNavFooter);
  expect(footer, "ComposeWizard renders the step footer").toBeDefined();
  return elementsOf(StepNavFooter(footer!.props as unknown as Parameters<typeof StepNavFooter>[0]));
}

function wizardNodes(state: WizardState, activeStep: string): Node[] {
  current.state = state;
  current.activeStep = activeStep;
  return elementsOf(hooks.render(ComposeWizard, undefined));
}

function byTestId(tree: Node[], testId: string): Node {
  const node = tree.find((n) => n.props["data-testid"] === testId);
  expect(node, testId).toBeDefined();
  return node!;
}

function noteOf(tree: Node[]): Node | undefined {
  return tree.find((n) => n.props.role === "note");
}

function click(footer: Node[], testId: string): void {
  const button = footer.find((n) => n.props["data-testid"] === testId);
  expect(button, testId).toBeDefined();
  (button!.props.onClick as () => void)();
}

beforeEach(() => {
  mocks.dispatch.mockReset();
  mocks.setActiveStep.mockReset();
});

describe("Compose wizard step footer", () => {
  it("identifies only the current workflow step independently of its visual styling", () => {
    const html = renderWizard(stateWith(M16), "channels");
    const currentStep = [...html.matchAll(/<button[^>]*aria-current="step"[^>]*>/g)].map((match) => match[0]);
    expect(currentStep).toHaveLength(1);
    expect(currentStep[0]).toContain('data-testid="step-channels"');
    const aligned = stateWith(M16, { alignedPaths: M16_ALIGNED, completedSteps: { align: true } });
    const changed = renderWizard(aligned, "align");
    const nextCurrent = [...changed.matchAll(/<button[^>]*aria-current="step"[^>]*>/g)].map((match) => match[0]);
    expect(nextCurrent).toHaveLength(1);
    expect(nextCurrent[0]).toContain('data-testid="step-align"');
  });

  it("Next skips a disabled Stack step and lands on Align for single-file channels", () => {
    expect(footerButtons(renderWizard(stateWith(M16), "channels"))).toEqual([{ text: "Next: Align", disabled: false }]);
    const footer = footerOfWizard(stateWith(M16), "channels");
    mocks.dispatch.mockReset();
    click(footer, "wizard-step-next");
    expect(mocks.setActiveStep).toHaveBeenCalledWith("align");
    expect(mocks.dispatch).not.toHaveBeenCalled();
  });

  it("after Align reads Previous: Channels and Next: Crop, and Previous goes back without recording", () => {
    const aligned = stateWith(M16, { alignedPaths: M16_ALIGNED, completedSteps: { align: true } });
    expect(footerButtons(renderWizard(aligned, "align"))).toEqual([
      { text: "Previous: Channels", disabled: false },
      { text: "Next: Crop", disabled: false },
    ]);
    const footer = footerOfWizard(aligned, "align");
    mocks.dispatch.mockReset();
    click(footer, "wizard-step-prev");
    expect(mocks.setActiveStep).toHaveBeenCalledWith("channels");
    expect(mocks.dispatch).not.toHaveBeenCalled();
  });

  it("disables Next and says why when no later step is ready", () => {
    const unstacked = stateWith({ r: ["/r1.fits", "/r2.fits"], g: ["/g1.fits", "/g2.fits"], b: ["/b1.fits", "/b2.fits"] });
    expect(footerButtons(renderWizard(unstacked, "stack"))).toEqual([
      { text: "Previous: Channels", disabled: false },
      { text: "Next: Align, stack R, G and B first (2+ frames each), or keep one file per channel", disabled: true },
    ]);
  });

  it("keeps Next focusable when it lands on a step whose own Next is not ready: aria-disabled, never disabled, and its click goes nowhere", () => {
    const unstacked = stateWith({ r: ["/r1.fits", "/r2.fits"], g: ["/g1.fits", "/g2.fits"], b: ["/b1.fits", "/b2.fits"] });
    const onChannels = nextButtonTag(renderWizard(unstacked, "channels"));
    expect(onChannels).not.toContain('aria-disabled="true"');
    click(footerOfWizard(unstacked, "channels"), "wizard-step-next");
    expect(mocks.setActiveStep).toHaveBeenCalledWith("stack");

    const onStack = nextButtonTag(renderWizard(unstacked, "stack"));
    expect(onStack).toContain('aria-disabled="true"');
    expect(onStack).not.toMatch(/\sdisabled=""/);
    const footer = footerOfWizard(unstacked, "stack");
    mocks.setActiveStep.mockReset();
    mocks.dispatch.mockReset();
    click(footer, "wizard-step-next");
    expect(mocks.setActiveStep).not.toHaveBeenCalled();
    expect(mocks.dispatch).not.toHaveBeenCalled();
  });
});

describe("Compose wizard blocked step pill", () => {
  it("stays focusable: aria-disabled, never disabled", () => {
    const pill = /<button[^>]*data-testid="step-stack"[^>]*>/.exec(renderWizard(stateWith(M16), "channels"))?.[0];
    expect(pill, "Stack pill").toBeDefined();
    expect(pill).toContain('aria-disabled="true"');
    expect(pill).not.toMatch(/\sdisabled=""/);
  });

  it("shows why the step is blocked under the stepper when it is clicked or focused, and does not open it", () => {
    hooks.mount();
    const state = stateWith(M16);
    let tree = wizardNodes(state, "channels");
    expect(noteOf(tree)).toBeUndefined();

    click(tree, "step-stack");
    tree = wizardNodes(state, "channels");
    const note = noteOf(tree);
    expect(note && textOf(note.props.children)).toBe("Stacking: needs a channel with 2+ frames");
    expect(note?.props.id).toBeTruthy();
    expect(byTestId(tree, "step-stack").props["aria-describedby"]).toBe(note?.props.id);
    expect(mocks.setActiveStep).not.toHaveBeenCalled();
    expect(mocks.dispatch).not.toHaveBeenCalled();

    (byTestId(tree, "step-stack").props.onBlur as () => void)();
    tree = wizardNodes(state, "channels");
    expect(noteOf(tree)).toBeUndefined();
    expect(byTestId(tree, "step-stack").props["aria-describedby"]).toBeUndefined();

    (byTestId(tree, "step-stack").props.onFocus as () => void)();
    tree = wizardNodes(state, "channels");
    expect(noteOf(tree) && textOf(noteOf(tree)!.props.children)).toBe("Stacking: needs a channel with 2+ frames");

    click(tree, "step-align");
    expect(mocks.setActiveStep).toHaveBeenCalledWith("align");
    expect(noteOf(wizardNodes(state, "align"))).toBeUndefined();
  });

  it("shows no note when an open step is focused", () => {
    hooks.mount();
    const state = stateWith(M16);
    (byTestId(wizardNodes(state, "channels"), "step-align").props.onFocus as () => void)?.();
    expect(noteOf(wizardNodes(state, "channels"))).toBeUndefined();
  });
});
