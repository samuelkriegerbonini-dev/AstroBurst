import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import type { ProcessedFile } from "../../../shared/types";
import type { MosaicEstimate, MosaicRequest, MosaicResult, MosaicTileRow } from "../../../shared/types/stacking";

const { estimateMock, runMock, progress, ingest, dock } = vi.hoisted(() => ({
  estimateMock: vi.fn(),
  runMock: vi.fn(),
  progress: { events: new Set<string>(), state: { current: 0, total: 0, percent: 0, stage: "", active: false } },
  ingest: { showWhenLoaded: vi.fn(), ingestUnlessLoaded: vi.fn() },
  dock: { dispatch: vi.fn() },
}));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../../hooks/useFileIngest", () => ingest);
vi.mock("../../../hooks/useDockLayout", () => ({ dockStore: dock }));
vi.mock("../../../hooks/useProgress", () => ({
  useProgress: (event: string) => {
    progress.events.add(event);
    return { ...progress.state, reset: () => {} };
  },
}));
vi.mock("../../../services/progress", () => ({ cancelProgress: vi.fn() }));
vi.mock("../../../services/stacking", () => ({ estimateMosaic: estimateMock, runMosaic: runMock }));

import { open } from "@tauri-apps/plugin-dialog";
import { hooks } from "../../../test/hookRuntime";
import { cancelProgress } from "../../../services/progress";
import { ErrorAlert, WarningList } from "../../ui";
import { composeMasters } from "../resultHandoff";
import { wizardReducer, type WizardAction } from "../../../context/ComposeWizardContext";
import { INITIAL_STATE, type WizardState } from "../../../utils/wizard";
import { mosaicOutputPaths, recordMosaic, sessionMosaics, type SessionMosaic } from "../../../utils/mosaicStore";
import { CHOOSE_CANVAS, NEW_CANVAS } from "../../../utils/mosaicView";
import MosaicPanel from "../MosaicPanel";

type AnyProps = Record<string, unknown> & { children?: ReactNode };
type PanelProps = Parameters<typeof MosaicPanel>[0];

const panel = MosaicPanel as (props: PanelProps) => ReactNode;

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

const press = (el: ReactElement<AnyProps> | undefined) => (el?.props.onClick as () => unknown)();
const byTestId = (tree: ReactNode, id: string) => nodes(tree).find((el) => el.props["data-testid"] === id);
const allByTestId = (tree: ReactNode, id: string) => nodes(tree).filter((el) => el.props["data-testid"] === id);
const estimateButton = (tree: ReactNode) => nodes(tree).find((el) => "runningLabel" in el.props && el.props.label === "Estimate");
const runButton = (tree: ReactNode) => nodes(tree).find((el) => "runningLabel" in el.props && String(el.props.label).startsWith("Run mosaic"));
const selectNamed = (tree: ReactNode, name: string) => nodes(tree).find((el) => el.type === "select" && el.props["aria-label"] === name);
const optionsOf = (select: ReactElement<AnyProps> | undefined) => nodes(select?.props.children).map((o) => [o.props.value, textOf(o.props.children)]);
const choose = (select: ReactElement<AnyProps> | undefined, value: string) => (select?.props.onChange as (e: { target: { value: string } }) => void)({ target: { value } });
const checkbox = (tree: ReactNode, name: string) => nodes(tree).find((el) => el.type === "input" && el.props["aria-label"] === `Use ${name} as a panel`);

function reasonOf(tree: ReactNode, button: ReactElement<AnyProps> | undefined): string | null {
  const id = button?.props.describedBy;
  if (typeof id !== "string") return null;
  return textOf(nodes(tree).find((el) => el.props.id === id));
}

