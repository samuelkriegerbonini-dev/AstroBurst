import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";
import { DEFAULT_QUICK_SLOPE_PARAMS, type QuickSlopeResult, type RampInfo } from "../../../shared/types/ramp";
import { createFramePublishGate } from "../../../utils/cubeNavigation";

const backend = vi.hoisted(() => ({ slope: vi.fn(), cancel: vi.fn(), pick: vi.fn() }));
const viewer = vi.hoisted(() => ({ shown: { dimensions: [8, 8] } as Record<string, unknown> }));
const laterParam = vi.hoisted(() => ({ value: null as string | null }));
vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../utils/rampLabels", async (original) => {
  const actual = await original<typeof import("../../../utils/rampLabels")>();
  return {
    ...actual,
    paramsFromDraft: (draft: Parameters<typeof actual.paramsFromDraft>[0]) =>
      laterParam.value === null ? actual.paramsFromDraft(draft) : { ...actual.paramsFromDraft(draft), later_option: laterParam.value },
  };
});
vi.mock("../../../services/ramp", async (original) => ({
  ...(await original<typeof import("../../../services/ramp")>()),
  runQuickSlope: backend.slope,
  getRampPixelSeries: () => new Promise(() => {}),
  getRampPixelFit: () => new Promise(() => {}),
  getRampTables: async () => ({ group: null, int_times: null }),
  cancelQuickSlope: backend.cancel,
}));
vi.mock("../../../services/display", () => ({ getDqFlagTable: async () => ({ flags: [] }) }));
vi.mock("../../../infrastructure/tauri", async (original) => ({
  ...(await original<typeof import("../../../infrastructure/tauri")>()),
  isTauri: () => true,
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: backend.pick }));
vi.mock("../../../hooks/useRampStore", () => ({ useRampIntegration: () => 0, setIntegration: () => {} }));
vi.mock("../../../hooks/useSpectrumStore", () => ({ useSpectrum: () => ({ coord: null }) }));
vi.mock("../../../hooks/useProgress", () => ({ useProgress: () => ({ reset: () => {}, active: false }) }));
vi.mock("../../../context/PreviewContext", () => ({ useDisplayedImage: () => viewer.shown }));
vi.mock("../../../context/ToolHostContext", () => ({ useToolHost: () => ({ active: true }) }));

import { hooks } from "../../../test/hookRuntime";
import RampPanel from "../RampPanel";
import RampComparePanel, { DIFFERENCE_LABEL, RATIO_LABEL } from "../RampComparePanel";
import StaleResultNote, { ReopenedProductNote } from "../StaleResultNote";

type Props = Record<string, unknown>;
type Panel = { type: (props: Props) => ReactNode };

function render(props: Props): ReactNode {
  hooks.render((RampPanel as unknown as Panel).type, props);
  return hooks.render((RampPanel as unknown as Panel).type, props) as ReactNode;
}

function all(node: ReactNode, match: (p: Props, type: unknown) => boolean): Props[] {
  if (Array.isArray(node)) return node.flatMap((child) => all(child, match));
  if (!isValidElement<Props>(node)) return [];
  return [...(match(node.props, node.type) ? [node.props] : []), ...all(node.props.children as ReactNode, match)];
}

const ramp: RampInfo = {
  kind: "jwst_groups", nints: 1, ngroups: 5, nframes: 1, groupgap: 0, tframe_s: 1, tgroup_s: 1, tgroup_source: "tgroup",
  group_times_s: null, readpatt: "NRSRAPID", instrument: "NIRSPEC", detector: "NRS1", exp_type: "NRS_IFU",
  datamodl: "Level1bModel", irs2: null, frame_width: 8, frame_height: 8,
};

function slope(extra: Partial<QuickSlopeResult> = {}): QuickSlopeResult {
  return {
    fits_path: "slope.fits", png_path: "slope.png", previewUrl: "asset://slope.png", dimensions: [8, 8], integration: 0, nints: 1,
    ngroups: 5, tgroup_s: 1, tgroup_source: "tgroup", detector: "NRS1", readpatt: "NRSRAPID", ref_corrected: false, stripped: false,
    params: DEFAULT_QUICK_SLOPE_PARAMS, band_scales_dn: [3], counts: { saturated: 0, jump_det: 0, do_not_use: 0 },
    rate_sibling: null, warnings: [], elapsed_ms: 1, ...extra,
  };
}

