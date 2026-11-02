import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";
import { DEFAULT_QUICK_SLOPE_PARAMS, type QuickSlopeResult, type RampInfo, type RateComparison } from "../../../shared/types/ramp";
import { createFramePublishGate } from "../../../utils/cubeNavigation";

const backend = vi.hoisted(() => ({
  compare: vi.fn(),
  slope: vi.fn(),
  series: vi.fn(),
  cancel: vi.fn(),
}));
const state = vi.hoisted(() => ({ integration: 0, coord: null as { x: number; y: number } | null }));
vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../services/ramp", async (original) => ({
  ...(await original<typeof import("../../../services/ramp")>()),
  compareWithRate: backend.compare,
  runQuickSlope: backend.slope,
  getRampPixelSeries: backend.series,
  getRampPixelFit: () => Promise.reject(new Error("no fit")),
  getRampTables: async () => ({ group: null, int_times: null }),
  cancelQuickSlope: backend.cancel,
}));
vi.mock("../../../services/display", () => ({ getDqFlagTable: async () => ({ flags: [] }) }));
vi.mock("../../../infrastructure/tauri", async (original) => ({
  ...(await original<typeof import("../../../infrastructure/tauri")>()),
  isTauri: () => false,
}));
vi.mock("../../../hooks/useRampStore", () => ({ useRampIntegration: () => state.integration, setIntegration: (n: number) => { state.integration = n; } }));
vi.mock("../../../hooks/useSpectrumStore", () => ({ useSpectrum: () => ({ coord: state.coord }) }));
vi.mock("../../../hooks/useProgress", () => ({ useProgress: () => ({ reset: () => {}, active: false }) }));
vi.mock("../../../context/PreviewContext", () => ({ useDisplayedImage: () => ({ dimensions: [8, 8] }) }));
vi.mock("../../../context/ToolHostContext", () => ({ useToolHost: () => ({ active: true }) }));

import { hooks } from "../../../test/hookRuntime";
import RampPanel from "../RampPanel";
import RampComparePanel from "../RampComparePanel";

type Props = Record<string, unknown>;
type Panel = { type: (props: Props) => ReactNode };

function render(panel: unknown, props: Props): ReactNode {
  hooks.render((panel as Panel).type, props);
  return hooks.render((panel as Panel).type, props) as ReactNode;
}

function find(node: ReactNode, match: (p: Props) => boolean): Props | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = find(child, match);
      if (found) return found;
    }
  } else if (isValidElement<Props>(node)) {
    if (match(node.props)) return node.props;
    return find(node.props.children as ReactNode, match);
  }
  return null;
}

