import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, type FunctionComponent, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { WizardState } from "../../../../utils/wizard";
import type { WizardRunGuard } from "../../../../utils/wizardRunGuard";

const { mocks, captured, stf } = vi.hoisted(() => ({
  mocks: {
    clearComposite: vi.fn(async () => {}),
    setCompositePreviewUrl: vi.fn(),
    blendChannels: vi.fn(),
    maskedStretchComposite: vi.fn(),
    extractBackground: vi.fn(),
    calibrateAndScnr: vi.fn(),
  },
  captured: { clicks: {} as Record<string, () => Promise<unknown>>, guard: null as WizardRunGuard | null },
  stf: { shadow: 0, midtone: 0.5, highlight: 1 },
}));

vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react")>();
  const runtime = await import("../../../../test/hookRuntime");
  const faked = runtime.reactWithFakeHooks(actual) as typeof actual;
  function useReducer(reducer: (state: unknown, action: unknown) => unknown, init: unknown): [unknown, (action: unknown) => void] {
    if (!runtime.hooks.run.active) return (actual.useReducer as unknown as typeof useReducer)(reducer, init);
    const [state, setState] = faked.useState(init);
    return [state, (action) => setState((current: unknown) => reducer(current, action))];
  }
  return { ...faked, useReducer };
});

vi.mock("../../../../context/CompositeContext", () => ({
  useCompositeActions: () => ({
    clearComposite: mocks.clearComposite,
    setCompositePreviewUrl: mocks.setCompositePreviewUrl,
    setCompositeStf: vi.fn(),
    setCompositeStfLinked: vi.fn(),
  }),
  useCompositeStf: () => ({
    compositeStfR: stf, compositeStfG: stf, compositeStfB: stf, compositeStfLinked: true,
    compositeAutoStfR: stf, compositeAutoStfG: stf, compositeAutoStfB: stf,
    wizardStfR: stf, wizardStfG: stf, wizardStfB: stf, wizardStfLinked: true,
  }),
  useCompositePreview: () => ({ compositeVersion: 0 }),
}));

vi.mock("../../../../context/WizardRunGuardContext", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../../context/WizardRunGuardContext")>()),
  useWizardRunGuard: () => captured.guard,
}));

vi.mock("../../../../context/PreviewContext", () => ({
  useRenderActions: () => ({ currentFileKey: () => null, publishProcessed: vi.fn() }),
}));

vi.mock("../../../../services/progress", () => ({ cancelProgress: vi.fn(() => Promise.resolve(true)) }));

vi.mock("../../../../services/compose", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../../services/compose")>()),
  blendChannels: mocks.blendChannels,
  calibrateAndScnr: mocks.calibrateAndScnr,
  computeAutoWb: vi.fn(() => new Promise(() => {})),
}));

vi.mock("../../../../services/processing", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../../services/processing")>()),
  maskedStretchComposite: mocks.maskedStretchComposite,
  extractBackground: mocks.extractBackground,
}));

vi.mock("../../../../infrastructure/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../../infrastructure/tauri")>()),
  getOutputDir: () => Promise.resolve("C:/out"),
  getPreviewUrl: (path: string) => Promise.resolve(`asset://${path}`),
}));

vi.mock("../../../../infrastructure/tauri/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../../infrastructure/tauri/client")>()),
  getPreviewUrl: (path: string) => Promise.resolve(`asset://${path}`),
}));

vi.mock("../../../../hooks/usePointingOverlap", () => ({ usePointingOverlap: () => ({ disjointPairs: [] }) }));

vi.mock("../../StfHistogram", () => ({ default: () => null }));

vi.mock("../../../ui", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../ui")>()),
  RunButton: (props: { label: string; onClick: () => Promise<unknown> }) => {
    captured.clicks[props.label] = props.onClick;
    return null;
  },
}));