const rampProps = () => ({ filePath: "uncal.fits", ramp, rampSource: "fits", cubeDims: { width: 8, height: 8, frames: 5 }, onCubeResult: vi.fn(), onInvalidateCubeResult: vi.fn() });
const runSlope = (tree: ReactNode) => (all(tree, (p) => p.label === "Run quick slope")[0].onClick as () => Promise<void>)();
const pickRate = (tree: ReactNode) =>
  (all(tree, (p) => Array.isArray(p.children) && p.children.includes("Pick the official rate file"))[0].onClick as () => Promise<void>)();
const setJumpK = (tree: ReactNode, value: number) => (all(tree, (p) => p.label === "Jump k")[0].onChange as (v: number) => void)(value);
const wholeBand = (tree: ReactNode, checked: boolean) =>
  (all(tree, (p) => p.type === "checkbox")[0].onChange as (e: unknown) => void)({ target: { checked } });
const comparePanel = (tree: ReactNode) => all(tree, (_p, type) => type === RampComparePanel)[0] ?? null;
const shownSlope = (tree: ReactNode) => all(tree, (p) => typeof p.path === "string" && p.path.endsWith(".fits") && p.kind === undefined)[0]?.path ?? null;
const isStale = (tree: ReactNode) => all(tree, (_p, type) => type === StaleResultNote).length > 0;
const invalidatedPaths = (fn: ReturnType<typeof vi.fn>) => fn.mock.calls.flatMap(([paths]) => paths as string[]);

beforeEach(() => {
  hooks.mount();
  laterParam.value = null;
  viewer.shown = { dimensions: [8, 8] };
  backend.slope.mockReset().mockResolvedValue(slope());
  backend.cancel.mockReset().mockResolvedValue(undefined);
  backend.pick.mockReset().mockResolvedValue("picked_rate.fits");
});
afterEach(() => hooks.mount());

describe("Quick slope after a parameter edit", () => {
  it("keeps the result, its viewer product and the picked rate file, marked stale", async () => {
    const props = rampProps();
    await runSlope(render(props));
    await pickRate(render(props));
    expect(comparePanel(render(props))?.ratePath).toBe("picked_rate.fits");
    setJumpK(render(props), 6);
    const edited = render(props);
    expect(shownSlope(edited)).toBe("slope.fits");
    expect(isStale(edited)).toBe(true);
    expect(comparePanel(edited)?.ratePath).toBe("picked_rate.fits");
    expect(comparePanel(edited)?.slopeStale).toBe(true);
    expect(invalidatedPaths(props.onInvalidateCubeResult)).not.toContain("slope.fits");
  });

  it("drops the stale note when the parameters return to the values of the result", async () => {
    const props = rampProps();
    await runSlope(render(props));
    setJumpK(render(props), 6);
    expect(isStale(render(props))).toBe(true);
    setJumpK(render(props), DEFAULT_QUICK_SLOPE_PARAMS.jump_k);
    const restored = render(props);
    expect(isStale(restored)).toBe(false);
    expect(comparePanel(restored)).toBeNull();
  });

  it("compares every parameter field, so a field added after the first six also marks the result stale", async () => {
    laterParam.value = "first";
    const props = rampProps();
    await runSlope(render(props));
    laterParam.value = "second";
    setJumpK(render(props), 6);
    setJumpK(render(props), DEFAULT_QUICK_SLOPE_PARAMS.jump_k);
    expect(isStale(render(props))).toBe(true);
  });

  it("tells an emptied reference window apart from the whole band the result used", async () => {
    const props = rampProps();
    const windowText = String(DEFAULT_QUICK_SLOPE_PARAMS.ref_window_rows);
    (all(render(props), (p) => p.inputMode === "numeric" && p.value === windowText)[0].onChange as (e: unknown) => void)({ target: { value: "" } });
    wholeBand(render(props), true);
    await runSlope(render(props));
    expect(backend.slope).toHaveBeenLastCalledWith("uncal.fits", undefined, 0, { ...DEFAULT_QUICK_SLOPE_PARAMS, ref_window_rows: null });
    expect(isStale(render(props))).toBe(false);
    wholeBand(render(props), false);
    expect(isStale(render(props))).toBe(true);
  });

  it("replaces the stale result on the next run, keeps the picked rate file and invalidates the outputs it replaced", async () => {
    const gate = createFramePublishGate();
    const props = { ...rampProps(), publishGate: gate, onInvalidateCubeResult: vi.fn((paths: string[]) => { if (paths.length) gate.supersede(); }) };
    await runSlope(render(props));
    await pickRate(render(props));
    (comparePanel(render(props))?.onCubeResult as (result: unknown) => void)({ label: "Quick / rate ratio", previewUrl: "ratio.png", fitsPath: "ratio.fits", dimensions: [8, 8] });
    setJumpK(render(props), 6);
    backend.slope.mockResolvedValue(slope({ fits_path: "slope2.fits", previewUrl: "asset://slope2.png" }));
    await runSlope(render(props));
    expect(backend.slope).toHaveBeenLastCalledWith("uncal.fits", undefined, 0, { ...DEFAULT_QUICK_SLOPE_PARAMS, jump_k: 6 });
    expect(props.onCubeResult).toHaveBeenLastCalledWith(expect.objectContaining({ fitsPath: "slope2.fits" }));
    expect(props.onInvalidateCubeResult).toHaveBeenCalledWith(["slope.fits", "ratio.fits"]);
    const replaced = render(props);
    expect(shownSlope(replaced)).toBe("slope2.fits");
    expect(isStale(replaced)).toBe(false);
    expect(comparePanel(replaced)?.ratePath).toBe("picked_rate.fits");
    expect(comparePanel(replaced)?.slopeStale).toBe(false);
  });
});

