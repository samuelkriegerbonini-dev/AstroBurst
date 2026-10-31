import { describe, it, expect, vi, beforeEach } from "vitest";
import { createElement, isValidElement, type FunctionComponent, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { exportMappingBanner, INITIAL_STATE, type WizardState } from "../../../utils/wizard";
import type { AlignedChannel, AlignPlan, AlignResult } from "../../../shared/types/compose";
import type { WizardAlignRun } from "../../../context/ComposeWizardContext";
import { STACK_INPUTS_CHANGED } from "../../../utils/stackRun";

const { current, captured, mocks } = vi.hoisted(() => ({
  current: { state: null as unknown as WizardState, alignRun: null as unknown, stackGeneration: 0 },
  captured: { clicks: {} as Record<string, () => unknown>, previewRun: undefined as unknown },
  mocks: {
    alignChannels: vi.fn(),
    alignPlan: vi.fn(),
    cropChannels: vi.fn(),
    dispatch: vi.fn(),
    stackFrames: vi.fn(),
    recordStackOutcome: vi.fn(),
  },
}));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

vi.mock("../../../context/ComposeWizardContext", () => ({
  useComposeWizardContext: () => ({
    state: current.state,
    dispatch: mocks.dispatch,
    getState: () => current.state,
    alignRun: current.alignRun,
    startAlignRun: () => {},
    finishAlignRun: () => true,
    stackRun: null,
    setStackRun: () => {},
    finishStackRun: () => {},
    getStackGeneration: () => current.stackGeneration,
    stackDiscarded: {},
    recordStackOutcome: mocks.recordStackOutcome,
  }),
}));

vi.mock("../../../services/stacking", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/stacking")>()),
  stackFrames: mocks.stackFrames,
}));

vi.mock("../../../context/CompositeContext", () => ({
  useCompositeStf: () => ({ compositeStfR: null, compositeStfG: null, compositeStfB: null, compositeStfLinked: true }),
}));

vi.mock("../../ui", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../ui")>()),
  RunButton: (props: { label: string; onClick: () => unknown }) => {
    captured.clicks[props.label] = props.onClick;
    return null;
  },
}));

vi.mock("../AlignPreview", () => ({
  default: (props: { run: unknown }) => {
    captured.previewRun = props.run;
    return null;
  },
}));

vi.mock("../CropEditor", () => ({ default: () => null }));

vi.mock("../../../services/compose", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/compose")>()),
  alignChannels: mocks.alignChannels,
  alignPlan: mocks.alignPlan,
  cropChannels: mocks.cropChannels,
}));

vi.mock("../../../infrastructure/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../infrastructure/tauri")>()),
  getOutputDir: () => Promise.resolve("C:/out"),
}));

import { hooks } from "../../../test/hookRuntime";
import AlignStep from "../steps/AlignStep";
import CropStep from "../steps/CropStep";
import ExportStep from "../steps/ExportStep";
import StackStep from "../steps/StackStep";

const HA = "/data/f444w.fits";
const OIII = "/data/f200w.fits";
const SII = "/data/f150w.fits";

function stateWith(files: Record<string, string[]>, extra: Partial<WizardState> = {}): WizardState {
  return {
    ...INITIAL_STATE,
    bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: files[b.id] ?? [] })),
    ...extra,
  };
}

function alignResult(extra: Partial<AlignResult> = {}): AlignResult {
  return {
    channels: [
      { offset: [0, 0], cache_key: "__wizard_ch_tk3x9_ha_aligned" },
      { offset: [0, 0], cache_key: "__wizard_ch_tk3x9_oiii_aligned" },
    ],
    align_method: "phase_correlation",
    dimensions: [14344, 8589],
    elapsed_ms: 12,
    reference_index: 0,
    reference_rule: "first",
    run_token: "k3x9",
    warnings: [],
    ...extra,
  };
}

const ALIGNED = { ha: "__wizard_ch_tk3x9_ha_aligned", oiii: "__wizard_ch_tk3x9_oiii_aligned" };

function finishedRun(result: AlignResult): WizardAlignRun {
  return { running: false, inputs: [HA, OIII], result, error: "" };
}

function render<P extends object>(component: FunctionComponent<P>, props: P): string {
  return renderToStaticMarkup(createElement(component, props));
}