import { hooks } from "../../../../test/hookRuntime";
import { elementsOf } from "../../../../test/elementTree";
import { ComposeWizardProvider, useComposeWizardContext } from "../../../../context/ComposeWizardContext";
import { BLEND_PRESETS, INITIAL_STATE } from "../../../../utils/wizard";
import BlendStep from "../BlendStep";
import StretchStep from "../StretchStep";
import BackgroundStep from "../BackgroundStep";
import ColorBalanceStep from "../ColorBalanceStep";

type WizardContextValue = ReturnType<typeof useComposeWizardContext>;

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve: (value: T) => void = () => {};
  const promise = new Promise<T>((r) => { resolve = r; });
  return { promise, resolve };
}

function mountProvider(): WizardContextValue {
  const tree = hooks.render(ComposeWizardProvider, { children: null }) as ReactElement<{ value: WizardContextValue }>;
  const guard = elementsOf<{ value: unknown }>(tree)
    .map((el) => el.props.value)
    .find((v): v is WizardRunGuard => typeof (v as Partial<WizardRunGuard> | null)?.begin === "function");
  captured.guard = guard ?? null;
  return tree.props.value;
}

function binsOf(files: Record<string, string[]>): WizardState["bins"] {
  return INITIAL_STATE.bins.map((b) => ({ ...b, files: files[b.id] ?? [] }));
}

function renderStep<P extends object>(component: FunctionComponent<P>, props: P): string {
  return renderToStaticMarkup(createElement(component, props));
}

beforeEach(() => {
  hooks.mount();
  captured.clicks = {};
  captured.guard = null;
  for (const mock of Object.values(mocks)) mock.mockReset();
  mocks.clearComposite.mockImplementation(async () => {});
});

