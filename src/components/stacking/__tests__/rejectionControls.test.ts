import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import type { RejectionMethod } from "../../../shared/types/stacking";

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

import { hooks } from "../../../test/hookRuntime";
import { DRIZZLE_FIXED_REJECTION } from "../../../utils/drizzleParams";
import {
  boundRejectionSettings,
  DEFAULT_STACK_SETTINGS,
  pipelineRejectionFields,
  REJECTION_OPTIONS,
  type RejectionSettings,
} from "../../../utils/stackingRejection";
import RejectionControls from "../RejectionControls";

type AnyProps = Record<string, unknown> & { children?: ReactNode };
type ControlProps = Parameters<typeof RejectionControls>[0];

const controls = RejectionControls as unknown as (props: ControlProps) => ReactNode;

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

const STACK_TAB: RejectionSettings = {
  rejection: "sigma_clip",
  sigmaLow: 4,
  sigmaHigh: 3,
  maxIterations: 5,
  winsorCutoff: 5,
  percentileLow: 0.2,
  percentileHigh: 0.1,
  minmaxLow: 1,
  minmaxHigh: 1,
};

function rendered(settings: Partial<RejectionSettings>, extra: Partial<ControlProps> = {}) {
  const onChange = vi.fn();
  const tree = hooks.render(controls, { settings: { ...STACK_TAB, ...settings }, onChange, frameCount: 7, accent: "amber", ...extra }) as ReactNode;
  return { tree, onChange };
}

const isSlider = (el: ReactElement<AnyProps>) => typeof el.props.label === "string" && "step" in el.props && "min" in el.props;
const sliderOf = (tree: ReactNode, label: string) => nodes(tree).find((el) => isSlider(el) && el.props.label === label);
const sliderLabels = (tree: ReactNode) => nodes(tree).filter(isSlider).map((el) => el.props.label);
const hintOf = (tree: ReactNode) => nodes(tree).find((el) => el.props["data-testid"] === "rejection-hint");
const fixedRows = (tree: ReactNode) =>
  nodes(tree)
    .filter((el) => el.props["data-testid"] === "rejection-fixed")
    .map((row) => nodes(row.props.children).filter((el) => el.type === "span").map((span) => textOf(span.props.children)));
const setSlider = (tree: ReactNode, label: string, value: number) => (sliderOf(tree, label)?.props.onChange as (v: number) => void)(value);