function renderAlign(state: WizardState, alignRun: WizardAlignRun | null = null, onAligned = vi.fn()): string {
  current.state = state;
  current.alignRun = alignRun;
  return render(AlignStep, { state, onAligned });
}

function row(html: string, binId: string): string {
  const start = html.indexOf(`data-bin="${binId}"`);
  if (start < 0) return "";
  const next = html.indexOf("data-bin=", start + 1);
  return html.slice(start, next < 0 ? undefined : next);
}

function warningItems(html: string): string[] | null {
  const start = html.indexOf('data-testid="align-warnings"');
  if (start < 0) return null;
  const body = html.slice(start, html.indexOf("</div>", start));
  return [...body.matchAll(/<p[^>]*>([^<]*)<\/p>/g)].map((m) => m[1]);
}

beforeEach(() => {
  captured.clicks = {};
  captured.previewRun = undefined;
  mocks.alignChannels.mockReset();
  mocks.alignPlan.mockReset();
  mocks.cropChannels.mockReset();
  mocks.dispatch.mockReset();
  mocks.stackFrames.mockReset();
  mocks.recordStackOutcome.mockReset();
  current.stackGeneration = 0;
});

describe("AlignStep reference selector", () => {
  it("offers the automatic choice first and one option per channel, showing auto by default", () => {
    const html = renderAlign(stateWith({ ha: [HA], oiii: [OIII] }));
    const select = html.slice(html.indexOf('data-testid="align-ref-select"'), html.indexOf("</select>", html.indexOf('data-testid="align-ref-select"')));
    expect(select).toMatch(/<option value="auto" selected="">Auto \(finest pixel scale\)<\/option><option value="ha">Hα<\/option><option value="oiii">OIII<\/option>/);
  });

  it("shows the user's choice while it names an align input", () => {
    const html = renderAlign(stateWith({ ha: [HA], oiii: [OIII] }, { alignRefChoice: "oiii" }));
    expect(html).toContain('<option value="oiii" selected="">OIII</option>');
  });

  it("sends the chosen reference index and a run token, then stores the reference bin and the token", async () => {
    const state = stateWith({ ha: [HA], oiii: [OIII] }, { alignRefChoice: "oiii" });
    const onAligned = vi.fn();
    mocks.alignChannels.mockResolvedValueOnce(alignResult({ reference_index: 1, reference_rule: "selected" }));
    renderAlign(state, null, onAligned);
    await captured.clicks["Align Channels"]();
    expect(mocks.alignChannels).toHaveBeenCalledTimes(1);
    const args = mocks.alignChannels.mock.calls[0];
    expect(args.slice(0, 5)).toEqual([[HA, OIII], "C:/out", "phase_correlation", ["ha", "oiii"], 1]);
    expect(args[5]).toMatch(/^[a-z0-9]{4,24}$/);
    expect(onAligned).toHaveBeenCalledWith(ALIGNED);
    expect(mocks.dispatch).toHaveBeenCalledWith({ type: "UPDATE", partial: { alignRefBinId: "oiii", alignRunToken: "k3x9" } });
  });

  it("sends no reference index under auto and stores the reference the backend chose", async () => {
    mocks.alignChannels.mockResolvedValueOnce(alignResult({ reference_index: 1, reference_rule: "finest_wcs" }));
    renderAlign(stateWith({ ha: [HA], oiii: [OIII] }));
    await captured.clicks["Align Channels"]();
    const args = mocks.alignChannels.mock.calls[0];
    expect(args[4]).toBeNull();
    expect(args[5]).toMatch(/^[a-z0-9]{4,24}$/);
    expect(mocks.dispatch).toHaveBeenCalledWith({ type: "UPDATE", partial: { alignRefBinId: "oiii", alignRunToken: "k3x9" } });
  });
});

