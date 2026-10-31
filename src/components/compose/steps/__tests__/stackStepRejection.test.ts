import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import type { WizardState } from "../../../../utils/wizard";
import type { StackResult } from "../../../../shared/types/stacking";

const wizard = vi.hoisted(() => ({ state: null as WizardState | null }));

vi.mock("react", async (importOriginal) => (await import("../../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../../context/ComposeWizardContext", () => ({
  useComposeWizardContext: () => ({
    stackRun: null,
    setStackRun: vi.fn(),
    finishStackRun: vi.fn(),
    getStackGeneration: () => 0,
    getState: () => wizard.state,
    stackDiscarded: {},
    recordStackOutcome: vi.fn(),
  }),
}));
vi.mock("../../../../services/stacking", () => ({ stackFrames: vi.fn(), drizzleFrames: vi.fn() }));
vi.mock("../../../../services/analysis", () => ({ analyzeSubframes: vi.fn() }));
vi.mock("../../../../services/progress", () => ({ cancelProgress: vi.fn() }));
vi.mock("../../../../infrastructure/tauri", () => ({ getOutputDir: vi.fn() }));
vi.mock("../../../../hooks/useProgress", () => ({
  useProgress: () => ({ active: false, percent: 0, stage: "", current: 0, total: 0, reset: () => {} }),
}));
vi.mock("../../../../hooks/useFileStore", () => ({ resolveEffectivePath: (path: string) => path }));

import { hooks } from "../../../../test/hookRuntime";
import { drizzleFrames, stackFrames } from "../../../../services/stacking";
import { getOutputDir } from "../../../../infrastructure/tauri";
import { releaseStackProgress } from "../../../../utils/stackProgressLock";
import { INITIAL_STATE } from "../../../../utils/wizard";
import { DRIZZLE_FIXED_REJECTION } from "../../../../utils/drizzleParams";
import { NORMALIZATION_OPTIONS } from "../../../../utils/stackingRejection";
import StackStep from "../StackStep";

type AnyProps = Record<string, unknown> & { children?: ReactNode };
type StepProps = { state: WizardState; dispatch: () => void; onStacked: () => void };
const step = StackStep as unknown as (props: StepProps) => ReactNode;

const R = [1, 2, 3, 4, 5, 6, 7].map((n) => `/sx02/r_000${n}.fits`);
const G = [1, 2, 3].map((n) => `/sx02/g_000${n}.fits`);

const STACKED: StackResult = {
  png_path: "/out/stacked_r.png",
  fits_path: "/out/stacked_r.fits",
  dimensions: [8, 8],
  elapsed_ms: 50,
  frame_count: 7,
  rejected_pixels: 1234567,
};

function nodes(node: ReactNode): ReactElement<AnyProps>[] {
  if (Array.isArray(node)) return node.flatMap(nodes);
  if (!isValidElement<AnyProps>(node)) return [];
  return [node, ...nodes(node.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<AnyProps>(node) ? textOf(node.props.children) : "";
}

function stepView(): () => ReactNode {
  const state: WizardState = { ...INITIAL_STATE, bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: b.id === "r" ? R : b.id === "g" ? G : [] })) };
  wizard.state = state;
  return () => hooks.render(step, { state, dispatch: vi.fn(), onStacked: vi.fn() }) as ReactNode;
}

const rejectionControls = (tree: ReactNode) => nodes(tree).find((el) => el.props.settings !== undefined && typeof el.props.onChange === "function");
const change = (tree: ReactNode, update: Record<string, unknown>) => (rejectionControls(tree)?.props.onChange as (c: Record<string, unknown>) => void)(update);
const normalizationSelect = (tree: ReactNode) => nodes(tree).find((el) => el.type === "select" && el.props["data-testid"] === "stack-normalization");
const labelled = (tree: ReactNode, label: string) => nodes(tree).find((el) => el.props.label === label);
const firstStack = (tree: ReactNode) => nodes(tree).find((el) => el.type === "button" && textOf(el.props.children).trim() === "Stack");
const TODAY = { rejection: "sigma_clip", sigmaLow: 3, sigmaHigh: 3, maxIterations: 5, winsorCutoff: 5, percentileLow: 0.2, percentileHigh: 0.1, minmaxLow: 1, minmaxHigh: 1 };

describe("compose Stack step integration settings", () => {
  beforeEach(() => {
    hooks.mount();
    vi.mocked(stackFrames).mockReset();
    vi.mocked(drizzleFrames).mockReset();
    vi.mocked(getOutputDir).mockReset().mockResolvedValue("C:/out");
  });

  afterEach(() => {
    for (const run of ["stack", "drizzle", "compose"] as const) releaseStackProgress(run);
    hooks.mount();
  });

  it("starts from what it ran before: sigma 3.0/3.0, the backend's other defaults and Additive with scaling", async () => {
    vi.mocked(stackFrames).mockResolvedValue(STACKED);
    const view = stepView();
    expect(rejectionControls(view())?.props).toMatchObject({ frameCount: 3, settings: TODAY });
    expect(rejectionControls(view())?.props.fixed).toBeUndefined();
    (firstStack(view())?.props.onClick as () => void)();
    await vi.waitFor(() => expect(vi.mocked(stackFrames)).toHaveBeenCalledTimes(1));
    expect(vi.mocked(stackFrames).mock.calls[0][2]).toMatchObject({ ...TODAY, combine: "mean", normalization: "additive_scaling", align: true });
  });

  it("offers every parameter of the rejection method and the Stack tab's normalization methods, and sends what they show", async () => {
    vi.mocked(stackFrames).mockResolvedValue(STACKED);
    const view = stepView();
    const select = normalizationSelect(view());
    expect(nodes(select?.props.children).map((option) => option.props.value)).toEqual(NORMALIZATION_OPTIONS.map((option) => option.value));
    expect(nodes(select?.props.children).map((option) => textOf(option.props.children))).toEqual(NORMALIZATION_OPTIONS.map((option) => option.label));
    expect(select?.props.value).toBe("additive_scaling");
    const label = nodes(view()).find((el) => el.type === "label" && el.props.htmlFor === select?.props.id);
    expect(textOf(label)).toBe("Normalization");
    change(view(), { rejection: "min_max" });
    change(view(), { minmaxLow: 2 });
    change(view(), { minmaxHigh: 2 });
    (normalizationSelect(view())?.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: "none" } });
    expect(rejectionControls(view())?.props.settings).toMatchObject({ rejection: "min_max", minmaxLow: 2, minmaxHigh: 2 });
    (firstStack(view())?.props.onClick as () => void)();
    await vi.waitFor(() => expect(vi.mocked(stackFrames)).toHaveBeenCalledTimes(1));
    const [paths, , options] = vi.mocked(stackFrames).mock.calls[0];
    expect(paths).toEqual(R);
    expect(options).toMatchObject({ ...TODAY, rejection: "min_max", minmaxLow: 2, minmaxHigh: 2, normalization: "none", combine: "mean" });
  });

  it("formats the rejected pixel count of a stack", async () => {
    vi.mocked(stackFrames).mockResolvedValue(STACKED);
    const view = stepView();
    (firstStack(view())?.props.onClick as () => void)();
    await vi.waitFor(() => expect(textOf(view())).toContain("7 frames, 1,234,567 rejected px, 50ms"));
  });

  it("in drizzle keeps its row, shows what the drizzle engine fixes read-only and sends the method and sigmas", async () => {
    vi.mocked(drizzleFrames).mockResolvedValue({ ...STACKED, scale: 2 });
    const view = stepView();
    (labelled(view(), "Drizzle")?.props.onChange as (on: boolean) => void)(true);
    expect(labelled(view(), "Scale")).toBeDefined();
    expect(labelled(view(), "Pixfrac")).toBeDefined();
    expect(nodes(view()).find((el) => el.props["data-testid"] === "stack-drizzle-kernel")).toBeDefined();
    expect(normalizationSelect(view())).toBeUndefined();
    expect(rejectionControls(view())?.props).toMatchObject({ fixed: DRIZZLE_FIXED_REJECTION, frameCount: 3 });
    change(view(), { rejection: "linear_fit_clip" });
    change(view(), { sigmaLow: 2 });
    (firstStack(view())?.props.onClick as () => void)();
    await vi.waitFor(() => expect(vi.mocked(drizzleFrames)).toHaveBeenCalledTimes(1));
    const options = vi.mocked(drizzleFrames).mock.calls[0][2];
    expect(options).toMatchObject({ scale: 2, pixfrac: 0.7, kernel: "square", align: true, rejection: "linear_fit_clip", sigmaLow: 2, sigmaHigh: 3 });
    for (const key of ["maxIterations", "winsorCutoff", "percentileLow", "percentileHigh", "minmaxLow", "minmaxHigh", "normalization"]) {
      expect(Object.keys(options ?? {})).not.toContain(key);
    }
  });
});
