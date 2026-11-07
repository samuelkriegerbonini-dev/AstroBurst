import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";
import type { RateComparison } from "../../../shared/types/ramp";

const backend = vi.hoisted(() => ({ compare: vi.fn() }));
vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../services/ramp", async (original) => ({
  ...(await original<typeof import("../../../services/ramp")>()),
  compareWithRate: backend.compare,
}));

import { hooks } from "../../../test/hookRuntime";
import RampComparePanel from "../RampComparePanel";
import StaleResultNote from "../StaleResultNote";

type Props = Record<string, unknown>;
type Panel = { type: (props: Props) => ReactNode };

function render(props: Props): ReactNode {
  hooks.render((RampComparePanel as unknown as Panel).type, props);
  return hooks.render((RampComparePanel as unknown as Panel).type, props) as ReactNode;
}

function all(node: ReactNode, match: (p: Props, type: unknown) => boolean): Props[] {
  if (Array.isArray(node)) return node.flatMap((child) => all(child, match));
  if (!isValidElement<Props>(node)) return [];
  return [...(match(node.props, node.type) ? [node.props] : []), ...all(node.props.children as ReactNode, match)];
}

function comparison(): RateComparison {
  const confusion = { tp: 0, fp: 0, fn_: 0, recall: null, precision: null };
  return {
    qslope_path: "slope.fits", rate_path: "rate.fits", detector: "NRS1", good_pixels: 64, quick_flagged_in_good: 0,
    bins: [], amp_bins: [], zero_shift: { best_dy: 0, corr_at_zero: 1, corr_best: 1, second_dy: 1, second_corr: 0, passed: true },
    jump: confusion, saturated: confusion, saturated_any_group: confusion, saturated_rule: "all_groups",
    rel_hist: { edges: [], counts: [] }, delta_hist: { edges: [], counts: [] }, science_rows: null,
    ratio_png_path: "ratio.png", ratio_fits_path: "ratio.fits", ratioPreviewUrl: "asset://ratio.png", elapsed_ms: 1,
    difference_png_path: "difference.png", difference_fits_path: "difference.fits", differencePreviewUrl: "asset://difference.png",
    quick_exclude_bits: 1, official_exclude_bits: 4294967295, ratio_min_abs_rate: 0.3, warnings: [],
    difference_metrics: { n: 64, bias: -0.1, mae: 0.2, rmse: 0.3, median_delta: -0.05, mad_sigma: 0.15 },
  };
}

const compareProps = () => ({ qslopePath: "slope.fits", ratePath: "rate.fits", dimensions: [8, 8], onCubeResult: vi.fn(), onInvalidateCubeResult: vi.fn() });
const compareButton = (tree: ReactNode) => all(tree, (p) => p.label === "Compare")[0];
const compare = (tree: ReactNode) => (compareButton(tree).onClick as () => Promise<void>)();
const typeRow = (tree: ReactNode, index: 0 | 1, text: string) =>
  (all(tree, (p) => p.inputMode === "numeric")[index].onChange as (e: unknown) => void)({ target: { value: text } });
const hasComparison = (tree: ReactNode) => all(tree, (p) => p.children === "Show ratio image").length > 0;
const isStale = (tree: ReactNode) => all(tree, (_p, type) => type === StaleResultNote).length > 0;
const compareReasons = (tree: ReactNode) =>
  String(compareButton(tree).describedBy ?? "")
    .split(" ")
    .filter((id) => id !== "")
    .map((id) => all(tree, (p) => p.id === id)[0]?.children);

beforeEach(() => {
  hooks.mount();
  backend.compare.mockReset().mockResolvedValue(comparison());
});
afterEach(() => hooks.mount());

describe("RATE comparison after a row-window edit", () => {
  it("keeps the comparison and its published images, marked stale", async () => {
    const props = compareProps();
    await compare(render(props));
    const completed = render(props);
    expect(hasComparison(completed)).toBe(true);
    expect(isStale(completed)).toBe(false);
    typeRow(completed, 0, "1");
    const edited = render(props);
    expect(hasComparison(edited)).toBe(true);
    expect(isStale(edited)).toBe(true);
    expect(props.onInvalidateCubeResult).not.toHaveBeenCalled();
  });

  it("drops the stale note when the rows return to the window the comparison used", async () => {
    const props = compareProps();
    typeRow(render(props), 0, "10");
    typeRow(render(props), 1, "20");
    await compare(render(props));
    expect(backend.compare).toHaveBeenLastCalledWith("slope.fits", "rate.fits", undefined, [10, 20]);
    expect(isStale(render(props))).toBe(false);
    typeRow(render(props), 1, "21");
    expect(isStale(render(props))).toBe(true);
    typeRow(render(props), 1, "20");
    expect(isStale(render(props))).toBe(false);
  });

  it("replaces the stale comparison on the next Compare and invalidates its images", async () => {
    const props = compareProps();
    await compare(render(props));
    typeRow(render(props), 0, "1");
    typeRow(render(props), 1, "5");
    expect(isStale(render(props))).toBe(true);
    await compare(render(props));
    expect(backend.compare).toHaveBeenLastCalledWith("slope.fits", "rate.fits", undefined, [1, 5]);
    expect(props.onInvalidateCubeResult).toHaveBeenCalledExactlyOnceWith(["ratio.fits", "difference.fits"]);
    const replaced = render(props);
    expect(hasComparison(replaced)).toBe(true);
    expect(isStale(replaced)).toBe(false);
  });

  it("does not compare a quick slope that is itself stale", async () => {
    const props = { ...compareProps(), slopeStale: true };
    const tree = render(props);
    expect(compareButton(tree).disabled).toBe(true);
    await compare(tree);
    expect(backend.compare).not.toHaveBeenCalled();
  });
});

describe("RATE comparison after a quick slope parameter edit", () => {
  it("keeps the verdict and metrics, marked stale, while the compared quick slope is stale", async () => {
    const props = compareProps();
    await compare(render(props));
    const slopeEdited = render({ ...props, slopeStale: true });
    expect(hasComparison(slopeEdited)).toBe(true);
    expect(isStale(slopeEdited)).toBe(true);
    expect(props.onInvalidateCubeResult).not.toHaveBeenCalled();
    expect(isStale(render({ ...props, slopeStale: false }))).toBe(false);
  });

  it("tells why Compare is disabled while the quick slope is stale", () => {
    const tree = render({ ...compareProps(), slopeStale: true });
    expect(compareButton(tree).disabled).toBe(true);
    expect(compareReasons(tree)).toEqual(["Run the quick slope again first."]);
  });

  it("tells why Compare is disabled while the row window is invalid", () => {
    const props = compareProps();
    typeRow(render(props), 0, "1");
    const tree = render(props);
    expect(compareButton(tree).disabled).toBe(true);
    expect(compareReasons(tree)).toEqual(["enter both rows of the uncal window or neither"]);
  });

  it("gives no reason while Compare is enabled", () => {
    expect(compareButton(render(compareProps())).describedBy).toBeUndefined();
  });
});