describe("AlignStep result rows", () => {
  const reprojected: AlignedChannel = {
    offset: [-0.24, 0.31],
    cache_key: ALIGNED.ha,
    method_used: "phase_correlation",
    registered: true,
    reprojected: true,
    wcs_scale_ratio: 2.014553,
    wcs_rotation_deg: 0.486,
    prefilter_k: null,
    residual_measured: true,
  };
  const reference: AlignedChannel = { offset: [0, 0], cache_key: ALIGNED.oiii, registered: true, reprojected: false };

  function renderFinished(result: AlignResult, extra: Partial<WizardState> = {}): string {
    const state = stateWith({ ha: [HA], oiii: [OIII] }, { alignedPaths: ALIGNED, alignRefBinId: "oiii", alignRunToken: "k3x9", ...extra });
    return renderAlign(state, finishedRun(result));
  }

  it("puts the REF tag on the row the backend chose, not on the first row", () => {
    const html = renderFinished(alignResult({ channels: [reprojected, reference], reference_index: 1, reference_rule: "finest_wcs" }));
    expect(row(html, "oiii")).toContain('data-testid="align-ref-tag"');
    expect(row(html, "ha")).not.toContain('data-testid="align-ref-tag"');
  });

  it("writes the WCS note on the reprojected row and none on the reference row", () => {
    const html = renderFinished(alignResult({ channels: [reprojected, reference], reference_index: 1, reference_rule: "finest_wcs" }));
    expect(row(html, "ha")).toContain(
      'data-testid="align-channel-note"',
    );
    expect(row(html, "ha")).toContain(
      "reprojected through WCS (scale x2.0146, rotation +0.49°); residual Δx +0.3 px  Δy −0.2 px",
    );
    expect(row(html, "oiii")).not.toContain('data-testid="align-channel-note"');
  });

  it("shows no residual offset on a row the WCS alone registered, because none was measured", () => {
    const wcsOnly: AlignedChannel = { ...reprojected, offset: [0, 0], method_used: "wcs", residual_measured: false };
    const html = renderFinished(alignResult({ channels: [wcsOnly, reference], reference_index: 1, reference_rule: "finest_wcs" }));
    expect(row(html, "ha")).toContain("reprojected through WCS (scale x2.0146, rotation +0.49°); residual not measured");
    expect(row(html, "ha")).not.toContain("Δx");
  });

  it("explains a channel that was not reprojected", () => {
    const html = renderFinished(
      alignResult({
        channels: [{ offset: [0, 0], cache_key: ALIGNED.ha }, { offset: [3, -2], cache_key: ALIGNED.oiii, method_used: "phase_correlation", registered: true }],
        reference_index: 0,
      }),
      { alignRefBinId: "ha" },
    );
    expect(row(html, "oiii")).toContain("no celestial WCS in OIII; resampled by array size and registered by phase correlation");
    expect(row(html, "ha")).not.toContain('data-testid="align-channel-note"');
  });

  it("names the pair, not the channel, on a row left unprojected against a reference the user selected", () => {
    const plain: AlignedChannel = { offset: [3, -2], cache_key: ALIGNED.ha, method_used: "phase_correlation", registered: true, reprojected: false };
    const html = renderFinished(alignResult({ channels: [plain, reference], reference_index: 1, reference_rule: "selected" }));
    expect(row(html, "ha")).toContain(
      "no celestial WCS pair between Hα and the reference OIII; resampled by array size and registered by phase correlation",
    );
    expect(row(html, "ha")).not.toContain("no celestial WCS in");
    expect(row(html, "oiii")).not.toContain('data-testid="align-channel-note"');
  });

  it("lists every backend warning verbatim, one element each", () => {
    const warnings = [
      "ha: its WCS footprint does not overlap the reference grid; the channel is empty after reprojection.",
      "oiii: the WCS of this channel and the reference disagree by 6.2 px after reprojection; the stored channel is shifted by that residual, but check the plate solutions before trusting the overlay.",
    ];
    const html = renderFinished(alignResult({ channels: [reprojected, reference], reference_index: 1, warnings }));
    expect(warningItems(html)).toEqual(warnings);
  });

  it("renders no warnings list when the backend sends none", () => {
    expect(warningItems(renderFinished(alignResult({ channels: [reprojected, reference], reference_index: 1, warnings: [] })))).toBeNull();
    expect(warningItems(renderFinished(alignResult({ channels: [reprojected, reference], reference_index: 1, warnings: undefined })))).toBeNull();
  });

  it("hands the overlay the reference bin of the stored run", () => {
    renderFinished(alignResult({ channels: [reprojected, reference], reference_index: 1 }));
    expect(captured.previewRun).toMatchObject({ referenceBinId: "oiii" });
  });
});

function planOf(referenceIndex: number, rule: AlignPlan["reference_rule"], channels: [boolean, boolean][]): AlignPlan {
  return {
    channels: channels.map(([has_wcs, reprojected]) => ({ has_wcs, reprojected })),
    reference_index: referenceIndex,
    reference_rule: rule,
  };
}