describe("RAMP product of an earlier run on the viewer", () => {
  const OLD_PATH = "C:/out/qslope-1/slope.fits";
  const product = (label: string, path: string | null = OLD_PATH) => ({
    path,
    dimensions: [8, 8],
    isProcessed: true,
    label,
    previewOnly: path === null,
    previewUrl: "asset://old.png",
  });
  const reopened = (tree: ReactNode) => all(tree, (_p, type) => type === ReopenedProductNote)[0] ?? null;

  it.each(["Quick slope (DN/s)", RATIO_LABEL, DIFFERENCE_LABEL])("names %s when the tool has no result and offers Show cube", (label) => {
    viewer.shown = product(label);
    const props = rampProps();
    const note = reopened(render(props));
    expect(note?.label).toBe(label);
    (note?.onShowCube as () => void)();
    expect(props.onInvalidateCubeResult).toHaveBeenLastCalledWith([OLD_PATH]);
  });

  it("hands focus to the Quick slope heading when Show cube removes the note", () => {
    viewer.shown = product("Quick slope (DN/s)");
    const tree = render(rampProps());
    const heading = all(tree, (p) => p.children === "Quick slope")[0];
    expect(heading.tabIndex).toBe(-1);
    expect(heading.ref).toEqual({ current: null });
    expect(reopened(tree)?.focusAfterShowCube).toBe(heading.ref);
  });

  it("does not name the slope of the result the tool shows", async () => {
    const props = rampProps();
    await runSlope(render(props));
    viewer.shown = product("Quick slope (DN/s)", "slope.fits");
    expect(reopened(render(props))).toBeNull();
  });

  it.each([
    ["a group preview", product("group 3 of 5")],
    ["a collapse of the Spectroscopy section", product("Collapse mean")],
    ["the input file", { path: "uncal.fits", dimensions: [8, 8], isProcessed: false, label: null, previewOnly: false, previewUrl: null }],
    ["a preview without a FITS file", product("Quick slope (DN/s)", null)],
  ])("does not name %s", (_name, shown) => {
    viewer.shown = shown;
    expect(reopened(render(rampProps()))).toBeNull();
  });

  it("hides the note while a quick slope run that will replace the product is in progress", async () => {
    viewer.shown = product("Quick slope (DN/s)");
    let finish!: (value: QuickSlopeResult) => void;
    backend.slope.mockReturnValue(new Promise<QuickSlopeResult>((resolve) => { finish = resolve; }));
    const props = rampProps();
    expect(reopened(render(props))).not.toBeNull();
    const running = runSlope(render(props));
    expect(reopened(render(props))).toBeNull();
    finish(slope());
    await running;
  });
});