describe("a wizard run that ends after its inputs changed", () => {
  it("drops a Blend that returns after the channels were reassigned and clears the planes it left", async () => {
    const ctx = mountProvider();
    ctx.dispatch({ type: "SET_BINS", bins: binsOf({ ha: ["/h.fits"], oiii: ["/o.fits"] }) });
    ctx.dispatch({ type: "UPDATE", partial: { levelMatch: false } });
    ctx.dispatch({ type: "SET_BLEND_WEIGHTS", weights: BLEND_PRESETS.hoo.weights, preset: "hoo" });
    const blend = deferred<object>();
    mocks.blendChannels.mockImplementation(() => blend.promise);
    const onCompositeReady = vi.fn(() => {
      ctx.dispatch({ type: "SET_COMPOSITE_READY", ready: true });
      ctx.dispatch({ type: "COMPLETE_STEP", stepId: "blend" });
    });
    renderStep(BlendStep, {
      state: ctx.getState(),
      files: [],
      onLevelMatchChange: vi.fn(),
      onLevelScales: vi.fn(),
      onWeightsChange: vi.fn(),
      onCompositeReady,
      onCompositeOp: vi.fn(),
    });
    const running = captured.clicks["Run Blend"]();
    await vi.waitFor(() => expect(mocks.blendChannels).toHaveBeenCalledTimes(1));
    ctx.dispatch({ type: "SET_BINS", bins: binsOf({ ha: ["/h.fits"], sii: ["/s.fits"] }) });
    blend.resolve({ previewUrl: "asset://out/blend.png", dimensions: [10, 10], channel_count: 2, elapsed_ms: 1 });
    await running;
    expect(onCompositeReady).not.toHaveBeenCalled();
    expect(ctx.getState().compositeReady).toBe(false);
    expect(ctx.getState().completedSteps.blend).toBeUndefined();
    expect(mocks.clearComposite).toHaveBeenCalled();
  });

  it("drops a composite stretch that returns after Reset", async () => {
    const ctx = mountProvider();
    ctx.dispatch({ type: "SET_BINS", bins: binsOf({ ha: ["/h.fits"], oiii: ["/o.fits"] }) });
    ctx.dispatch({ type: "SET_COMPOSITE_READY", ready: true });
    const stretch = deferred<object>();
    mocks.maskedStretchComposite.mockImplementation(() => stretch.promise);
    const onResult = vi.fn(() => ctx.dispatch({ type: "COMPLETE_STEP", stepId: "stretch" }));
    renderStep(StretchStep, {
      state: ctx.getState(),
      onStretchChange: vi.fn(),
      onMaskParams: vi.fn(),
      onResult,
      onChannelOutput: vi.fn(),
      onCompositeOp: vi.fn(),
    });
    const running = captured.clicks["Apply Masked Stretch"]();
    await vi.waitFor(() => expect(mocks.maskedStretchComposite).toHaveBeenCalledTimes(1));
    ctx.dispatch({ type: "RESET" });
    stretch.resolve({ png_path: "C:/out/stretched.png", elapsed_ms: 1 });
    await running;
    expect(onResult).not.toHaveBeenCalled();
    expect(ctx.getState().completedSteps.stretch).toBeUndefined();
    expect(mocks.clearComposite).toHaveBeenCalled();
  });

  it("drops a Background model fitted on the input that Align replaced, and keeps one fitted after it", async () => {
    const ctx = mountProvider();
    ctx.dispatch({ type: "SET_BINS", bins: binsOf({ ha: ["/h.fits"], oiii: ["/o.fits"] }) });
    const first = deferred<object>();
    mocks.extractBackground
      .mockImplementationOnce(() => first.promise)
      .mockResolvedValueOnce({ cache_key: "__wizard_ch_oiii_bg", elapsed_ms: 1 });
    const onBackground = vi.fn((channelId: string, path: string) => ctx.dispatch({ type: "SET_BACKGROUND", channelId, path }));
    renderStep(BackgroundStep, { state: ctx.getState(), onBackground });
    const running = captured.clicks["Extract All"]();
    await vi.waitFor(() => expect(mocks.extractBackground).toHaveBeenCalledTimes(1));
    ctx.dispatch({ type: "SET_ALIGNED", paths: { ha: "__wizard_ch_ha_aligned", oiii: "__wizard_ch_oiii_aligned" } });
    first.resolve({ cache_key: "__wizard_ch_ha_bg", elapsed_ms: 1 });
    await running;
    expect(ctx.getState().backgroundPaths.ha).toBeUndefined();
    expect(ctx.getState().backgroundPaths.oiii).toBe("__wizard_ch_oiii_bg");
  });

  it("drops a colour balance that returns after a new Blend replaced the composite", async () => {
    const ctx = mountProvider();
    ctx.dispatch({ type: "SET_BINS", bins: binsOf({ r: ["/r.fits"], g: ["/g.fits"], b: ["/b.fits"] }) });
    ctx.dispatch({ type: "SET_COMPOSITE_READY", ready: true });
    ctx.dispatch({ type: "SET_WB", mode: "manual", r: 1.2, g: 1, b: 0.9 });
    const balance = deferred<object>();
    mocks.calibrateAndScnr.mockImplementation(() => balance.promise);
    const onResult = vi.fn();
    const onCompositeOp = vi.fn();
    renderStep(ColorBalanceStep, {
      state: ctx.getState(),
      doneFiles: [],
      onWbChange: vi.fn(),
      onSpccFactors: vi.fn(),
      onScnrChange: vi.fn(),
      onResult,
      onCompositeOp,
    });
    const apply = Object.entries(captured.clicks).find(([label]) => label.startsWith("Apply"));
    expect(apply, "apply button").toBeDefined();
    const running = apply![1]();
    await vi.waitFor(() => expect(mocks.calibrateAndScnr).toHaveBeenCalledTimes(1));
    ctx.dispatch({ type: "SET_COMPOSITE_READY", ready: true });
    balance.resolve({ png_path: "C:/out/wb.png", scnr_applied: false, elapsed_ms: 1 });
    await running;
    expect(onResult).not.toHaveBeenCalled();
    expect(onCompositeOp).not.toHaveBeenCalled();
    expect(ctx.getState().compositeReady).toBe(false);
  });
});