interface PlanChip {
  kind: string;
  text: string;
  title: string | null;
  refTag: boolean;
}

function planChips(html: string, binId?: string): PlanChip[] {
  const scope = binId === undefined ? html : row(html, binId);
  return [...scope.matchAll(/<span([^>]*data-align-plan="([^"]+)"[^>]*)>([^<]*)<\/span>/g)].map(([, attrs, kind, text]) => ({
    kind,
    text,
    title: /title="([^"]*)"/.exec(attrs)?.[1] ?? null,
    refTag: attrs.includes('data-testid="align-ref-tag"'),
  }));
}

function methodLabel(html: string): string | undefined {
  return /<label[^>]*>([^<]*)<\/label>/.exec(html)?.[1];
}

interface TreeNode {
  type: unknown;
  props: Record<string, unknown> & { children?: ReactNode };
}

function nodes(node: ReactNode): TreeNode[] {
  if (Array.isArray(node)) return node.flatMap(nodes);
  if (!isValidElement<TreeNode["props"]>(node)) return [];
  return [{ type: node.type, props: node.props }, ...nodes(node.props.children)];
}

function renderLive(state: WizardState, alignRun: WizardAlignRun | null = null): ReactElement {
  current.state = state;
  current.alignRun = alignRun;
  return hooks.render(AlignStep, { state, onAligned: vi.fn() }) as ReactElement;
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("AlignStep plan before the run", () => {
  beforeEach(() => hooks.mount());

  it("with two WCS channels the select is labelled Fallback and each row shows its chip before the run", async () => {
    mocks.alignPlan.mockResolvedValue(planOf(1, "finest_wcs", [[true, true], [true, false]]));
    const state = stateWith({ ha: [HA], oiii: [OIII] });
    const pending = renderToStaticMarkup(renderLive(state));
    expect(mocks.alignPlan).toHaveBeenCalledWith([HA, OIII], null);
    expect(methodLabel(pending)).toBe("Method");
    expect(planChips(pending)).toEqual([]);
    await settle();
    const shown = renderToStaticMarkup(renderLive(state));
    expect(methodLabel(shown)).toBe("Fallback (channels without a WCS pair)");
    expect(planChips(shown, "ha").map((c) => [c.kind, c.text])).toEqual([["wcs", "WCS"]]);
    expect(planChips(shown, "oiii")).toEqual([
      { kind: "reference", text: "reference", title: "Reference: the finest WCS pixel scale (Auto)", refTag: true },
    ]);
    expect(row(shown, "ha")).not.toContain('data-testid="align-ref-tag"');
    expect(mocks.alignPlan).toHaveBeenCalledTimes(1);
  });

  it("names the fallback method on every row that a selected reference without a WCS leaves without a pair", async () => {
    mocks.alignPlan.mockResolvedValue(planOf(2, "selected", [[true, false], [true, false], [false, false]]));
    const state = stateWith({ ha: [HA], oiii: [OIII], sii: [SII] }, { alignRefChoice: "sii" });
    renderLive(state);
    await settle();
    const tree = renderLive(state);
    const shown = renderToStaticMarkup(tree);
    expect(mocks.alignPlan).toHaveBeenCalledWith([HA, OIII, SII], 2);
    expect(methodLabel(shown)).toBe("Method");
    expect(planChips(shown, "ha").map((c) => c.text)).toEqual(["no WCS pair: Phase Correlation"]);
    expect(planChips(shown, "oiii").map((c) => c.title)).toEqual([
      "No celestial WCS in the reference SII; resampled by array size and registered by Phase Correlation",
    ]);
    expect(planChips(shown, "sii").map((c) => [c.text, c.refTag])).toEqual([["reference", true]]);
    expect(row(shown, "sii")).not.toContain(">REF<");
    const methodSelect = nodes(tree).find((n) => n.type === "select");
    expect(methodSelect, "method select").toBeDefined();
    expect(methodSelect!.props["data-testid"]).toBe("align-method-select");
    (methodSelect!.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: "affine" } });
    const affine = renderToStaticMarkup(renderLive(state));
    expect(planChips(affine, "oiii").map((c) => c.text)).toEqual(["no WCS pair: Star-based Affine"]);
    expect(mocks.alignPlan).toHaveBeenCalledTimes(1);
  });

  it("re-reads the plan when the reference or the channels change and never shows a read made for other inputs", async () => {
    let answerAuto: (plan: AlignPlan) => void = () => {};
    mocks.alignPlan
      .mockImplementationOnce(() => new Promise<AlignPlan>((resolve) => { answerAuto = resolve; }))
      .mockResolvedValueOnce(planOf(1, "selected", [[true, true], [true, false]]))
      .mockImplementationOnce(() => new Promise<AlignPlan>(() => {}));
    const selected = stateWith({ ha: [HA], oiii: [OIII] }, { alignRefChoice: "oiii" });
    renderLive(stateWith({ ha: [HA], oiii: [OIII] }));
    renderLive(selected);
    answerAuto(planOf(0, "finest_wcs", [[true, false], [true, true]]));
    await settle();
    const shown = renderToStaticMarkup(renderLive({ ...selected, wbR: 1.2 }));
    expect(planChips(shown, "oiii").map((c) => c.text)).toEqual(["reference"]);
    expect(planChips(shown, "ha").map((c) => c.text)).toEqual(["WCS"]);
    const three = renderToStaticMarkup(renderLive(stateWith({ ha: [HA], oiii: [OIII], sii: [SII] }, { alignRefChoice: "oiii" })));
    expect(planChips(three)).toEqual([]);
    expect(methodLabel(three)).toBe("Method");
    expect(mocks.alignPlan.mock.calls).toEqual([
      [[HA, OIII], null],
      [[HA, OIII], 1],
      [[HA, OIII, SII], 1],
    ]);
  });

  it("shows no chip and keeps the plain label when the plan cannot be read", async () => {
    mocks.alignPlan.mockRejectedValue(new Error("Need at least 2 channels to align"));
    const state = stateWith({ ha: [HA], oiii: [OIII] });
    renderLive(state);
    await settle();
    const shown = renderToStaticMarkup(renderLive(state));
    expect(planChips(shown)).toEqual([]);
    expect(methodLabel(shown)).toBe("Method");
    expect(shown).not.toContain("no WCS pair");
  });

  it("shows the run's notes instead of the chips once Align ran, and keeps the Fallback label", async () => {
    const reprojectedHa: AlignedChannel = {
      offset: [-0.24, 0.31],
      cache_key: ALIGNED.ha,
      method_used: "phase_correlation",
      registered: true,
      reprojected: true,
      wcs_scale_ratio: 2.014553,
      wcs_rotation_deg: 0.486,
      prefilter_k: null,
      residual_measured: true,
    };
    const referenceOiii: AlignedChannel = { offset: [0, 0], cache_key: ALIGNED.oiii, registered: true, reprojected: false };
    mocks.alignPlan.mockResolvedValue(planOf(1, "finest_wcs", [[true, true], [true, false]]));
    const state = stateWith({ ha: [HA], oiii: [OIII] }, { alignedPaths: ALIGNED, alignRefBinId: "oiii", alignRunToken: "k3x9" });
    const run = finishedRun(alignResult({ channels: [reprojectedHa, referenceOiii], reference_index: 1, reference_rule: "finest_wcs" }));
    renderLive(state, run);
    await settle();
    const shown = renderToStaticMarkup(renderLive(state, run));
    expect(planChips(shown)).toEqual([]);
    expect(row(shown, "ha")).toContain("reprojected through WCS (scale x2.0146, rotation +0.49°)");
    expect(row(shown, "oiii")).toContain(">REF<");
    expect(methodLabel(shown)).toBe("Fallback (channels without a WCS pair)");
  });

  it("asks nothing with fewer than two channels", () => {
    renderLive(stateWith({ ha: [HA] }));
    expect(mocks.alignPlan).not.toHaveBeenCalled();
  });
});