function required(node: ReactNode, match: (p: Props) => boolean): Props {
  const props = find(node, match);
  if (!props) throw new Error("Missing control");
  return props;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
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

const ramp: RampInfo = {
  kind: "jwst_groups", nints: 2, ngroups: 5, nframes: 1, groupgap: 0, tframe_s: 1, tgroup_s: 1, tgroup_source: "tgroup",
  group_times_s: null, readpatt: "NRSRAPID", instrument: "NIRSPEC", detector: "NRS1", exp_type: "NRS_IFU",
  datamodl: "Level1bModel", irs2: null, frame_width: 8, frame_height: 8,
};

function slope(): QuickSlopeResult {
  return {
    fits_path: "slope.fits", png_path: "slope.png", previewUrl: "asset://slope.png", dimensions: [8, 8], integration: 0, nints: 2,
    ngroups: 5, tgroup_s: 1, tgroup_source: "tgroup", detector: "NRS1", readpatt: "NRSRAPID", ref_corrected: false, stripped: false,
    params: DEFAULT_QUICK_SLOPE_PARAMS, band_scales_dn: [3], counts: { saturated: 0, jump_det: 0, do_not_use: 0 },
    rate_sibling: "rate.fits", warnings: [], elapsed_ms: 1,
  };
}

const compareProps = () => ({ qslopePath: "slope.fits", ratePath: "rate.fits", dimensions: [8, 8], onCubeResult: vi.fn(), onInvalidateCubeResult: vi.fn() });
const rampProps = () => ({ filePath: "uncal.fits", ramp, rampSource: "fits", cubeDims: { width: 8, height: 8, frames: 10 }, onCubeResult: vi.fn(), onInvalidateCubeResult: vi.fn() });
const run = (tree: ReactNode, label: string) => (required(tree, (p) => p.label === label).onClick as () => Promise<void>)();
const hasComparison = (tree: ReactNode) => find(tree, (p) => p.children === "Show ratio image") !== null;
const hasSlope = (tree: ReactNode) => find(tree, (p) => p.path === "slope.fits") !== null;

beforeEach(() => {
  hooks.mount();
  state.integration = 0;
  state.coord = null;
  backend.compare.mockReset().mockResolvedValue(comparison());
  backend.slope.mockReset().mockResolvedValue(slope());
  backend.cancel.mockReset().mockResolvedValue(undefined);
  backend.series.mockReset();
});
afterEach(() => hooks.mount());

describe("RATE comparison validity", () => {
  it("exposes only the current completed comparison manifest", async () => {
    backend.compare.mockResolvedValue({ ...comparison(), provenance_path: "C:/out/compare/provenance.json" });
    const props = compareProps();
    await run(render(RampComparePanel, props), "Compare");
    expect(required(render(RampComparePanel, props), (p) => p.path === "C:/out/compare/provenance.json").kind).toBe("manifest");
    expect(find(render(RampComparePanel, { ...props, ratePath: "other.fits" }), (p) => p.path === "C:/out/compare/provenance.json")).toBeNull();
  });

  it("publishes the signed difference and invalidates both products when its reference changes", async () => {
    const props = compareProps();
    await run(render(RampComparePanel, props), "Compare");
    const completed = render(RampComparePanel, props);
    (required(completed, (p) => p.children === "Show signed difference").onClick as () => void)();
    expect(props.onCubeResult).toHaveBeenCalledWith(expect.objectContaining({ fitsPath: "difference.fits", previewUrl: "asset://difference.png", label: "Quick − rate (DN/s)" }));
    expect(find(completed, (p) => p.children === "RMSE")).not.toBeNull();
    render(RampComparePanel, { ...props, ratePath: "new.fits" });
    expect(props.onInvalidateCubeResult).toHaveBeenCalledWith(["ratio.fits", "difference.fits"]);
  });

  const edits: [string, (props: Props, tree: ReactNode) => Props][] = [
    ["RATE file", (props) => ({ ...props, ratePath: "other_rate.fits" })],
    ["Quick Slope product", (props) => ({ ...props, qslopePath: "other_slope.fits" })],
    ["row window", (props, tree) => {
      (required(tree, (p) => p.inputMode === "numeric").onChange as (e: unknown) => void)({ target: { value: "1" } });
      return props;
    }],
  ];

  it.each(edits)("clears completed comparison after changing %s", async (_name, edit) => {
    const props = compareProps();
    await run(render(RampComparePanel, props), "Compare");
    const completed = render(RampComparePanel, props);
    expect(hasComparison(completed)).toBe(true);
    expect(hasComparison(render(RampComparePanel, edit(props, completed)))).toBe(false);
    expect(props.onInvalidateCubeResult).toHaveBeenCalledWith(["ratio.fits", "difference.fits"]);
  });

  it.each(edits)("rejects a late comparison after changing %s", async (_name, edit) => {
    const pending = deferred<RateComparison>();
    backend.compare.mockReturnValue(pending.promise);
    const props = compareProps();
    const running = run(render(RampComparePanel, props), "Compare");
    const next = edit(props, render(RampComparePanel, props));
    render(RampComparePanel, next);
    pending.resolve(comparison());
    await running;
    expect(hasComparison(render(RampComparePanel, next))).toBe(false);
  });

  it("does not let an old failure stop a newer comparison", async () => {
    const old = deferred<RateComparison>();
    const current = deferred<RateComparison>();
    backend.compare.mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise);
    const props = compareProps();
    const first = run(render(RampComparePanel, props), "Compare");
    const next = { ...props, ratePath: "new_rate.fits" };
    const second = run(render(RampComparePanel, next), "Compare");
    old.reject(new Error("old failure"));
    await first;
    const tree = render(RampComparePanel, next);
    expect(required(tree, (p) => p.label === "Compare").running).toBe(true);
    expect(find(tree, (p) => p.message === "old failure")).toBeNull();
    current.resolve(comparison());
    await second;
    expect(hasComparison(render(RampComparePanel, next))).toBe(true);
  });

  it("rejects a response after leaving and returning to the same RATE file", async () => {
    const pending = deferred<RateComparison>();
    backend.compare.mockReturnValue(pending.promise);
    const props = compareProps();
    const running = run(render(RampComparePanel, props), "Compare");
    render(RampComparePanel, { ...props, ratePath: "other_rate.fits" });
    render(RampComparePanel, props);
    pending.resolve(comparison());
    await running;
    expect(hasComparison(render(RampComparePanel, props))).toBe(false);
  });
});