function panelFile(name: string, filter: string | null): ProcessedFile {
  const header: Record<string, string> = filter === null ? { IMAGETYP: "LIGHT" } : { IMAGETYP: "LIGHT", FILTER: filter };
  return {
    id: name,
    name,
    path: `C:/m42/panels/${name}`,
    sourcePath: `C:/m42/panels/${name}`,
    imageRef: null,
    size: 0,
    status: "done",
    result: { header } as ProcessedFile["result"],
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

const HA = ["m42_p1_Ha.fits", "m42_p2_Ha.fits", "m42_p3_Ha.fits"].map((name) => panelFile(name, "Ha"));
const OIII = ["m42_p1_OIII.fits", "m42_p2_OIII.fits"].map((name) => panelFile(name, "OIII"));

function tileRow(name: string, extra: Partial<MosaicTileRow> = {}): MosaicTileRow {
  return {
    name,
    path: `C:/m42/panels/${name}.fits`,
    width: 4000,
    height: 3000,
    planes: 1,
    bunit: null,
    single_exposure: false,
    solve: null,
    coverage_frac: 0.4,
    scale_ratio: 1,
    rotation_deg: 0.04,
    wcs_kind: "header",
    window: { x0: 0, y0: 0, cols: 4000, rows: 3000 },
    ...extra,
  };
}

const ESTIMATE: MosaicEstimate = {
  cols: 9000,
  rows: 3200,
  megapixels: 28.8,
  ram_bytes: 1.5 * 2 ** 30,
  canvas_cards: [["CTYPE1", "'RA---TAN'"]],
  tiles: [
    tileRow("m42_p1_Ha"),
    tileRow("m42_p2_Ha", { wcs_kind: "gwcs", scale_ratio: 1.04, rotation_deg: -1.24 }),
    tileRow("m42_p3_Ha", { solve: { solver: "local", tier: 0, catalog: "tier0-tycho2", matched: 42, rms_px: 0.314 }, rotation_deg: null }),
  ],
  pairs: [{ i: 0, j: 1, frac: 0.2 }],
  components: [{ tiles: [0, 1], reference: 0 }, { tiles: [2], reference: 2 }],
  isolated: [2],
  warnings: ["The mosaic needs about 1.5 GB of memory and 1.2 GB are free: the system may page."],
  refusal: null,
  default_reject: null,
  reject: null,
  planes: 1,
};

let runs = 0;

function runReply(paths: readonly string[], extra: Partial<MosaicResult> = {}): MosaicResult {
  runs += 1;
  const stem = `C:/out/m42_mosaic_run${runs}`;
  return {
    fits_path: `${stem}.fits`,
    png_path: `${stem}.png`,
    coverage_path: null,
    written: [`${stem}.fits`, `${stem}.png`],
    dimensions: [9000, 3200],
    planes: 1,
    canvas: { cols: 9000, rows: 3200, megapixels: 28.8, cards: [] },
    tiles: paths.map((path) => tileRow(path.replace(/^.*\//, "").replace(/\.fits$/, ""))),
    pairs: [{ i: 0, j: 1, frac: 0.2 }],
    components: [{ tiles: paths.map((_, k) => k), reference: 0 }],
    isolated: [],
    levels: [{ offsets: paths.map(() => 0), scales: paths.map(() => 1), unmatched: [], unscaled: [] }],
    pair_residuals: [],
    reject: null,
    rejected_px: 0,
    rejections: [],
    warnings: [],
    elapsed_ms: 83_000,
    previewUrl: `asset://${stem}.png`,
    ...extra,
  };
}

function session(name: string, label: SessionMosaic["label"]): SessionMosaic {
  return { fitsPath: `C:/out/${name}.fits`, written: [`C:/out/${name}.fits`, `C:/out/${name}.png`], name, label, planes: 1, dimensions: [9000, 3200], panels: 2 };
}

async function estimated(props: PanelProps, reply: MosaicEstimate = ESTIMATE): Promise<() => ReactNode> {
  estimateMock.mockResolvedValueOnce(reply);
  const view = () => hooks.render(panel, props) as ReactNode;
  const calls = estimateMock.mock.calls.length;
  press(estimateButton(view()));
  await vi.waitFor(() => expect(estimateMock.mock.calls.length).toBe(calls + 1));
  await vi.waitFor(() => expect(estimateButton(view())?.props.running).toBe(false));
  return view;
}

const lastEstimate = () => estimateMock.mock.calls[estimateMock.mock.calls.length - 1] as [string[], MosaicRequest];

async function ran(view: () => ReactNode, reply: MosaicResult): Promise<void> {
  runMock.mockResolvedValueOnce(reply);
  const calls = runMock.mock.calls.length;
  press(runButton(view()));
  await vi.waitFor(() => expect(runMock.mock.calls.length).toBe(calls + 1));
  await vi.waitFor(() => expect(runButton(view())?.props.running).toBe(false));
}

describe("MosaicPanel estimate before Run", () => {
  beforeEach(() => {
    hooks.mount();
    estimateMock.mockReset();
    runMock.mockReset();
    Object.assign(progress.state, { current: 0, total: 0, percent: 0, stage: "", active: false });
  });

  afterEach(() => hooks.mount());

  it("says the panels are stacked panels, not raw subs, and recommends Background extraction per panel first", () => {
    const intro = textOf(byTestId(hooks.render(panel, { files: HA }) as ReactNode, "mosaic-intro"));
    expect(intro).toContain("panels, not raw subs");
    expect(intro).toContain("Background extraction on each panel");
  });

  it("keeps Run off until the estimate, then runs exactly what was estimated and shows the mosaic", async () => {
    const onResult = vi.fn();
    const props: PanelProps = { files: HA, runTarget: { key: "k1", path: HA[0].path }, onResult };
    const view = () => hooks.render(panel, props) as ReactNode;
    expect(runButton(view())?.props.disabled).toBe(true);
    expect(reasonOf(view(), runButton(view()))).toBe("Estimate first: Run unlocks once the estimate has checked the canvas, the memory and the panel groups.");
    await estimated(props);
    const [paths, request] = lastEstimate();
    expect(paths).toEqual(HA.map((f) => f.path));
    expect(request).toEqual({
      canvas: { mode: "auto", north_up: false, reference: null },
      levels: "offset",
      references: [],
      feather_px: 64,
      dq_mask: true,
      reject: { mode: "auto" },
      solve: { online: true, use_twomass: true, max_sip_order: null },
    });
    expect(textOf(byTestId(view(), "mosaic-canvas"))).toBe("Canvas 9,000 × 3,200 px · 28.8 MP · about 1.5 GB of memory");
    expect(runButton(view())?.props.disabled).toBe(false);
    expect(runButton(view())?.props.describedBy).toBeUndefined();

    const reply = runReply(paths);
    await ran(view, reply);
    const [runPaths, outputDir, name, runRequest] = runMock.mock.calls[0];
    expect(runPaths).toEqual(paths);
    expect(outputDir).toBeUndefined();
    expect(name).toMatch(/^m42_p1_Ha_mosaic3_\d{8}-\d{6}-\d{3}$/);
    expect(runRequest).toEqual(request);
    expect(onResult.mock.calls).toEqual([[reply, { key: "k1", path: HA[0].path }]]);
    expect(mosaicOutputPaths()).toEqual(expect.arrayContaining(reply.written));
    expect(sessionMosaics()[0]).toMatchObject({ fitsPath: reply.fits_path, label: "Ha", panels: 3, planes: 1 });
    expect(textOf(view())).toContain("Finished in 1m 23s");
  });

  it("asks for a new estimate when the panels change, and names a refusal", async () => {
    const props: PanelProps = { files: HA };
    const view = await estimated(props);
    (checkbox(view(), "m42_p3_Ha.fits")?.props.onChange as () => void)();
    expect(runButton(view())?.props.disabled).toBe(true);
    expect(reasonOf(view(), runButton(view()))).toBe("The panels or settings changed since the estimate: Estimate again.");

    const refusal = "The canvas would be 52000x3200 px, above the limit of 40000 px per side. Leaving out tile 'm42_p2_Ha' shrinks it the most: check its WCS.";
    await estimated(props, { ...ESTIMATE, refusal });
    expect(lastEstimate()[0]).toEqual(HA.slice(0, 2).map((f) => f.path));
    expect(textOf(byTestId(view(), "mosaic-refusal"))).toBe(refusal);
    expect(runButton(view())?.props.disabled).toBe(true);
    expect(reasonOf(view(), runButton(view()))).toBe("The estimate refused this mosaic: change what it names, then Estimate again.");

    (checkbox(view(), "m42_p2_Ha.fits")?.props.onChange as () => void)();
    expect(estimateButton(view())?.props.disabled).toBe(true);
    expect(reasonOf(view(), estimateButton(view()))).toBe("Pick at least two panels.");
  });

  it("shows the panel groups with their level references and warns about an isolated panel", async () => {
    const view = await estimated({ files: HA });
    const groups = allByTestId(view(), "mosaic-group");
    expect(groups).toHaveLength(1);
    expect(textOf(groups[0])).toContain("Group 1: m42_p1_Ha, m42_p2_Ha");
    const reference = selectNamed(view(), "Level reference of group 1");
    expect(reference?.props.value).toBe("0");
    expect(optionsOf(reference)).toEqual([["0", "m42_p1_Ha"], ["1", "m42_p2_Ha"]]);
    const warnings = nodes(byTestId(view(), "mosaic-estimate-warnings")).find((el) => el.type === WarningList)?.props.warnings;
    expect(warnings).toEqual([
      "Panel 'm42_p3_Ha' overlaps no other panel: it keeps its own level, not matched to the others.",
      "The mosaic needs about 1.5 GB of memory and 1.2 GB are free: the system may page.",
    ]);

    choose(reference, "1");
    expect(reasonOf(view(), runButton(view()))).toBe("The panels or settings changed since the estimate: Estimate again.");
    await estimated({ files: HA });
    expect(lastEstimate()[1].references).toEqual([1]);
  });

  it("lists every panel with its WCS kind, scale, rotation, coverage, and the solver, tier and RMS of a solved panel", async () => {
    const view = await estimated({ files: HA });
    const rows = allByTestId(view(), "mosaic-tile-row").map(textOf);
    expect(rows).toHaveLength(3);
    for (const part of ["m42_p1_Ha", "Group 1 · reference", "Header WCS", "scale 1.00×", "rotation 0.0°", "40.0 % of the canvas"]) expect(rows[0]).toContain(part);
    for (const part of ["m42_p2_Ha", "gWCS", "scale 1.04×", "rotation -1.2°"]) expect(rows[1]).toContain(part);
    for (const part of ["m42_p3_Ha", "Isolated", "Solved from its header", "rotation --", "Offline solve · Tier 0: Tycho-2 (bundled) · 42 stars · RMS 0.31 px"]) expect(rows[2]).toContain(part);
    expect(textOf(byTestId(view(), "mosaic-rejection-line"))).toBe(
      "Auto: outlier rejection is off for stacked panels; choose On to reject outliers where three or more panels overlap.",
    );
  });

  it("sends the level model, feather, DQ mask and hand-set rejection", async () => {
    const props: PanelProps = { files: HA };
    const view = () => hooks.render(panel, props) as ReactNode;
    choose(selectNamed(view(), "Level matching"), "offset_scale");
    (nodes(view()).find((el) => el.props.label === "Seam feather")?.props.onChange as (v: number) => void)(128);
    (nodes(view()).find((el) => el.props.label === "Leave out DQ-flagged pixels")?.props.onChange as (v: boolean) => void)(false);
    choose(selectNamed(view(), "Outlier rejection"), "on");
    (nodes(view()).find((el) => el.props.label === "Rejection SNR")?.props.onChange as (v: number) => void)(4);
    (nodes(view()).find((el) => el.props.label === "Gradient scale")?.props.onChange as (v: number) => void)(0.8);
    await estimated(props, { ...ESTIMATE, reject: { snr: 4, scale: 0.8 } });
    expect(lastEstimate()[1]).toMatchObject({ levels: "offset_scale", feather_px: 128, dq_mask: false, reject: { mode: "on", snr: 4, scale: 0.8 } });
    expect(textOf(byTestId(view(), "mosaic-rejection-line"))).toBe(
      "Outlier rejection is on (SNR 4, scale 0.8) where three or more panels overlap; where only two overlap nothing is rejected.",
    );
  });

  it("warns when the picked panels hold several filters and picks one filter in one press", async () => {
    const props: PanelProps = { files: [...HA, ...OIII] };
    const view = () => hooks.render(panel, props) as ReactNode;
    expect(textOf(byTestId(view(), "mosaic-filters"))).toContain(
      "The picked panels hold 2 filters (Ha 3, OIII 2): a mosaic joins panels of one filter, so build one mosaic per filter.",
    );
    press(nodes(view()).find((el) => el.type === "button" && textOf(el.props.children) === "Only OIII"));
    expect(byTestId(view(), "mosaic-filters")).toBeUndefined();
    await estimated(props);
    expect(lastEstimate()[0]).toEqual(OIII.map((f) => f.path));
  });

  it("leaves the mosaics of this session out of the panels until ticked", async () => {
    const earlier = session("m42_mosaic_earlier", "Ha");
    recordMosaic(earlier);
    const loaded = { ...panelFile("m42_mosaic_earlier.fits", "Ha"), path: earlier.fitsPath };
    const props: PanelProps = { files: [...HA, loaded] };
    const view = () => hooks.render(panel, props) as ReactNode;
    expect(checkbox(view(), "m42_mosaic_earlier.fits")?.props.checked).toBe(false);
    await estimated(props);
    expect(lastEstimate()[0]).toEqual(HA.map((f) => f.path));
  });
});

describe("MosaicPanel canvas picker", () => {
  beforeEach(() => {
    hooks.mount();
    estimateMock.mockReset();
    vi.mocked(open).mockReset();
  });

  afterEach(() => hooks.mount());

  it("uses the canvas of a session mosaic or a chosen FITS, and offers the axes only for a new canvas", async () => {
    const oiii = session("m42_oiii_canvas", "OIII");
    recordMosaic(oiii);
    const props: PanelProps = { files: HA };
    const view = () => hooks.render(panel, props) as ReactNode;
    const canvas = () => selectNamed(view(), "Canvas");
    expect(canvas()?.props.value).toBe(NEW_CANVAS);
    expect(optionsOf(canvas())).toContainEqual([oiii.fitsPath, "Use the canvas of m42_oiii_canvas"]);
    expect(optionsOf(canvas())).toContainEqual([CHOOSE_CANVAS, "Use the canvas of another FITS..."]);
    expect(optionsOf(selectNamed(view(), "Canvas axes follow"))).toEqual(HA.map((f) => [f.path, f.name]));

    choose(canvas(), oiii.fitsPath);
    expect(selectNamed(view(), "Canvas axes follow")).toBeUndefined();
    expect(nodes(view()).find((el) => el.props.label === "North up")).toBeUndefined();
    await estimated(props);
    expect(lastEstimate()[1].canvas).toEqual({ mode: "like", path: oiii.fitsPath });

    vi.mocked(open).mockResolvedValueOnce("D:/archive/ha_mosaic.fits");
    choose(canvas(), CHOOSE_CANVAS);
    await vi.waitFor(() => expect(canvas()?.props.value).toBe("D:/archive/ha_mosaic.fits"));
    expect(vi.mocked(open).mock.calls[0][0]).toMatchObject({ multiple: false, title: "FITS whose canvas to use" });
    expect(optionsOf(canvas())).toContainEqual(["D:/archive/ha_mosaic.fits", "Use the canvas of ha_mosaic.fits"]);
    await estimated(props);
    expect(lastEstimate()[1].canvas).toEqual({ mode: "like", path: "D:/archive/ha_mosaic.fits" });

    vi.mocked(open).mockResolvedValueOnce(null);
    choose(canvas(), CHOOSE_CANVAS);
    await vi.waitFor(() => expect(vi.mocked(open)).toHaveBeenCalledTimes(2));
    expect(canvas()?.props.value).toBe("D:/archive/ha_mosaic.fits");

    choose(canvas(), NEW_CANVAS);
    choose(selectNamed(view(), "Canvas axes follow"), HA[1].path);
    await estimated(props);
    expect(lastEstimate()[1].canvas).toEqual({ mode: "auto", north_up: false, reference: 1 });
    (nodes(view()).find((el) => el.props.label === "North up")?.props.onChange as (v: boolean) => void)(true);
    expect(selectNamed(view(), "Canvas axes follow")).toBeUndefined();
    await estimated(props);
    expect(lastEstimate()[1].canvas).toEqual({ mode: "auto", north_up: true, reference: null });
  });
});

describe("MosaicPanel progress, cancel and results", () => {
  beforeEach(() => {
    hooks.mount();
    estimateMock.mockReset();
    runMock.mockReset();
    progress.events.clear();
    Object.assign(progress.state, { current: 0, total: 0, percent: 0, stage: "", active: false });
    vi.mocked(cancelProgress).mockReset().mockResolvedValue(true);
  });

  afterEach(() => hooks.mount());

  it("follows mosaic-progress, cancels through it, and reads a cancelled run as Mosaic cancelled", async () => {
    const props: PanelProps = { files: HA };
    const view = await estimated(props);
    let fail: (error: Error) => void = () => {};
    runMock.mockReturnValueOnce(new Promise<MosaicResult>((_, reject) => { fail = reject; }));
    press(runButton(view()));
    await vi.waitFor(() => expect(runMock).toHaveBeenCalledTimes(1));
    expect([...progress.events]).toEqual(["mosaic-progress"]);
    expect(textOf(byTestId(view(), "mosaic-progress-label"))).toBe("Starting the mosaic...");
    Object.assign(progress.state, { current: 8, total: 14, percent: 57, stage: "warp:2:2/3:m42_p2_Ha", active: true });
    expect(textOf(byTestId(view(), "mosaic-progress-label"))).toBe("Reprojecting m42_p2_Ha (2/3)");
    expect(byTestId(view(), "mosaic-progress-bar")?.props.style).toMatchObject({ width: "57%" });
    expect(estimateButton(view())?.props.disabled).toBe(true);
    const cancel = byTestId(view(), "mosaic-cancel");
    expect(cancel?.props["aria-label"]).toBe("Cancel mosaic");
    press(cancel);
    expect(vi.mocked(cancelProgress).mock.calls).toEqual([["mosaic-progress"]]);
    fail(new Error("Operation cancelled"));
    await vi.waitFor(() => expect(textOf(view())).toContain("Mosaic cancelled"));
    expect(nodes(view()).find((el) => el.type === ErrorAlert)?.props.message).toBeNull();
    expect(byTestId(view(), "mosaic-cancel")).toBeUndefined();
  });

  it("names the cancel of an estimate and shows an estimate error", async () => {
    let fail: (error: Error) => void = () => {};
    estimateMock.mockReturnValueOnce(new Promise<MosaicEstimate>((_, reject) => { fail = reject; }));
    const view = () => hooks.render(panel, { files: HA }) as ReactNode;
    press(estimateButton(view()));
    await vi.waitFor(() => expect(estimateMock).toHaveBeenCalledTimes(1));
    expect(textOf(byTestId(view(), "mosaic-progress-label"))).toBe("Reading the panels...");
    Object.assign(progress.state, { current: 1, total: 3, percent: 33, stage: "solve:2:2/3:m42_p2_Ha", active: true });
    expect(textOf(byTestId(view(), "mosaic-progress-label"))).toBe("Solving m42_p2_Ha (2/3)");
    expect(byTestId(view(), "mosaic-cancel")?.props["aria-label"]).toBe("Cancel estimate");
    const message = "Tile 'm42_p2_Ha' (C:/m42/panels/m42_p2_Ha.fits) has no celestial WCS and no pointing in the header: solve it in Plate Solve with a hint, then Write WCS";
    fail(new Error(message));
    await vi.waitFor(() => expect(nodes(view()).find((el) => el.type === ErrorAlert)?.props.message).toBe(message));
    expect(reasonOf(view(), runButton(view()))).toBe("Estimate first: Run unlocks once the estimate has checked the canvas, the memory and the panel groups.");
  });

  it("reports the star offsets of every overlap after the run and flags a pair above 0.5 px by name", async () => {
    const view = await estimated({ files: HA });
    const warning = "Tiles 'm42_p1_Ha' and 'm42_p2_Ha' disagree by 1.00 px on the positions of 34 shared stars after the warp: check the WCS of both, for example by plate solving them again.";
    await ran(view, runReply(HA.map((f) => f.path), {
      pair_residuals: [
        { i: 0, j: 1, dx: 0.8, dy: 0.6, matched: 34, rms_px: 0.2 },
        { i: 1, j: 2, dx: 0.1, dy: 0.05, matched: 12, rms_px: 0.1 },
      ],
      warnings: [warning],
    }));
    const rows = allByTestId(view(), "mosaic-residual-row");
    expect(rows.map((row) => row.props["data-warn"])).toEqual([true, false]);
    expect(textOf(rows[0])).toContain("m42_p1_Ha ↔ m42_p2_Ha");
    expect(textOf(rows[0])).toContain("1.00 px (dx 0.80, dy 0.60)");
    expect(textOf(rows[0])).toContain("above 0.5 px");
    expect(textOf(rows[1])).not.toContain("above 0.5 px");
    expect(nodes(byTestId(view(), "mosaic-result-warnings")).find((el) => el.type === WarningList)?.props.warnings).toEqual([warning]);
  });
});

describe("MosaicPanel Compose these mosaics", () => {
  let state: WizardState;
  const wizard = {
    getState: () => state,
    dispatch: (action: WizardAction) => {
      state = wizardReducer(state, action);
    },
    setActiveStep: vi.fn(),
  };
  const filled = () => Object.fromEntries(state.bins.filter((bin) => bin.files.length > 0).map((bin) => [bin.id, bin.files]));

  beforeEach(() => {
    hooks.mount();
    state = INITIAL_STATE;
    estimateMock.mockReset();
    runMock.mockReset();
  });

  afterEach(() => hooks.mount());

  async function haMosaicAndOiii(extra: Partial<PanelProps>): Promise<{ view: () => ReactNode; ha: string; oiii: string }> {
    const oiii = session(`m42_oiii_compose${runs}`, "OIII");
    recordMosaic(oiii);
    const props: PanelProps = { files: HA, onComposeMosaics: (masters) => composeMasters(masters, wizard), ...extra };
    const view = await estimated(props);
    const reply = runReply(HA.map((f) => f.path));
    await ran(view, reply);
    return { view, ha: reply.fits_path, oiii: oiii.fitsPath };
  }

  it("puts each mosaic in its filter's Compose bin at once when the wizard has no progress", async () => {
    const { view, ha, oiii } = await haMosaicAndOiii({ composeHasProgress: false });
    const compose = byTestId(view(), "mosaic-compose");
    expect(textOf(compose)).toBe("Compose these mosaics");
    press(compose);
    expect(filled()).toEqual({ ha: [ha], oiii: [oiii] });
    expect(dock.dispatch).toHaveBeenCalledWith({ type: "open", tool: "compose" });
  });

  it("arms when the wizard has progress and replaces the channels on the second press", async () => {
    const { view, ha, oiii } = await haMosaicAndOiii({ composeHasProgress: true });
    press(byTestId(view(), "mosaic-compose"));
    expect(filled()).toEqual({});
    const armed = byTestId(view(), "mosaic-compose");
    expect(textOf(armed)).toBe("Replace Compose channels?");
    expect(armed?.props["aria-label"]).toBe("Confirm replacing the Compose channels with these mosaics");
    expect(armed?.props["aria-live"]).toBe("polite");
    press(armed);
    expect(filled()).toEqual({ ha: [ha], oiii: [oiii] });
    expect(textOf(byTestId(view(), "mosaic-compose"))).toBe("Compose these mosaics");
  });

  it("lets the channel of a session mosaic be changed by hand", () => {
    const relabelled = session("m42_relabel", "OIII");
    recordMosaic(relabelled);
    const view = () => hooks.render(panel, { files: HA }) as ReactNode;
    choose(selectNamed(view(), "Channel of m42_relabel"), "SII");
    expect(sessionMosaics().find((m) => m.fitsPath === relabelled.fitsPath)?.label).toBe("SII");
    expect(selectNamed(view(), "Channel of m42_relabel")?.props.value).toBe("SII");
  });
});