describe("CropStep after Align", () => {
  it("crops the only filled channel without an Align run, through its own path and no run token", async () => {
    const state = stateWith({ ha: [HA] });
    current.state = state;
    current.alignRun = null;
    const onCropped = vi.fn();
    mocks.cropChannels.mockResolvedValueOnce({ cache_keys: ["__wizard_ch_ha_cropped"], paths: [], dimensions: [10, 10] });
    const html = render(CropStep, { state, onCropped });
    expect(html).not.toContain("Run Alignment first");
    expect(html).toContain("Apply crops the channel to the box.");
    expect(html).not.toContain("aligned channel");
    await captured.clicks["Apply Crop"]();
    const args = mocks.cropChannels.mock.calls[0];
    expect(args[0]).toEqual([HA]);
    expect(args[7]).toEqual(["ha"]);
    expect(args[8]).toBeNull();
    expect(onCropped).toHaveBeenCalledWith({ ha: "__wizard_ch_ha_cropped" });
  });

  it("passes the Align run token so the cropped keys join that run", async () => {
    const state = stateWith({ ha: [HA], oiii: [OIII] }, { alignedPaths: ALIGNED, alignRunToken: "k3x9" });
    current.state = state;
    current.alignRun = null;
    mocks.cropChannels.mockResolvedValueOnce({ cache_keys: ["c1", "c2"], paths: [], dimensions: [10, 10] });
    expect(render(CropStep, { state, onCropped: vi.fn() })).toContain("Apply crops every aligned channel to the box.");
    await captured.clicks["Apply Crop"]();
    const args = mocks.cropChannels.mock.calls[0];
    expect(args[0]).toEqual([ALIGNED.ha, ALIGNED.oiii]);
    expect(args[7]).toEqual(["ha", "oiii"]);
    expect(args[8]).toBe("k3x9");
  });
});