describe("RejectionControls", () => {
  beforeEach(() => hooks.mount());
  afterEach(() => hooks.mount());

  it("Min/max shows its counts and the hint uses them", () => {
    const { tree } = rendered({ rejection: "min_max", minmaxLow: 2, minmaxHigh: 2 }, { frameCount: 4 });
    expect(sliderLabels(tree)).toEqual(["Reject Low", "Reject High"]);
    expect([sliderOf(tree, "Reject Low")?.props.value, sliderOf(tree, "Reject High")?.props.value]).toEqual([2, 2]);
    expect(textOf(hintOf(tree))).toBe("Min/max needs more than 4 frames to drop 2 low and 2 high (have 4).");
    expect(String(hintOf(tree)?.props.className)).toContain("amber");
    hooks.mount();
    const enough = rendered({ rejection: "min_max", minmaxLow: 2, minmaxHigh: 2 }, { frameCount: 5 }).tree;
    expect(textOf(hintOf(enough))).toBe("Drops the 2 lowest and 2 highest values at every pixel.");
  });

  it.each<[RejectionMethod, string[]]>([
    ["none", []],
    ["sigma_clip", ["Sigma Low", "Sigma High", "Max Iterations"]],
    ["winsorized_sigma_clip", ["Sigma Low", "Sigma High", "Max Iterations", "Winsorization Cutoff"]],
    ["linear_fit_clip", ["Sigma Low", "Sigma High", "Max Iterations"]],
    ["percentile_clip", ["Percentile Low", "Percentile High"]],
    ["min_max", ["Reject Low", "Reject High"]],
  ])("shows the parameters %s uses and no other", (rejection, labels) => {
    expect(sliderLabels(rendered({ rejection }).tree)).toEqual(labels);
  });

  it("bounds its ranges: sigmas 1-6, at least one iteration, no negative percentile, whole counts", () => {
    const sigma = rendered({ rejection: "winsorized_sigma_clip" });
    const ranges = (tree: ReactNode, labels: string[]) => labels.map((label) => [sliderOf(tree, label)?.props.min, sliderOf(tree, label)?.props.max]);
    expect(ranges(sigma.tree, ["Sigma Low", "Sigma High", "Max Iterations", "Winsorization Cutoff"])).toEqual([[1, 6], [1, 6], [1, 20], [1, 10]]);
    setSlider(sigma.tree, "Max Iterations", 0);
    setSlider(sigma.tree, "Sigma High", 9);
    setSlider(sigma.tree, "Sigma Low", Number.POSITIVE_INFINITY);
    setSlider(sigma.tree, "Winsorization Cutoff", 0.2);
    expect(sigma.onChange.mock.calls).toEqual([[{ maxIterations: 1 }], [{ sigmaHigh: 6 }], [{ sigmaLow: 6 }], [{ winsorCutoff: 1 }]]);
    hooks.mount();
    const percentile = rendered({ rejection: "percentile_clip" });
    expect(ranges(percentile.tree, ["Percentile Low", "Percentile High"])).toEqual([[0, 1], [0, 1]]);
    setSlider(percentile.tree, "Percentile Low", -0.2);
    setSlider(percentile.tree, "Percentile High", 0.35);
    expect(percentile.onChange.mock.calls).toEqual([[{ percentileLow: 0 }], [{ percentileHigh: 0.35 }]]);
    hooks.mount();
    const counts = rendered({ rejection: "min_max" });
    expect(ranges(counts.tree, ["Reject Low", "Reject High"])).toEqual([[0, 10], [0, 10]]);
    setSlider(counts.tree, "Reject Low", 2.6);
    setSlider(counts.tree, "Reject High", -1);
    expect(counts.onChange.mock.calls).toEqual([[{ minmaxLow: 3 }], [{ minmaxHigh: 0 }]]);
  });

  it("shows a value it is given outside its range at the bound that runs", () => {
    const { tree } = rendered({ rejection: "sigma_clip", maxIterations: 0, sigmaLow: 7.5 });
    expect(sliderOf(tree, "Max Iterations")?.props.value).toBe(1);
    expect(sliderOf(tree, "Sigma Low")?.props.value).toBe(6);
  });

  it("shows the parameters an engine fixes read-only, with the values it runs, and the hint uses them", () => {
    const sigma = rendered({ rejection: "sigma_clip", maxIterations: 12 }, { fixed: DRIZZLE_FIXED_REJECTION }).tree;
    expect(sliderLabels(sigma)).toEqual(["Sigma Low", "Sigma High"]);
    expect(fixedRows(sigma)).toEqual([["Max Iterations", "5, fixed in drizzle"]]);
    hooks.mount();
    const winsorized = rendered({ rejection: "winsorized_sigma_clip", winsorCutoff: 2.5 }, { fixed: DRIZZLE_FIXED_REJECTION }).tree;
    expect(fixedRows(winsorized)).toEqual([["Max Iterations", "5, fixed in drizzle"], ["Winsorization Cutoff", "5.0σ, fixed in drizzle"]]);
    hooks.mount();
    const percentile = rendered({ rejection: "percentile_clip", percentileLow: 0.4 }, { fixed: DRIZZLE_FIXED_REJECTION }).tree;
    expect(sliderLabels(percentile)).toEqual([]);
    expect(fixedRows(percentile)).toEqual([["Percentile Low", "0.20, fixed in drizzle"], ["Percentile High", "0.10, fixed in drizzle"]]);
    hooks.mount();
    const counts = rendered({ rejection: "min_max", minmaxLow: 3, minmaxHigh: 3 }, { fixed: DRIZZLE_FIXED_REJECTION, frameCount: 2 }).tree;
    expect(sliderLabels(counts)).toEqual([]);
    expect(fixedRows(counts)).toEqual([["Reject Low", "1, fixed in drizzle"], ["Reject High", "1, fixed in drizzle"]]);
    expect(textOf(hintOf(counts))).toBe("Min/max needs more than 2 frames to drop 1 low and 1 high (have 2).");
  });

  it("fixes in drizzle the values DrizzleConfig::rejection_params runs", () => {
    expect(DRIZZLE_FIXED_REJECTION).toEqual({
      note: "fixed in drizzle",
      values: { maxIterations: 5, winsorCutoff: 5, percentileLow: 0.2, percentileHigh: 0.1, minmaxLow: 1, minmaxHigh: 1 },
    });
  });

  it("says nothing about the frame count without one", () => {
    expect(hintOf(rendered({}, { frameCount: null }).tree)).toBeUndefined();
  });

  it("picks the method in a select named after its label, and disables every control while a run holds them", () => {
    const { tree, onChange } = rendered({}, { label: "Pixel rejection" });
    const select = nodes(tree).find((el) => el.type === "select");
    const label = nodes(tree).find((el) => el.type === "label");
    expect(textOf(label)).toBe("Pixel rejection");
    expect(label?.props.htmlFor).toBe(select?.props.id);
    expect(nodes(select?.props.children).map((option) => option.props.value)).toEqual(REJECTION_OPTIONS.map((option) => option.value));
    (select?.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: "min_max" } });
    expect(onChange.mock.calls).toEqual([[{ rejection: "min_max" }]]);
    hooks.mount();
    const busy = rendered({}, { disabled: true }).tree;
    expect(nodes(busy).find((el) => el.type === "select")?.props.disabled).toBe(true);
    expect(nodes(busy).filter(isSlider).map((el) => el.props.disabled)).toEqual([true, true, true]);
  });
});

describe("rejection settings sent by a run", () => {
  it("keep the nine Stack tab defaults and nothing else", () => {
    expect(boundRejectionSettings(DEFAULT_STACK_SETTINGS)).toEqual(STACK_TAB);
  });

  it("map to the Pipeline request's snake_case keys, with whole numbers where the backend takes a count", () => {
    expect(pipelineRejectionFields({ ...STACK_TAB, rejection: "min_max", maxIterations: 7.4, minmaxLow: 2, minmaxHigh: 1.6, percentileLow: -0.1 })).toEqual({
      rejection: "min_max",
      sigma_low: 4,
      sigma_high: 3,
      max_iterations: 7,
      winsor_cutoff: 5,
      percentile_low: 0,
      percentile_high: 0.1,
      minmax_low: 2,
      minmax_high: 2,
    });
  });
});