describe("Quick Slope validity", () => {
  it("exposes the quick-slope manifest and clears it with its result", async () => {
    backend.slope.mockResolvedValue({ ...slope(), provenance_path: "C:/out/slope/provenance.json" });
    const props = rampProps();
    await run(render(RampPanel, props), "Run quick slope");
    expect(required(render(RampPanel, props), (p) => p.path === "C:/out/slope/provenance.json").kind).toBe("manifest");
    expect(find(render(RampPanel, { ...props, filePath: "other_uncal.fits" }), (p) => p.path === "C:/out/slope/provenance.json")).toBeNull();
  });

  const edits: [string, (props: Props, tree: ReactNode) => Props][] = [
    ["file", (props) => ({ ...props, filePath: "other_uncal.fits" })],
    ["integration", (props) => { state.integration = 1; return props; }],
    ["parameters", (props, tree) => {
      (required(tree, (p) => p.label === "Jump k").onChange as (v: number) => void)(6);
      return props;
    }],
  ];

  it.each(edits)("clears completed slope after changing %s", async (_name, edit) => {
    const props = rampProps();
    await run(render(RampPanel, props), "Run quick slope");
    const completed = render(RampPanel, props);
    expect(hasSlope(completed)).toBe(true);
    expect(hasSlope(render(RampPanel, edit(props, completed)))).toBe(false);
    expect(props.onInvalidateCubeResult).toHaveBeenCalledWith(["slope.fits"]);
  });

  it.each(edits)("does not publish a late slope after changing %s", async (_name, edit) => {
    const pending = deferred<QuickSlopeResult>();
    backend.slope.mockReturnValue(pending.promise);
    const props = rampProps();
    const running = run(render(RampPanel, props), "Run quick slope");
    const next = edit(props, render(RampPanel, props));
    render(RampPanel, next);
    pending.resolve(slope());
    await running;
    expect(props.onCubeResult).not.toHaveBeenCalled();
    expect(hasSlope(render(RampPanel, next))).toBe(false);
  });

  it("clears a pixel series when its coordinate is removed", async () => {
    state.coord = { x: 2, y: 3 };
    backend.series.mockResolvedValue({ x: 2, y: 3, integration: 0, ngroups: 5, nints: 2, values: [1, 2, 3, 4, 5], group_times_s: [0, 1, 2, 3, 4], unit: "DN" });
    const props = rampProps();
    render(RampPanel, props);
    await vi.waitFor(() => expect(backend.series).toHaveBeenCalledTimes(1));
    expect(find(render(RampPanel, props), (p) => p.yLabel === "DN")).not.toBeNull();
    state.coord = null;
    expect(find(render(RampPanel, props), (p) => p.yLabel === "DN")).toBeNull();
  });

  it("does not overlap backend runs after the integration changes", async () => {
    const pending = deferred<QuickSlopeResult>();
    backend.slope.mockReturnValueOnce(pending.promise).mockResolvedValue(slope());
    const props = rampProps();
    const first = run(render(RampPanel, props), "Run quick slope");
    state.integration = 1;
    await run(render(RampPanel, props), "Run quick slope");
    expect(backend.slope).toHaveBeenCalledTimes(1);
    pending.reject(new Error("old integration failure"));
    await first;
    expect(find(render(RampPanel, props), (p) => p.message === "old integration failure")).toBeNull();
    await run(render(RampPanel, props), "Run quick slope");
    expect(backend.slope).toHaveBeenCalledTimes(2);
    expect(hasSlope(render(RampPanel, props))).toBe(true);
  });

  it("publishes a repeated run without a viewer reset invalidating its new publication ticket", async () => {
    const gate = createFramePublishGate();
    const props = {
      ...rampProps(),
      publishGate: gate,
      onInvalidateCubeResult: vi.fn((paths: string[]) => { if (paths.length) gate.supersede(); }),
    };
    await run(render(RampPanel, props), "Run quick slope");
    expect(props.onCubeResult).toHaveBeenCalledTimes(1);
    await run(render(RampPanel, props), "Run quick slope");
    expect(props.onCubeResult).toHaveBeenCalledTimes(2);
  });

  it("does not publish a slope after unmount", async () => {
    const pending = deferred<QuickSlopeResult>();
    backend.slope.mockReturnValue(pending.promise);
    const props = rampProps();
    const running = run(render(RampPanel, props), "Run quick slope");
    hooks.mount();
    pending.resolve(slope());
    await running;
    expect(props.onCubeResult).not.toHaveBeenCalled();
    expect(backend.cancel).toHaveBeenCalledTimes(1);
  });

  it.each(["another result", "a frame"])("does not replace %s selected while Quick Slope was running", async (selection) => {
    const pending = deferred<QuickSlopeResult>();
    backend.slope.mockReturnValue(pending.promise);
    const gate = createFramePublishGate();
    const props = { ...rampProps(), publishGate: gate };
    const running = run(render(RampPanel, props), "Run quick slope");
    if (selection === "a frame") gate.commit();
    else gate.supersede();
    pending.resolve(slope());
    await running;
    expect(props.onCubeResult).not.toHaveBeenCalled();
    expect(hasSlope(render(RampPanel, props))).toBe(true);
  });

  it("invalidates a displayed comparison together with the old integration", async () => {
    const props = rampProps();
    await run(render(RampPanel, props), "Run quick slope");
    const child = required(render(RampPanel, props), (p) => p.qslopePath === "slope.fits");
    (child.onCubeResult as (result: unknown) => void)({ label: "ratio", previewUrl: "ratio.png", fitsPath: "ratio.fits", dimensions: [8, 8] });
    state.integration = 1;
    render(RampPanel, props);
    expect(props.onInvalidateCubeResult).toHaveBeenCalledWith(["slope.fits", "ratio.fits"]);
  });

  it("rejects a pending pixel series when its coordinate is removed", async () => {
    state.coord = { x: 2, y: 3 };
    const pending = deferred<unknown>();
    backend.series.mockReturnValue(pending.promise);
    const props = rampProps();
    render(RampPanel, props);
    await vi.waitFor(() => expect(backend.series).toHaveBeenCalledTimes(1));
    state.coord = null;
    render(RampPanel, props);
    pending.resolve({ x: 2, y: 3, integration: 0, ngroups: 5, nints: 2, values: [1, 2, 3, 4, 5], group_times_s: [0, 1, 2, 3, 4], unit: "DN" });
    await pending.promise;
    await Promise.resolve();
    expect(find(render(RampPanel, props), (p) => p.yLabel === "DN")).toBeNull();
  });
});