describe("ExportStep mapping banner", () => {
  it("shows the fixed mapping of a channel export without a composite", () => {
    const state = stateWith({ ha: [HA], oiii: [OIII], sii: ["/data/sii.fits"] });
    current.state = state;
    const html = render(ExportStep, { state });
    const banner = exportMappingBanner(state);
    expect(banner).toContain("R=SII G=Hα B=OIII");
    expect(html).toContain(`data-testid="export-mapping-banner"`);
    expect(html).toContain(banner as string);
  });

  it("shows no banner for a composite", () => {
    const state = stateWith({ ha: [HA], oiii: [OIII] }, { compositeReady: true });
    current.state = state;
    expect(render(ExportStep, { state })).not.toContain("export-mapping-banner");
  });
});

describe("StackStep run that outlives its inputs", () => {
  const FRAMES = ["/data/r_1.fits", "/data/r_2.fits", "/data/r_3.fits"];
  const STACKED = "/out/r_stacked.fits";

  function renderStack(): ReturnType<typeof vi.fn> {
    const state = stateWith({ r: FRAMES });
    current.state = state;
    current.alignRun = null;
    const onStacked = vi.fn();
    render(StackStep, { state, dispatch: mocks.dispatch, onStacked });
    return onStacked;
  }

  it("discards a stack that resolves after a reset instead of storing it in the new wizard", async () => {
    mocks.stackFrames.mockImplementationOnce(() => {
      current.stackGeneration = 1;
      return Promise.resolve({ fits_path: STACKED });
    });
    const onStacked = renderStack();
    await captured.clicks["Stack All"]();
    expect(mocks.stackFrames).toHaveBeenCalledTimes(1);
    expect(onStacked).not.toHaveBeenCalled();
    expect(mocks.recordStackOutcome.mock.calls).toEqual([["r", { store: false, notice: STACK_INPUTS_CHANGED }, 0]]);
  });

  it("discards a stack that the reset cancelled instead of reporting it as cancelled", async () => {
    mocks.stackFrames.mockImplementationOnce(() => {
      current.stackGeneration = 1;
      return Promise.reject(new Error("Stacking cancelled"));
    });
    const onStacked = renderStack();
    await captured.clicks["Stack All"]();
    expect(onStacked).not.toHaveBeenCalled();
    expect(mocks.recordStackOutcome.mock.calls).toEqual([["r", { store: false, notice: STACK_INPUTS_CHANGED }, 0]]);
  });

  it("stores a stack whose inputs did not change and clears the bin's notice", async () => {
    mocks.stackFrames.mockResolvedValueOnce({ fits_path: STACKED });
    const onStacked = renderStack();
    await captured.clicks["Stack All"]();
    expect(mocks.stackFrames.mock.calls[0][0]).toEqual(FRAMES);
    expect(onStacked).toHaveBeenCalledWith("r", STACKED);
    expect(mocks.recordStackOutcome.mock.calls).toEqual([["r", { store: true, notice: null }, 0]]);
  });
});
