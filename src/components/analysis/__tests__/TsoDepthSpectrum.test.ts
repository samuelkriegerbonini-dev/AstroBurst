import { beforeEach, afterEach, describe, expect, it, vi, type MockInstance } from "vitest";
import type { ReactElement, ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { TsoCurvesState, TsoSectionProps } from "../../../hooks/useTsoStore";
import { DEFAULT_LIGHT_CURVE_VIEW, type LightCurveDoc, type LightCurveViewState } from "../../../shared/types/lightCurve";
import type { TransitFitResult, TransitFittedParameter, TransitParameterName } from "../../../shared/types/transit";
import type { TsoCurve, TsoFile, TsoLightCurves, TsoLightCurvesRequest, TsoOpenResult, TsoSeriesSummary } from "../../../shared/types/tso";
import type { TsoTransitBin, TsoTransitFitRequest, TsoTransitSpectrum } from "../../../shared/types/tsoTransit";
import { QUICK_LOOK_LABEL } from "../../../utils/lightCurveAnalysis";
import { BAND_SPECIES } from "../../../utils/lineList";
import type { DepthRow, DepthSpectrum } from "../../../utils/tsoLightCurves";

const state = vi.hoisted(() => ({
  docs: [] as unknown[],
  view: null as unknown,
  curves: { loading: false, result: null, error: null } as unknown,
  spectrum: null as unknown,
  depthSpectrum: vi.fn(),
  patchDraft: vi.fn(),
  useTsoCurves: vi.fn(),
  useLightCurves: vi.fn(),
  useLightCurveView: vi.fn(),
  plot: null as Record<string, unknown> | null,
}));
const curvesRequest = vi.hoisted(() => {
  const REQ = { mode: "pipeline", bins: [[0.6, 1], [1, 2]] };
  return { REQ, getRequest: vi.fn((_group: string) => REQ) };
});
const transit = vi.hoisted(() => ({ fn: vi.fn() }));
const wfit = vi.hoisted(() => ({ record: null as unknown }));
const left = vi.hoisted(() => ({ fn: vi.fn((..._args: unknown[]) => [] as number[]) }));

vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../hooks/useLightCurveStore", () => ({
  useLightCurves: (group: string | null) => state.useLightCurves(group),
  useLightCurveView: (group: string | null) => state.useLightCurveView(group),
}));
vi.mock("../../../hooks/useTsoStore", () => ({
  tsoStore: { patchDraft: state.patchDraft, getRequest: curvesRequest.getRequest },
  useTsoCurves: (group: string | null) => state.useTsoCurves(group),
}));
vi.mock("../../../services/tsoTransit", () => ({ tsoTransitSpectrum: transit.fn }));
vi.mock("../../../hooks/useTransitFitStore", async (original) => ({
  ...(await original<typeof import("../../../hooks/useTransitFitStore")>()),
  useTransitFit: () => wfit.record,
}));
vi.mock("../../../utils/tsoTransitSpectrum", async (original) => ({
  ...(await original<typeof import("../../../utils/tsoTransitSpectrum")>()),
  leftOutRows: left.fn,
}));
vi.mock("../../../utils/tsoLightCurves", async (original) => ({
  ...(await original<typeof import("../../../utils/tsoLightCurves")>()),
  depthSpectrum: (...args: unknown[]) => state.depthSpectrum(...args),
}));
const exportMocks = vi.hoisted(() => ({
  save: vi.fn(),
  csv: vi.fn(() => "DEPTH"),
  drafts: 2,
}));
vi.mock("../../../utils/spectrumExport", async (original) => ({
  ...(await original<typeof import("../../../utils/spectrumExport")>()),
  saveCsvDialog: exportMocks.save,
}));
vi.mock("../../../utils/tsoExports", () => ({ tsoDepthSpectrumCsv: exportMocks.csv, tsoDepthSpectrumCsvFileName: () => "depth.csv" }));
vi.mock("../../../utils/measurementLogTso", () => ({
  tsoDepthSpectrumEntries: (_spectrum: unknown, _result: unknown, _view: unknown, _files: unknown, path: string | null) =>
    [
      { draft: "white", path },
      { draft: "bin", path },
    ].slice(0, exportMocks.drafts),
}));
const fitExports = vi.hoisted(() => ({
  csvArgs: null as unknown[] | null,
  nameArgs: null as unknown[] | null,
  entriesArgs: null as unknown[] | null,
  drafts: 2,
}));
vi.mock("../../../utils/tsoTransitExports", () => ({
  tsoTransitSpectrumCsv: (...args: unknown[]) => {
    fitExports.csvArgs = args;
    return "FIT";
  },
  tsoTransitSpectrumCsvFileName: (...args: unknown[]) => {
    fitExports.nameArgs = args;
    return "fit.csv";
  },
  tsoTransitSpectrumEntries: (spectrum: unknown, files: unknown, path: string | null) => {
    fitExports.entriesArgs = [spectrum, files, path];
    return [
      { draft: "bin0", path },
      { draft: "bin1", path },
    ].slice(0, fitExports.drafts);
  },
}));
vi.mock("../../regions/ProfilePlot", () => ({
  default: (props: Record<string, unknown>) => {
    state.plot = props;
    return "[plot]";
  },
}));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import { measurementLog } from "../../../utils/measurementLog";
import { APP_VERSION } from "../../../utils/constants";
import { EMPTY_TRANSIT_FIT, transitFitStore, type TransitFitRecord } from "../../../hooks/useTransitFitStore";
import { lightCurveModel } from "../../../utils/lightCurveViewModel";
import { DEFAULT_TRANSIT_FIT_SETUP, transitFitInput, transitFitSignature } from "../../../utils/transitFitSetup";
import { TRANSIT_FIT_LABEL, fitTableRows } from "../../../utils/tsoTransitSpectrum";

const { default: TsoDepthSpectrum } = await import("../TsoDepthSpectrum");
const { bandText } = await import("../../../utils/tsoLightCurves");

type Section = (props: TsoSectionProps) => ReactNode;
const section = (TsoDepthSpectrum as unknown as { type: Section }).type;

type Props = Record<string, unknown>;

const PATH = "C:/d/e1_x1dints.fits";
const GROUP = "tso:4b385da81d5c70b8:NRS1:0";
const OTHER_GROUP = "tso:4b385da81d5c70b8:NRS2:0";
const PIXEL_WARNING =
  "mean NPIXELS inside the window is below 0.9 of the baseline in 1 of 1 curves (lowest ratio 0.439): the extraction lost pixels during the event, so these depths are biased high";
const HINT = "Add wavelength bins and compute the light curves to get a depth spectrum.";
const NO_WINDOW = "Set a transit window in the light-curve view above: the depth of every bin is then drawn against wavelength.";
const NO_DEPTH = "No bin has a depth: the transit window holds no point, or no point is left outside it.";
const USE_BANDS = "Use the shown bands as bins";
const RECOMPUTE = "Compute the light curves again to get one curve per band.";
const CAPTION = "Shaded ranges are where each species absorbs (vacuum, observed frame); they are a guide, not an identification.";

const E1_BANDS: [number, number][] = [
  [0.6, 1],
  [1, 2],
  [2, 3],
  [3, 4],
  [4, 5.3],
];
const E1_DEPTHS = [0.9674, 0.9833, 0.936, 0.9695, 0.8731];
const E1_ERRORS = [0.010016353251191705, 0.0048012563432253096, 0.0071442668865198, 0.0030302124497207017, 0.00809113864280491];

function e1File(): TsoFile {
  return {
    path: PATH,
    file_name: "e1_x1dints.fits",
    sha256: "a5e7022d22e779fdf43b14fba6e7d23644c9547c920b6a52c13435c63706bfbc",
    level: 3,
    datamodel: "TSOMultiSpecModel",
    cal_ver: "1.18.0",
    exp_type: "NRS_BRIGHTOBJ",
    layout: "rows",
    nints: 240,
    intstart: 1,
    intend: 240,
    exsegnum: null,
    exsegtot: null,
    instrument: "NIRSPEC",
    detector: "NRS1",
    filter: "CLEAR",
    pupil: null,
    grating: "PRISM",
    subarray: "SUB2048",
    target: "WD 1856+534 b",
    expstart: 60061.44,
    outlier_detection: "COMPLETE",
    whitelight_log: { min_um: null, max_um: null },
    whtlt_path: null,
    n_extract1d: 1,
    data_bytes: 14702400,
  };
}

function e1Series(): TsoSeriesSummary {
  return {
    key: { detector: "NRS1", sporder: 0 },
    label: "NRS1",
    n_integrations: 240,
    n_channels: 431,
    flux_unit: "Jy",
    time_scale: "bmjd_tdb",
    time_source: "TDB-MID column",
    time_first: 60061.4421733457,
    time_last: 60061.52706665472,
    wavelength_min_um: 0.5530469695008425,
    wavelength_max_um: 5.370352185327354,
    common_grid: true,
    outlier_detection: "COMPLETE",
    completeness: [{ exposure: "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048", nints: 240, found: 240, missing: [], complete: true }],
    notes: [],
    warnings: [],
  };
}

function e1Set(): TsoOpenResult {
  return { set_id: "4b385da81d5c70b8", files: [e1File()], series: [e1Series()], notes: [], warnings: [] };
}

function props(group = GROUP): TsoSectionProps {
  return { fileKey: "f1|p", paths: [PATH], set: e1Set(), series: e1Series(), group };
}

function curve(band: [number, number]): TsoCurve {
  return { band_um: band, limits_origin: "open", flux: [1, 1], flux_err: [0.01, 0.01], n_used: [3, 3], npixels_mean: [7, 7] };
}

function curvesResult(bins: [number, number][]): TsoLightCurves {
  return {
    set_id: "4b385da81d5c70b8",
    label: "NRS1",
    series: { detector: "NRS1", sporder: 0 },
    mode: "pipeline",
    time_scale: "bmjd_tdb",
    time: [60061.44, 60061.45],
    mjd_utc: [null, null],
    int_num: [1, 2],
    row_index: [0, 1],
    flux_unit: "Jy",
    white: curve([0.553, 5.37]),
    bins: bins.map(curve),
    dropped: { nan_time: 0, repeated_time: 0 },
    rejected_elements: [],
    notes: [],
    warnings: [],
  };
}

function row(docId: string, band: [number, number], depth: number | null, err: number | null, extra: Partial<DepthRow> = {}): DepthRow {
  return {
    docId,
    label: bandText(band),
    band,
    centre: (band[0] + band[1]) / 2,
    halfWidth: (band[1] - band[0]) / 2,
    depth,
    errScatter: err,
    errFormal: null,
    err,
    nIn: 6,
    nOut: 210,
    pixelRatio: null,
    pixelLoss: false,
    ...extra,
  };
}

function e1Spectrum(extra: Partial<DepthSpectrum> = {}, white: Partial<DepthRow> = {}): DepthSpectrum {
  return {
    white: row("white", [0.553, 5.37], 0.9504, 0.0063, white),
    rows: E1_BANDS.map((band, k) => row(`bin:${k}`, band, E1_DEPTHS[k], E1_ERRORS[k])),
    window: { start: 60061.50682013, end: 60061.50859614 },
    marginDays: 0.0044,
    notes: [QUICK_LOOK_LABEL],
    warnings: [],
    ...extra,
  };
}

const WHITE: LightCurveDoc = {
  id: "white",
  label: "white light 0.6 to 5.3 um",
  quantity: "flux",
  unit: "Jy",
  timeScale: "bmjd_tdb",
  time: [60000.1, 60000.2, 60000.3, 60000.4, 60000.5, 60000.6],
  value: [10, 10, 10, 10, 10, 10],
  err: [0.5, 0.5, 0.5, 0.5, 0.5, 0.5],
  pointRef: [1, 2, 3, 4, 5, 6],
  flags: [[], [], ["pixel loss"], [], [], []],
  source: { kind: "tso", paths: ["series_x1dints.fits"], detector: "NRS1", sporder: null, band_um: [0.6, 5.3], mode: "pipeline" },
  notes: [],
};
const DOCS = [WHITE, { id: "bin:0" }] as unknown as LightCurveDoc[];
const VIEW: LightCurveViewState = { ...DEFAULT_LIGHT_CURVE_VIEW, window: { start: 60061.50682013, end: 60061.50859614 }, marginDays: 0.0044 };

function show(group = GROUP): { tree: ReactNode; markup: string } {
  const tree = hooks.render(section, props(group)) as ReactElement | null;
  return { tree, markup: tree ? renderToStaticMarkup(tree) : "" };
}

function withProp(tree: ReactNode, name: string): ReactElement<Props>[] {
  return elementsOf<Props>(tree).filter((element) => element.props[name] !== undefined);
}

function button(tree: ReactNode, text: string): ReactElement<Props> | undefined {
  return elementsOf<Props>(tree).find((element) => element.type === "button" && textOf(element) === text);
}

function click(tree: ReactNode, text: string): void {
  (button(tree, text)?.props.onClick as () => void)();
}

function bandLabels(): string[] {
  return ((state.plot?.bands as { label: string }[]) ?? []).map((band) => band.label);
}

beforeEach(() => {
  hooks.mount();
  state.docs = DOCS;
  state.view = VIEW;
  state.curves = { loading: false, result: curvesResult(E1_BANDS), error: null } satisfies TsoCurvesState;
  state.spectrum = e1Spectrum();
  state.plot = null;
  state.depthSpectrum.mockReset();
  state.depthSpectrum.mockImplementation(() => state.spectrum);
  state.patchDraft.mockReset();
  state.useTsoCurves.mockReset();
  state.useTsoCurves.mockImplementation(() => state.curves);
  state.useLightCurves.mockReset();
  state.useLightCurves.mockImplementation(() => state.docs);
  state.useLightCurveView.mockReset();
  state.useLightCurveView.mockImplementation(() => state.view);
  wfit.record = EMPTY_TRANSIT_FIT;
});

afterEach(() => hooks.mount());

describe("TsoDepthSpectrum", () => {
  it("asks for bins when the curves have none", () => {
    state.curves = { loading: false, result: null, error: null };
    const { tree, markup } = show();
    expect(markup).toContain("Depth spectrum");
    expect(markup).toContain(HINT);
    expect(markup).not.toContain("[plot]");
    expect(markup).not.toContain("<table");
    expect(withProp(tree, "data-tso-bands")).toEqual([]);
    expect(markup).not.toContain(QUICK_LOOK_LABEL);

    state.curves = { loading: false, result: curvesResult([]), error: null };
    const { tree: noBins, markup: noBinsMarkup } = show();
    expect(noBinsMarkup).toContain("Depth spectrum");
    expect(noBinsMarkup).toContain(HINT);
    expect(noBinsMarkup).not.toContain("[plot]");
    expect(noBinsMarkup).not.toContain("<table");
    expect(withProp(noBins, "data-tso-bands")).toEqual([]);
    expect(state.depthSpectrum).not.toHaveBeenCalled();
    expect(state.plot).toBeNull();
  });

  it("asks for a transit window when the view has none", () => {
    state.spectrum = e1Spectrum({ window: null });
    const { tree, markup } = show();
    const text = textOf(tree);
    expect(text).toContain("Depth spectrum");
    expect(text).toContain(QUICK_LOOK_LABEL);
    expect(withProp(tree, "data-tso-no-window").map(textOf)).toEqual([NO_WINDOW]);
    expect(markup).not.toContain("[plot]");
    expect(markup).not.toContain("<table");
    expect(withProp(tree, "data-tso-bands")).toEqual([]);
    expect(withProp(tree, "data-tso-depth-table")).toEqual([]);
    expect(button(tree, USE_BANDS)).toBeUndefined();
    expect(markup).not.toContain(HINT);
  });

  it("draws the depth spectrum with the default bands and the white light line", () => {
    const { tree, markup } = show();

    expect(markup).toContain("[plot]");
    expect(withProp(tree, "data-tso-depth")).toHaveLength(1);
    expect(state.depthSpectrum).toHaveBeenCalledWith(DOCS, VIEW, (state.curves as TsoCurvesState).result);
    expect(state.useLightCurves).toHaveBeenCalledWith(GROUP);
    expect(state.useLightCurveView).toHaveBeenCalledWith(GROUP);
    expect(state.useTsoCurves).toHaveBeenCalledWith(GROUP);
    expect(state.plot).toMatchObject({
      xLabel: "wavelength (um)",
      yLabel: "depth (1 - F_in / F_out)",
      csvName: "tso_depth_spectrum",
      logToggle: false,
      toolbar: true,
      height: 170,
    });
    const series = state.plot?.series as { x: number[]; y: (number | null)[]; yErr: (number | null)[]; label: string; color: string; mode: string }[];
    expect(series).toHaveLength(1);
    expect(series[0].x).toEqual([0.8, 1.5, 2.5, 3.5, 4.65]);
    expect(series[0].y).toEqual(E1_DEPTHS);
    expect(series[0].yErr).toEqual(E1_ERRORS);
    expect(series[0]).toMatchObject({ label: "depth", color: "#fbbf24", mode: "points" });
    expect(state.plot?.referenceLines).toEqual([{ axis: "y", value: 0.9504, label: "white light", dashed: true }]);
    expect(bandLabels()).toEqual(["H2O 2.7", "CO2 4.3", "CO 4.7", "CH4 3.3"]);
    expect(withProp(tree, "data-tso-quick-look").map(textOf)).toEqual(["quick look, no limb darkening or systematics model"]);
    expect(textOf(withProp(tree, "data-tso-bands")[0])).toContain(CAPTION);
    expect(textOf(tree)).not.toContain(NO_DEPTH);
    expect(textOf(tree)).not.toContain(HINT);
  });

  it("switches a species on and off", () => {
    const { tree } = show();
    const control = withProp(tree, "data-tso-bands");
    expect(control).toHaveLength(1);
    expect(textOf(control[0])).toContain("Bands");
    const chips = elementsOf<Props>(control[0]).filter((element) => element.type === "button");
    expect(chips).toHaveLength(14);
    expect(chips.map(textOf)).toEqual([...BAND_SPECIES]);
    expect(chips.filter((chip) => chip.props["aria-pressed"] === true).map(textOf)).toEqual(["H2O", "CO2", "CO", "CH4", "SO2"]);
    expect(chips.filter((chip) => chip.props["aria-pressed"] === false)).toHaveLength(9);

    click(control[0], "NH3");
    const { tree: withNh3 } = show();
    expect(bandLabels()).toEqual(["H2O 2.7", "CO2 4.3", "CO 4.7", "CH4 3.3", "NH3 3.0"]);
    expect(button(withProp(withNh3, "data-tso-bands")[0], "NH3")?.props["aria-pressed"]).toBe(true);

    click(withProp(withNh3, "data-tso-bands")[0], "CO2");
    show();
    expect(bandLabels()).toEqual(["H2O 2.7", "CO 4.7", "CH4 3.3", "NH3 3.0"]);

    show(OTHER_GROUP);
    expect(bandLabels()).toEqual(["H2O 2.7", "CO2 4.3", "CO 4.7", "CH4 3.3"]);
  });

  it("prints the table and the pixel-loss warning", () => {
    state.spectrum = e1Spectrum({ warnings: [PIXEL_WARNING] }, { pixelRatio: 0.4386, pixelLoss: true });
    const { tree, markup } = show();

    expect(markup).toContain(PIXEL_WARNING);
    const table = withProp(tree, "data-tso-depth-table");
    expect(table).toHaveLength(1);
    expect(elementsOf<Props>(table[0]).filter((element) => element.type === "th").map(textOf)).toEqual([
      "band",
      "depth",
      "error",
      "N in / out",
      "NPIXELS in / out",
    ]);
    const bodies = elementsOf<Props>(table[0]).filter((element) => element.type === "tbody");
    expect(bodies).toHaveLength(1);
    const rows = elementsOf<Props>(bodies[0]).filter((element) => element.type === "tr");
    expect(rows).toHaveLength(6);
    const cells = (tr: ReactElement<Props>) => elementsOf<Props>(tr).filter((element) => element.type === "td");
    expect(cells(rows[0]).map(textOf)).toEqual(["white light", "0.9504", "0.0063", "6 / 210", "0.439"]);
    expect(String(cells(rows[0])[4].props.className)).toContain("text-amber-300");
    expect(cells(rows[1]).map(textOf)).toEqual(["0.6 to 1 um", "0.9674", "0.01", "6 / 210", "--"]);
    expect(String(cells(rows[1])[4].props.className)).not.toContain("amber");
    expect(rows.slice(1).map((tr) => textOf(cells(tr)[0]))).toEqual(["0.6 to 1 um", "1 to 2 um", "2 to 3 um", "3 to 4 um", "4 to 5.3 um"]);
    expect(rows.map((tr) => textOf(cells(tr)[2]))).toEqual(["0.0063", "0.01", "0.0048", "0.0071", "0.003", "0.0081"]);
  });

  it("turns the shown bands into bins of the next computation", () => {
    const { tree } = show();
    expect(button(tree, USE_BANDS)?.props.disabled).toBe(false);
    expect(withProp(tree, "data-tso-bands-hint")).toEqual([]);

    click(tree, USE_BANDS);
    expect(state.patchDraft).toHaveBeenCalledTimes(1);
    expect(state.patchDraft).toHaveBeenCalledWith(GROUP, {
      extraBins: [
        [2.5, 3],
        [4.15, 4.5],
        [4.3, 5.1],
        [3.15, 3.5],
      ],
    });

    const { tree: asked } = show();
    expect(withProp(asked, "data-tso-bands-hint").map(textOf)).toEqual([RECOMPUTE]);
    const { tree: again } = show();
    expect(textOf(again)).toContain(RECOMPUTE);
    const { tree: elsewhere } = show(OTHER_GROUP);
    expect(textOf(elsewhere)).not.toContain(RECOMPUTE);
    const { tree: back } = show();
    expect(textOf(back)).not.toContain(RECOMPUTE);

    click(back, USE_BANDS);
    const { tree: askedAgain } = show();
    expect(textOf(askedAgain)).toContain(RECOMPUTE);
    click(withProp(askedAgain, "data-tso-bands")[0], "NH3");
    const { tree: toggled } = show();
    expect(textOf(toggled)).not.toContain(RECOMPUTE);

    let current = toggled;
    for (const species of ["H2O", "CO2", "CO", "CH4", "SO2", "NH3"]) {
      click(withProp(current, "data-tso-bands")[0], species);
      current = show().tree;
    }
    expect(bandLabels()).toEqual([]);
    expect(button(current, USE_BANDS)?.props.disabled).toBe(true);
  });

  it("says when no bin has a depth", () => {
    state.spectrum = e1Spectrum({ rows: E1_BANDS.map((band, k) => row(`bin:${k}`, band, null, null, { nIn: 0 })) }, { depth: null, err: null, errScatter: null });
    const { tree, markup } = show();

    expect(textOf(tree)).toContain(NO_DEPTH);
    expect(markup).not.toContain("[plot]");
    expect(state.plot).toBeNull();
    const table = withProp(tree, "data-tso-depth-table");
    expect(table).toHaveLength(1);
    const body = elementsOf<Props>(table[0]).filter((element) => element.type === "td").map(textOf);
    expect(body.filter((cell) => cell === "--").length).toBeGreaterThanOrEqual(12);
    expect(body.slice(0, 5)).toEqual(["white light", "--", "--", "6 / 210", "--"]);
  });
});

describe("TsoDepthSpectrum export and log", () => {
  const SAVED = "C:/out/depth.csv";
  let append: MockInstance<typeof measurementLog.append>;

  const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

  beforeEach(() => {
    exportMocks.save.mockReset();
    exportMocks.csv.mockClear();
    exportMocks.drafts = 2;
    append = vi.spyOn(measurementLog, "append").mockImplementation(() => ({}) as never);
  });

  afterEach(() => append.mockRestore());

  function actions(tree: ReactNode): ReactElement<Props>[] {
    return withProp(tree, "data-tso-depth-actions");
  }

  function actionButton(tree: ReactNode, text: string): Props {
    const block = actions(tree);
    expect(block).toHaveLength(1);
    const found = elementsOf<Props>(block[0]).filter((element) => element.type === "button" && textOf(element) === text);
    expect(found).toHaveLength(1);
    return found[0].props;
  }

  it("offers Save CSV and Log under the depth table and disables them without a depth", () => {
    const { tree } = show();
    expect(actionButton(tree, "Save CSV").disabled).toBe(false);
    expect(actionButton(tree, "Log").disabled).toBe(false);
    const children = elementsOf<Props>(tree);
    expect(children.indexOf(actions(tree)[0])).toBeGreaterThan(children.indexOf(withProp(tree, "data-tso-depth-table")[0]));

    hooks.mount();
    state.spectrum = e1Spectrum({ rows: E1_BANDS.map((band, k) => row(`bin:${k}`, band, null, null)) });
    const { tree: whiteOnly } = show();
    expect(actionButton(whiteOnly, "Save CSV").disabled).toBe(false);

    hooks.mount();
    state.spectrum = e1Spectrum({ rows: E1_BANDS.map((band, k) => row(`bin:${k}`, band, null, null)) }, { depth: null, err: null });
    const { tree: none } = show();
    expect(actionButton(none, "Save CSV").disabled).toBe(true);
    expect(actionButton(none, "Log").disabled).toBe(true);

    hooks.mount();
    state.spectrum = e1Spectrum({ window: null });
    expect(actions(show().tree)).toEqual([]);

    hooks.mount();
    state.curves = { loading: false, result: null, error: null };
    expect(actions(show().tree)).toEqual([]);
  });

  it("saves the depth spectrum and logs one row per band with the saved path", async () => {
    exportMocks.save.mockResolvedValue(SAVED);
    const before = new Date().toISOString();
    (actionButton(show().tree, "Save CSV").onClick as () => void)();
    await flush();
    expect(exportMocks.save).toHaveBeenCalledTimes(1);
    expect(exportMocks.save).toHaveBeenCalledWith("DEPTH", "depth.csv", "Save depth spectrum");
    expect(exportMocks.csv).toHaveBeenCalledTimes(1);
    const [spectrum, curves, view, context] = exportMocks.csv.mock.calls[0] as unknown as [
      unknown,
      unknown,
      unknown,
      { software: string; exportedUtc: string; files: unknown },
    ];
    expect(spectrum).toBe(state.spectrum);
    expect(curves).toBe((state.curves as TsoCurvesState).result);
    expect(view).toBe(VIEW);
    expect(context.software).toBe(`AstroBurst ${APP_VERSION}`);
    expect(context.files).toEqual(e1Set().files);
    expect(context.exportedUtc >= before && context.exportedUtc <= new Date().toISOString()).toBe(true);
    expect(append.mock.calls).toEqual([[{ draft: "white", path: SAVED }], [{ draft: "bin", path: SAVED }]]);
    const { tree, markup } = show();
    expect(withProp(tree, "data-tso-saved").map(textOf)).toEqual([`Saved ${SAVED}`]);
    expect(markup).not.toContain('role="alert"');
    show();
    expect(append).toHaveBeenCalledTimes(2);

    exportMocks.save.mockResolvedValue(null);
    (actionButton(tree, "Save CSV").onClick as () => void)();
    await flush();
    expect(append).toHaveBeenCalledTimes(2);

    exportMocks.save.mockRejectedValue(new Error("disk full"));
    (actionButton(tree, "Save CSV").onClick as () => void)();
    await flush();
    expect(append).toHaveBeenCalledTimes(2);
    const { markup: failed } = show();
    expect(failed.match(/role="alert"/g)).toHaveLength(1);
    expect(failed).toContain('role="alert">Could not save the file: disk full</div>');
  });

  it("logs the depths without saving", () => {
    (actionButton(show().tree, "Log").onClick as () => void)();
    expect(exportMocks.save).not.toHaveBeenCalled();
    expect(append.mock.calls).toEqual([[{ draft: "white", path: null }], [{ draft: "bin", path: null }]]);
    const { tree } = show();
    expect(withProp(tree, "data-tso-logged").map(textOf)).toEqual(["Added 2 rows to the measurement log"]);

    exportMocks.drafts = 1;
    (actionButton(tree, "Log").onClick as () => void)();
    expect(append).toHaveBeenCalledTimes(3);
    expect(withProp(show().tree, "data-tso-logged").map(textOf)).toEqual(["Added 1 row to the measurement log"]);
  });
});

describe("TsoDepthSpectrum model fit", () => {
  type PlotSeries = { x: number[]; y: (number | null)[]; yErr?: (number | null)[]; label: string; color: string; mode: string };

  const FIT_HINT = "Fit the white light in the light-curve view above, then take its values here, or type the orbit.";
  const ORIGIN = "Values taken from the white-light fit.";
  const TAKE = "Take the white-light fit";
  const NO_WHITE_FIT = "No white-light fit yet: run the transit fit on the white light in the view above.";
  const UNUSABLE =
    "The white-light fit cannot be taken (an eclipse fit, no convergence or a value at its limit): fit the white light again in the view above.";
  const WHITE_STALE = "The white light changed since its transit fit: fit it again in the view above before taking its values.";
  const NO_TIME = "A transit fit needs a time axis: this series is on integration numbers.";
  const FIXED_CAPTION = "Period, t0, a / Rs and b stay fixed in every bin; Rp / Rs is fitted in every bin from this starting value.";
  const LIMB = "Fit u1 and u2 in every bin";
  const SCALE = "Scale errors by sqrt(reduced chi2)";
  const RUN = "Fit every bin";
  const STALE_FIT = "The light curves changed since this fit: fit again.";
  const SCALE_NOTE = "The two series are on different scales here (grazing transit): switch one off to read the other.";
  const QUICK_LABEL = "quick look, 1 - F_in / F_out";
  const FIT_LABEL = "model fit, (Rp/Rs)^2";
  const WHITE_LINE = { axis: "y", value: 0.9504, label: "white light", dashed: true };
  const REFUSAL =
    "no integration of this series falls inside the transit of this geometry (t0 1, period 3 d, a/Rs 8, b 0.3, Rp/Rs 0.1): check t0 and the period";
  const FIELDS: [string, string][] = [
    ["period (d)", "Orbital period in days"],
    ["t0 (BMJD (TDB))", "Mid-transit time"],
    ["a / Rs", "Scaled semi-major axis a over Rs"],
    ["b", "Impact parameter"],
    ["Rp / Rs start", "Starting radius ratio Rp over Rs"],
    ["u1", "Limb darkening u1"],
    ["u2", "Limb darkening u2"],
  ];
  const E1_TEXTS = ["1.4079405", "60061.507668", "336", "7.16", "7.73", "0.2", "0.25"];
  const WHITE_TEXTS = ["1.4079405", "60061.507668411155", "336", "7.16", "7.72564861863833", "0.2", "0.25"];
  const EMPTY_TEXTS = ["", "", "", "", "", "", ""];
  const REQ = curvesRequest.REQ as unknown as TsoLightCurvesRequest;
  const CURVES: TsoLightCurves = { ...curvesResult([[0.6, 1], [1, 2]]), row_index: Array.from({ length: 240 }, (_, k) => k) };
  const QUICK = e1Spectrum({ rows: e1Spectrum().rows.slice(0, 2) });
  const SIG = transitFitSignature(transitFitInput(lightCurveModel(WHITE, VIEW)));
  const E1_FIT: Required<TsoTransitFitRequest> = {
    period_days: 1.4079405,
    t0: 60061.507668,
    a_rs: 336,
    b: 7.16,
    rp_start: 7.73,
    u1: 0.2,
    u2: 0.25,
    limb_darkening: "fixed",
    baseline_order: 1,
    exclude_rows: [],
    scale_errors: false,
  };

  function parameter(name: TransitParameterName, value: number, err: number | null, free: boolean): TransitFittedParameter {
    return { name, value, err, free, at_bound: false };
  }

  const R: TransitFitResult = {
    model: "Mandel and Agol 2002, quadratic limb darkening, circular orbit",
    kind: "transit",
    period: 1.4079405,
    parameters: [
      parameter("t0", 60061.507668411155, 0.00001, true),
      parameter("rp", 7.72564861863833, 0.02, true),
      parameter("a_rs", 336, null, false),
      parameter("b", 7.16, null, false),
      parameter("u1", 0.2, null, false),
      parameter("u2", 0.25, null, false),
    ],
    baseline: { order: 1, reference_time: 60061.48, coefficients: [1, 0], errors: [null, null] },
    converged: true,
    depth: 59.68,
    depth_err: 0.3,
    chi2: 0,
    dof: 236,
    reduced_chi2: 0,
    weighted: true,
    error_scale: 1,
    rms: 0,
    rms_ppm: 0,
    n_points: 240,
    n_in_transit: 18,
    iterations: 5,
    free_names: ["t0", "rp", "c0", "c1"],
    time: [],
    model_flux: [],
    baseline_flux: [],
    residual: [],
    notes: [],
    warnings: [],
    curve_time: [],
    curve_flux: [],
    curve_baseline: [],
    correlation: null,
    derived: {
      inclination_deg: null,
      inclination_err: null,
      duration_t14_days: null,
      duration_t14_err: null,
      duration_t23_days: null,
      duration_t23_err: null,
      stellar_density_cgs: null,
      stellar_density_err: null,
    },
    time_averaging: { cadence_days: 0, bin_points: [], n_bins: [], rms_ppm: [], expected_ppm: [], beta: 1 },
    elapsed_ms: 0,
  };

  const WREC: TransitFitRecord = { setup: DEFAULT_TRANSIT_FIT_SETUP, request: null, result: R, signature: SIG, fittedUtc: "2026-11-13T00:00:00Z" };

  function fitBin(lo: number, hi: number, extra: Partial<TsoTransitBin> = {}): TsoTransitBin {
    return {
      band_um: [lo, hi],
      n_points: 240,
      n_in_transit: 18,
      weighted: true,
      rp: 0.1,
      rp_err: 0.001,
      depth: 0.01,
      depth_err: 0.0002,
      u1: 0.2,
      u1_err: null,
      u2: 0.25,
      u2_err: null,
      chi2: 250.5,
      dof: 237,
      rms: 0.0003,
      grazing: false,
      npixels_ratio: 1,
      pixel_loss: false,
      error: null,
      notes: [],
      warnings: [],
      ...extra,
    };
  }

  function fitted(bins: TsoTransitBin[]): TsoTransitSpectrum {
    return {
      set_id: "4b385da81d5c70b8",
      label: "NRS1",
      series: { detector: "NRS1", sporder: 0 },
      mode: "pipeline",
      time_scale: "bmjd_tdb",
      fit: E1_FIT,
      model: "Mandel and Agol 2002, quadratic limb darkening, circular orbit",
      n_integrations: 240,
      n_excluded: 0,
      n_in_transit: 18,
      bins,
      notes: ["n-fit"],
      warnings: ["w-fit"],
    };
  }

  const FITTED = fitted([fitBin(0.6, 1, { depth: 0.0102 }), fitBin(1, 2, { depth: 0.0098 })]);

  const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

  function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void; reject: (reason: unknown) => void } {
    let resolve: (value: T) => void = () => undefined;
    let reject: (reason: unknown) => void = () => undefined;
    const promise = new Promise<T>((done, fail) => {
      resolve = done;
      reject = fail;
    });
    return { promise, resolve, reject };
  }

  function block(tree: ReactNode): ReactElement<Props> {
    const found = withProp(tree, "data-tso-fit");
    expect(found).toHaveLength(1);
    return found[0];
  }

  function single(tree: ReactNode, name: string, value: unknown): Props {
    const found = elementsOf<Props>(tree).filter((element) => element.props[name] === value);
    expect(found).toHaveLength(1);
    return found[0].props;
  }

  function labelled(tree: ReactNode, text: string): Props {
    const label = elementsOf<Props>(tree).find((element) => element.type === "label" && textOf(element) === text);
    expect(label).toBeDefined();
    return single(tree, "id", label?.props.htmlFor);
  }

  function texts(tree: ReactNode): string[] {
    return FIELDS.map(([, aria]) => single(tree, "aria-label", aria).value as string);
  }

  function typeInto(aria: string, text: string): void {
    (single(show().tree, "aria-label", aria).onChange as (text: string) => void)(text);
  }

  function typeOrbit(values: string[] = E1_TEXTS): void {
    FIELDS.forEach(([, aria], k) => typeInto(aria, values[k]));
  }

  function flip(tree: ReactNode, label: string, on: boolean): void {
    (single(tree, "label", label).onChange as (on: boolean) => void)(on);
  }

  function run(tree: ReactNode): void {
    (single(tree, "label", RUN).onClick as () => void)();
  }

  async function fitWith(spectrum: TsoTransitSpectrum): Promise<{ tree: ReactNode; markup: string }> {
    transit.fn.mockResolvedValueOnce(spectrum);
    run(show().tree);
    await flush();
    return show();
  }

  function fitRows(tree: ReactNode): ReactElement<Props>[] {
    const table = withProp(tree, "data-tso-fit-table");
    expect(table).toHaveLength(1);
    const body = elementsOf<Props>(table[0]).filter((element) => element.type === "tbody");
    expect(body).toHaveLength(1);
    return elementsOf<Props>(body[0]).filter((element) => element.type === "tr");
  }

  function cells(tr: ReactElement<Props>): ReactElement<Props>[] {
    return elementsOf<Props>(tr).filter((element) => element.type === "td");
  }

  function plotProps(): Record<string, unknown> {
    return state.plot ?? {};
  }

  function plotted(): PlotSeries[] {
    return (plotProps().series as PlotSeries[]) ?? [];
  }

  function pressed(tree: ReactNode): [string, unknown][] {
    const row = withProp(tree, "data-tso-series-switches");
    expect(row).toHaveLength(1);
    return elementsOf<Props>(row[0])
      .filter((element) => element.type === "button")
      .map((element) => [textOf(element), element.props["aria-pressed"]]);
  }

  function switchTo(tree: ReactNode, text: string): void {
    click(withProp(tree, "data-tso-series-switches")[0], text);
  }

  function details(tree: ReactNode, name: string): { summary: string; lines: string[] } {
    const found = withProp(tree, name);
    expect(found).toHaveLength(1);
    expect(found[0].type).toBe("details");
    const inner = elementsOf<Props>(found[0]);
    return {
      summary: inner.filter((element) => element.type === "summary").map(textOf).join(""),
      lines: inner.filter((element) => element.type === "li").map(textOf),
    };
  }

  beforeEach(() => {
    state.curves = { loading: false, result: CURVES, error: null } satisfies TsoCurvesState;
    state.spectrum = QUICK;
    wfit.record = EMPTY_TRANSIT_FIT;
    transit.fn.mockReset();
    left.fn.mockReset();
    left.fn.mockImplementation(() => []);
    curvesRequest.getRequest.mockClear();
  });

  it("offers the model fit under the quick look and waits for an orbit", () => {
    const { tree, markup } = show();
    const fit = block(tree);
    const all = elementsOf<Props>(tree);
    expect(all.indexOf(fit)).toBeGreaterThan(all.indexOf(withProp(tree, "data-tso-depth-table")[0]));
    expect(all.indexOf(fit)).toBeGreaterThan(all.indexOf(withProp(tree, "data-tso-depth-actions")[0]));
    expect(fit.props.className).toBe("space-y-2");
    expect(textOf(fit)).toContain("Model fit");
    expect(withProp(fit, "data-tso-fit-label").map(textOf)).toEqual([TRANSIT_FIT_LABEL]);
    expect(withProp(fit, "data-tso-fit-label")[0].props.className).toBe("text-[9px] text-zinc-500");
    expect(withProp(fit, "data-tso-fit-hint").map(textOf)).toEqual([FIT_HINT]);
    expect(withProp(fit, "data-tso-fit-origin")).toEqual([]);
    for (const [label, aria] of FIELDS) {
      expect(labelled(fit, label)).toMatchObject({ "aria-label": aria, value: "", optional: true });
    }
    expect(textOf(fit)).toContain(FIXED_CAPTION);
    expect(single(fit, "label", LIMB).checked).toBe(false);
    expect(single(fit, "label", SCALE).checked).toBe(false);
    expect(labelled(fit, "Baseline")).toMatchObject({ "aria-label": "Baseline polynomial", value: "1" });
    const options = elementsOf<Props>(fit).filter((element) => element.type === "option");
    expect(options.map(textOf)).toEqual(["constant", "linear", "quadratic"]);
    expect(options.map((option) => option.props.value)).toEqual(["0", "1", "2"]);
    expect(button(fit, TAKE)?.props).toMatchObject({ disabled: true, title: NO_WHITE_FIT });
    expect(withProp(fit, "data-tso-fit-white-stale")).toEqual([]);
    expect(single(fit, "label", RUN)).toMatchObject({ disabled: true, running: false, runningLabel: "Fitting every bin" });
    expect(withProp(tree, "data-field-error")).toEqual([]);
    expect(withProp(tree, "data-tso-fit-left-out")).toEqual([]);
    expect(withProp(tree, "data-tso-fit-table")).toEqual([]);
    expect(withProp(tree, "data-tso-series-switches")).toEqual([]);
    expect(markup).not.toContain('role="alert"');
    expect(plotted().map((line) => line.label)).toEqual(["depth"]);

    hooks.mount();
    const indexTree = hooks.render(section, { ...props(), series: { ...e1Series(), time_scale: "index" } }) as ReactNode;
    const indexBlock = block(indexTree);
    expect(textOf(indexBlock)).toContain("Model fit");
    expect(withProp(indexBlock, "data-tso-fit-label").map(textOf)).toEqual([TRANSIT_FIT_LABEL]);
    expect(withProp(indexBlock, "data-tso-fit-no-time").map(textOf)).toEqual([NO_TIME]);
    expect(withProp(indexBlock, "aria-label")).toEqual([]);
    expect(withProp(indexBlock, "runningLabel")).toEqual([]);
    expect(button(indexBlock, TAKE)).toBeUndefined();
    expect(textOf(indexBlock)).not.toContain(FIT_HINT);

    hooks.mount();
    state.curves = { loading: false, result: curvesResult([]), error: null };
    const { tree: noBins } = show();
    expect(withProp(noBins, "data-tso-fit")).toEqual([]);
    expect(textOf(noBins)).toContain(HINT);
  });

  it("takes the white-light fit once on its own and again on a click, and refuses one it cannot use", () => {
    wfit.record = WREC;
    show();
    const { tree } = show();
    expect(texts(tree)).toEqual(WHITE_TEXTS);
    expect(withProp(tree, "data-tso-fit-origin").map(textOf)).toEqual([ORIGIN]);
    expect(withProp(tree, "data-tso-fit-hint")).toEqual([]);
    expect(withProp(tree, "data-tso-fit-white-stale")).toEqual([]);
    expect(button(tree, TAKE)?.props.disabled).toBe(false);

    flip(tree, SCALE, true);
    const { tree: scaled } = show();
    expect(single(scaled, "label", SCALE).checked).toBe(true);
    expect(withProp(scaled, "data-tso-fit-origin").map(textOf)).toEqual([ORIGIN]);

    typeInto("Impact parameter", "7.2");
    const typedTexts = WHITE_TEXTS.map((text, k) => (k === 3 ? "7.2" : text));
    const { tree: typed } = show();
    expect(texts(typed)).toEqual(typedTexts);
    expect(withProp(typed, "data-tso-fit-origin")).toEqual([]);

    wfit.record = { ...WREC, result: { ...R } };
    show();
    const { tree: kept } = show();
    expect(texts(kept)).toEqual(typedTexts);
    expect(withProp(kept, "data-tso-fit-origin")).toEqual([]);

    click(kept, TAKE);
    const { tree: taken } = show();
    expect(texts(taken)).toEqual(WHITE_TEXTS);
    expect(withProp(taken, "data-tso-fit-origin").map(textOf)).toEqual([ORIGIN]);
    expect(single(taken, "label", SCALE).checked).toBe(true);

    for (const [, aria] of FIELDS) typeInto(aria, "");
    show();
    const { tree: emptied } = show();
    expect(texts(emptied)).toEqual(EMPTY_TEXTS);
    expect(withProp(emptied, "data-tso-fit-hint").map(textOf)).toEqual([FIT_HINT]);
    expect(withProp(emptied, "data-tso-fit-origin")).toEqual([]);

    show(OTHER_GROUP);
    const { tree: elsewhere } = show(OTHER_GROUP);
    expect(texts(elsewhere)).toEqual(WHITE_TEXTS);
    show();
    const { tree: back } = show();
    expect(texts(back)).toEqual(WHITE_TEXTS);
    expect(single(back, "label", SCALE).checked).toBe(false);

    hooks.mount();
    wfit.record = { ...WREC, result: { ...R, converged: false } };
    show();
    const { tree: refused } = show();
    expect(button(refused, TAKE)?.props).toMatchObject({ disabled: true, title: UNUSABLE });
    expect(texts(refused)).toEqual(EMPTY_TEXTS);
    click(refused, TAKE);
    expect(texts(show().tree)).toEqual(EMPTY_TEXTS);

    hooks.mount();
    wfit.record = { ...WREC, signature: "old" };
    show();
    const { tree: stale } = show();
    expect(withProp(stale, "data-tso-fit-white-stale").map(textOf)).toEqual([WHITE_STALE]);
    expect(withProp(stale, "data-tso-fit-white-stale")[0].props.className).toBe("text-[9px] text-amber-400/90");
    expect(texts(stale)).toEqual(EMPTY_TEXTS);
    expect(button(stale, TAKE)?.props.disabled).toBe(false);
    click(stale, TAKE);
    const { tree: forced } = show();
    expect(texts(forced)).toEqual(WHITE_TEXTS);
    expect(withProp(forced, "data-tso-fit-origin").map(textOf)).toEqual([ORIGIN]);
  });

  it("fits every bin with the curve request of the store, the typed orbit and the rows left out", async () => {
    left.fn.mockImplementation(() => [3, 7]);
    typeOrbit();
    const { tree } = show();
    expect(withProp(tree, "data-tso-fit-left-out").map(textOf)).toEqual([
      "2 integration(s) left out in the light-curve view are left out of every bin.",
    ]);
    expect(left.fn).toHaveBeenCalledWith(WHITE, VIEW, CURVES.row_index);
    expect(withProp(tree, "data-tso-fit-hint")).toEqual([]);
    expect(withProp(tree, "data-field-error")).toEqual([]);
    expect(single(tree, "label", RUN).disabled).toBe(false);

    const sizeBefore = transitFitStore.size();
    const pending = deferred<TsoTransitSpectrum>();
    transit.fn.mockReturnValueOnce(pending.promise);
    run(tree);
    expect(curvesRequest.getRequest).toHaveBeenCalledWith(GROUP);
    expect(transit.fn).toHaveBeenCalledTimes(1);
    expect(transit.fn).toHaveBeenCalledWith([PATH], { detector: "NRS1", sporder: 0 }, REQ, { ...E1_FIT, exclude_rows: [3, 7] });
    expect(transit.fn.mock.calls[0][2]).toBe(curvesRequest.REQ);
    expect(curvesRequest.getRequest.mock.invocationCallOrder[0]).toBeLessThan(transit.fn.mock.invocationCallOrder[0]);
    const { tree: running, markup } = show();
    expect(single(running, "label", RUN).running).toBe(true);
    expect(markup).toContain("Fitting every bin");
    pending.resolve(FITTED);
    await flush();
    const { tree: done } = show();
    expect(single(done, "label", RUN).running).toBe(false);
    expect(withProp(done, "data-tso-fit-table")).toHaveLength(1);
    expect(transitFitStore.size()).toBe(sizeBefore);

    flip(done, LIMB, true);
    (labelled(show().tree, "Baseline").onChange as (event: { target: { value: string } }) => void)({ target: { value: "2" } });
    const { tree: free } = show();
    expect(single(free, "label", LIMB).checked).toBe(true);
    expect(labelled(free, "Baseline").value).toBe("2");
    transit.fn.mockResolvedValueOnce(FITTED);
    run(free);
    expect(transit.fn).toHaveBeenLastCalledWith([PATH], { detector: "NRS1", sporder: 0 }, REQ, {
      ...E1_FIT,
      limb_darkening: "free",
      baseline_order: 2,
      exclude_rows: [3, 7],
    });
    await flush();

    flip(show().tree, SCALE, true);
    transit.fn.mockResolvedValueOnce(FITTED);
    run(show().tree);
    expect(transit.fn).toHaveBeenLastCalledWith([PATH], { detector: "NRS1", sporder: 0 }, REQ, {
      ...E1_FIT,
      limb_darkening: "free",
      baseline_order: 2,
      exclude_rows: [3, 7],
      scale_errors: true,
    });
    await flush();
    expect(transit.fn).toHaveBeenCalledTimes(3);
    expect(curvesRequest.getRequest.mock.calls.every(([group]) => group === GROUP)).toBe(true);
    expect(transitFitStore.size()).toBe(sizeBefore);
  });

  it("blocks an orbit that cannot be read and says why", () => {
    typeOrbit(E1_TEXTS.map((text, k) => (k === 3 ? "400" : text)));
    const { tree } = show();
    expect(single(tree, "label", RUN).disabled).toBe(true);
    expect(withProp(tree, "data-field-error").map(textOf)).toEqual(["b must be below a/Rs"]);
    expect(withProp(tree, "data-field-error")[0].props.className).toBe("text-[9px] text-amber-400/90");
    run(tree);

    typeInto("Impact parameter", "7.16");
    typeInto("Limb darkening u1", "0,2");
    const { tree: comma } = show();
    expect(single(comma, "label", RUN).disabled).toBe(true);
    expect(withProp(comma, "data-field-error").map(textOf)).toEqual(["u1 and u2 must be numbers"]);
    run(comma);
    expect(transit.fn).not.toHaveBeenCalled();
  });

  it("draws the fitted spectrum in the plot of the quick look, both labelled, with a switch for each", async () => {
    typeOrbit();
    const { tree, markup } = await fitWith(FITTED);
    expect(plotted()).toHaveLength(2);
    expect(plotted()[0]).toMatchObject({ label: QUICK_LABEL, x: [0.8, 1.5], y: [0.9674, 0.9833], color: "#fbbf24" });
    expect(plotted()[1]).toEqual({ x: [0.8, 1.5], y: [0.0102, 0.0098], yErr: [0.0002, 0.0002], color: "#67e8f9", label: FIT_LABEL, mode: "points" });
    expect(plotProps()).toMatchObject({ yLabel: "depth", xLabel: "wavelength (um)", height: 170, toolbar: true, logToggle: false, csvName: "tso_depth_spectrum" });
    expect(plotProps().referenceLines).toEqual([WHITE_LINE]);
    expect(bandLabels()).toEqual(["H2O 2.7", "CO2 4.3", "CO 4.7", "CH4 3.3"]);
    expect(textOf(withProp(tree, "data-tso-series-switches")[0])).toContain("Show");
    expect(pressed(tree)).toEqual([
      ["quick look", true],
      ["model fit", true],
    ]);
    expect(withProp(tree, "data-tso-scale-note")).toEqual([]);

    switchTo(tree, "quick look");
    const { tree: fitOnly } = show();
    expect(plotted().map((line) => line.label)).toEqual([FIT_LABEL]);
    expect(plotProps().yLabel).toBe("(Rp/Rs)^2");
    expect(plotProps().referenceLines).toEqual([]);
    expect(pressed(fitOnly)).toEqual([
      ["quick look", false],
      ["model fit", true],
    ]);

    switchTo(fitOnly, "model fit");
    const { tree: still } = show();
    expect(plotted().map((line) => line.label)).toEqual([FIT_LABEL]);
    expect(pressed(still)).toEqual([
      ["quick look", false],
      ["model fit", true],
    ]);

    switchTo(still, "quick look");
    const { tree: both } = show();
    expect(plotted().map((line) => line.label)).toEqual([QUICK_LABEL, FIT_LABEL]);
    expect(plotProps().yLabel).toBe("depth");

    switchTo(both, "model fit");
    const { tree: quickOnly } = show();
    expect(plotted().map((line) => line.label)).toEqual([QUICK_LABEL]);
    expect(plotProps().yLabel).toBe("depth (1 - F_in / F_out)");
    expect(plotProps().referenceLines).toEqual([WHITE_LINE]);
    switchTo(quickOnly, "model fit");
    show();
    expect(plotted().map((line) => line.label)).toEqual([QUICK_LABEL, FIT_LABEL]);

    const fit = block(tree);
    expect(withProp(fit, "data-tso-fit-fixed").map(textOf)).toEqual([
      "Fixed in every bin: period 1.4079405 d, t0 60061.507668 (BMJD (TDB)), a/Rs 336, b 7.16, u1 0.2, u2 0.25; linear baseline",
    ]);
    expect(withProp(fit, "warnings").map((element) => element.props.warnings)).toEqual([["w-fit"]]);
    expect(markup).toContain("w-fit");
    expect(details(fit, "data-tso-fit-notes")).toEqual({ summary: "notes (1)", lines: ["n-fit"] });
    const table = withProp(fit, "data-tso-fit-table");
    expect(elementsOf<Props>(table[0]).filter((element) => element.type === "th").map(textOf)).toEqual([
      "band",
      "(Rp/Rs)^2",
      "error",
      "Rp/Rs",
      "error",
      "chi2 / dof",
      "in transit / points",
      "quick look",
      "remark",
    ]);
    const shown = fitRows(tree).map((tr) => cells(tr).map(textOf));
    const expected = fitTableRows(FITTED, QUICK).map((row) => [
      row.band,
      row.depth,
      row.depthErr,
      row.rp,
      row.rpErr,
      row.chi2,
      row.points,
      row.quick,
      row.remark,
    ]);
    expect(shown).toEqual(expected);
    expect(shown[0]).toEqual(["0.6 to 1 um", "0.0102", "0.0002", "0.1", "0.001", "250.5 / 237", "18 / 240", "0.9674", ""]);
    expect(shown[1]).toEqual(["1 to 2 um", "0.0098", "0.0002", "0.1", "0.001", "250.5 / 237", "18 / 240", "0.9833", ""]);
    expect(fitRows(tree).map((tr) => String(cells(tr)[8].props.className))).not.toContainEqual(expect.stringContaining("amber"));
    expect(withProp(tree, "data-tso-fit-warnings")).toEqual([]);
    expect(withProp(tree, "data-tso-fit-stale")).toEqual([]);
    expect(withProp(tree, "data-tso-fit-hint")).toEqual([]);
  });

  it("says when the two series cannot share a scale, lists the warnings of each bin, and draws the fit without a quick-look window", async () => {
    typeOrbit();
    const { tree: grazing } = await fitWith(
      fitted([fitBin(0.6, 1, { depth: 59.29, grazing: true }), fitBin(1, 2, { depth: 63.39, grazing: true })]),
    );
    expect(withProp(grazing, "data-tso-scale-note").map(textOf)).toEqual([SCALE_NOTE]);
    expect(String(withProp(grazing, "data-tso-scale-note")[0].props.className)).toContain("amber");
    const remarks = fitRows(grazing).map((tr) => cells(tr)[8]);
    expect(remarks.map(textOf)).toEqual(["grazing", "grazing"]);
    for (const remark of remarks) expect(String(remark.props.className)).toContain("amber");
    expect(plotted().map((line) => line.label)).toEqual([QUICK_LABEL, FIT_LABEL]);

    const { tree: warned } = await fitWith(fitted([fitBin(0.6, 1, { depth: 0.0102, warnings: ["w-bin"] }), fitBin(1, 2, { depth: 0.0098 })]));
    expect(withProp(warned, "data-tso-scale-note")).toEqual([]);
    const rows = fitRows(warned);
    expect(textOf(cells(rows[0])[8])).toBe("fit warnings");
    expect(String(cells(rows[0])[8].props.className)).toContain("amber");
    expect(String(cells(rows[1])[8].props.className)).not.toContain("amber");
    expect(details(warned, "data-tso-fit-warnings")).toEqual({ summary: "fit warnings (1)", lines: ["0.6 to 1 um: w-bin"] });

    const { tree: many } = await fitWith(
      fitted([fitBin(1, 2, { warnings: ["b1", "b2"] }), fitBin(0.6, 1, { warnings: ["a1"] })]),
    );
    expect(details(many, "data-tso-fit-warnings")).toEqual({
      summary: "fit warnings (3)",
      lines: ["0.6 to 1 um: a1", "1 to 2 um: b1", "1 to 2 um: b2"],
    });

    const { tree: broken } = await fitWith(
      fitted([fitBin(0.6, 1, { depth: 0.0102 }), fitBin(1, 2, { depth: null, depth_err: null, rp: null, rp_err: null, chi2: null, error: "no integration has a flux in this band" })]),
    );
    const brokenRows = fitRows(broken);
    expect(textOf(cells(brokenRows[1])[8])).toBe("no integration has a flux in this band");
    expect(String(cells(brokenRows[1])[8].props.className)).toContain("amber");
    expect(String(cells(brokenRows[0])[8].props.className)).not.toContain("amber");
    expect(withProp(broken, "data-tso-fit-warnings")).toEqual([]);

    hooks.mount();
    state.spectrum = e1Spectrum({ window: null });
    state.plot = null;
    typeOrbit();
    const { tree: noWindow } = await fitWith(FITTED);
    expect(withProp(noWindow, "data-tso-no-window").map(textOf)).toEqual([NO_WINDOW]);
    expect(plotted()).toHaveLength(1);
    expect(plotted()[0].label).toBe(FIT_LABEL);
    expect(plotProps().yLabel).toBe("(Rp/Rs)^2");
    expect(plotProps().referenceLines).toEqual([]);
    expect(withProp(noWindow, "data-tso-bands")).toHaveLength(1);
    expect(button(noWindow, USE_BANDS)).toBeDefined();
    expect(withProp(noWindow, "data-tso-series-switches")).toEqual([]);
    expect(withProp(noWindow, "data-tso-depth-table")).toEqual([]);
    expect(withProp(noWindow, "data-tso-depth-actions")).toEqual([]);
    expect(fitRows(noWindow).map((tr) => textOf(cells(tr)[7]))).toEqual(["--", "--"]);
    expect(textOf(noWindow)).not.toContain(NO_DEPTH);

    hooks.mount();
    state.spectrum = QUICK;
    typeOrbit();
    const { tree: bothShown } = await fitWith(FITTED);
    switchTo(bothShown, "model fit");
    show();
    expect(plotted().map((line) => line.label)).toEqual([QUICK_LABEL]);
    state.spectrum = e1Spectrum({ rows: QUICK.rows.map((quickRow) => ({ ...quickRow, depth: null, err: null })) });
    state.view = { ...VIEW };
    state.plot = null;
    const { tree: noDepth } = show();
    expect(textOf(noDepth)).not.toContain(NO_DEPTH);
    expect(plotted().map((line) => line.label)).toEqual([FIT_LABEL]);
    expect(withProp(noDepth, "data-tso-series-switches")).toEqual([]);
    expect(fitRows(noDepth).map((tr) => textOf(cells(tr)[7]))).toEqual(["--", "--"]);
  });

  it("shows a failed fit, drops a late answer and marks a fit of older curves", async () => {
    typeOrbit();
    transit.fn.mockRejectedValueOnce(new Error(REFUSAL));
    run(show().tree);
    await flush();
    const { tree: failed, markup } = show();
    expect(markup.match(/role="alert"/g)).toHaveLength(1);
    expect(markup).toContain(`role="alert">${REFUSAL}</div>`);
    expect(withProp(failed, "data-tso-fit-table")).toEqual([]);
    expect(single(failed, "label", RUN)).toMatchObject({ running: false, disabled: false });

    const first = deferred<TsoTransitSpectrum>();
    const second = deferred<TsoTransitSpectrum>();
    transit.fn.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const { tree } = show();
    run(tree);
    run(tree);
    const { tree: pending, markup: pendingMarkup } = show();
    expect(pendingMarkup).not.toContain('role="alert"');
    expect(single(pending, "label", RUN).running).toBe(true);
    second.resolve(fitted([fitBin(0.6, 1, { depth: 0.0111 }), fitBin(1, 2, { depth: 0.0098 })]));
    await flush();
    first.resolve(FITTED);
    await flush();
    const { tree: latest, markup: latestMarkup } = show();
    expect(fitRows(latest).map((tr) => textOf(cells(tr)[1]))).toEqual(["0.0111", "0.0098"]);
    expect(latestMarkup).not.toContain('role="alert"');

    const third = deferred<TsoTransitSpectrum>();
    transit.fn.mockReturnValueOnce(third.promise).mockResolvedValueOnce(fitted([fitBin(0.6, 1, { depth: 0.0111 }), fitBin(1, 2, { depth: 0.0098 })]));
    run(latest);
    run(latest);
    await flush();
    third.reject(new Error("an older refusal"));
    await flush();
    const { tree: afterLate, markup: afterLateMarkup } = show();
    expect(afterLateMarkup).not.toContain('role="alert"');
    expect(fitRows(afterLate).map((tr) => textOf(cells(tr)[1]))).toEqual(["0.0111", "0.0098"]);
    expect(single(afterLate, "label", RUN).running).toBe(false);

    transit.fn.mockRejectedValueOnce("the series is gone");
    run(latest);
    await flush();
    const { tree: kept, markup: keptMarkup } = show();
    expect(keptMarkup).toContain('role="alert">the series is gone</div>');
    expect(fitRows(kept).map((tr) => textOf(cells(tr)[1]))).toEqual(["0.0111", "0.0098"]);

    switchTo(kept, "quick look");
    show();
    expect(plotted().map((line) => line.label)).toEqual([FIT_LABEL]);
    state.curves = { loading: false, result: { ...CURVES }, error: null };
    state.plot = null;
    const { tree: older } = show();
    expect(withProp(older, "data-tso-fit-stale").map(textOf)).toEqual([STALE_FIT]);
    expect(String(withProp(older, "data-tso-fit-stale")[0].props.className)).toContain("amber");
    expect(withProp(older, "data-tso-fit-table")).toEqual([]);
    expect(withProp(older, "data-tso-fit-fixed")).toEqual([]);
    expect(withProp(older, "data-tso-fit-notes")).toEqual([]);
    expect(withProp(older, "data-tso-series-switches")).toEqual([]);
    expect(plotted().map((line) => line.label)).toEqual(["depth"]);
    expect(plotProps().yLabel).toBe("depth (1 - F_in / F_out)");

    hooks.mount();
    state.curves = { loading: false, result: CURVES, error: null };
    typeOrbit();
    const old = deferred<TsoTransitSpectrum>();
    transit.fn.mockReturnValueOnce(old.promise);
    run(show().tree);
    expect(single(show().tree, "label", RUN).running).toBe(true);
    const { tree: moved } = show(OTHER_GROUP);
    expect(single(moved, "label", RUN).running).toBe(false);
    expect(texts(moved)).toEqual(EMPTY_TEXTS);
    old.resolve(FITTED);
    await flush();
    expect(withProp(show(OTHER_GROUP).tree, "data-tso-fit-table")).toEqual([]);
    const { tree: returned } = show();
    expect(withProp(returned, "data-tso-fit-table")).toEqual([]);
    expect(single(returned, "label", RUN).running).toBe(false);
    expect(texts(returned)).toEqual(EMPTY_TEXTS);
  });

  describe("fit export and log", () => {
    const FIT_SAVED = "C:/out/fit.csv";
    let append: MockInstance<typeof measurementLog.append>;

    beforeEach(() => {
      exportMocks.save.mockReset();
      exportMocks.csv.mockClear();
      fitExports.csvArgs = null;
      fitExports.nameArgs = null;
      fitExports.entriesArgs = null;
      fitExports.drafts = 2;
      append = vi.spyOn(measurementLog, "append").mockImplementation(() => ({}) as never);
    });

    afterEach(() => append.mockRestore());

    function fitActions(tree: ReactNode): ReactElement<Props>[] {
      return withProp(tree, "data-tso-fit-actions");
    }

    function fitActionButton(tree: ReactNode, text: string): Props {
      const found = fitActions(tree);
      expect(found).toHaveLength(1);
      const buttons = elementsOf<Props>(found[0]).filter((element) => element.type === "button" && textOf(element) === text);
      expect(buttons).toHaveLength(1);
      return buttons[0].props;
    }

    it("offers Save CSV and Log under the fit table only for a fit of the curves on screen", async () => {
      typeOrbit();
      expect(fitActions(show().tree)).toEqual([]);
      const { tree } = await fitWith(FITTED);
      expect(fitActionButton(tree, "Save CSV").disabled).toBe(false);
      expect(fitActionButton(tree, "Log").disabled).toBe(false);
      expect(withProp(block(tree), "data-tso-fit-actions")).toHaveLength(1);
      const all = elementsOf<Props>(tree);
      const at = all.indexOf(fitActions(tree)[0]);
      expect(at).toBeGreaterThan(all.indexOf(withProp(tree, "data-tso-fit-table")[0]));
      expect(at).toBeGreaterThan(all.indexOf(withProp(tree, "data-tso-fit-notes")[0]));
      const quickActions = withProp(tree, "data-tso-depth-actions");
      expect(quickActions).toHaveLength(1);
      expect(elementsOf<Props>(quickActions[0]).filter((element) => element.type === "button").map(textOf)).toEqual(["Save CSV", "Log"]);

      const pending = deferred<TsoTransitSpectrum>();
      transit.fn.mockReturnValueOnce(pending.promise);
      run(tree);
      const { tree: running } = show();
      expect(fitActionButton(running, "Save CSV").disabled).toBe(true);
      expect(fitActionButton(running, "Log").disabled).toBe(true);
      pending.resolve(fitted([fitBin(0.6, 1, { depth: null, depth_err: null }), fitBin(1, 2, { depth: null, depth_err: null })]));
      await flush();
      const { tree: empty } = show();
      expect(fitActionButton(empty, "Save CSV").disabled).toBe(true);
      expect(fitActionButton(empty, "Log").disabled).toBe(true);

      state.curves = { loading: false, result: { ...CURVES }, error: null };
      const { tree: stale } = show();
      expect(withProp(stale, "data-tso-fit-stale")).toHaveLength(1);
      expect(fitActions(stale)).toEqual([]);
      expect(withProp(stale, "data-tso-depth-actions")).toHaveLength(1);
    });

    it("saves the fitted spectrum with the quick look beside it and logs one row per band with the saved path", async () => {
      typeOrbit();
      const { tree } = await fitWith(FITTED);

      exportMocks.save.mockResolvedValue(null);
      (fitActionButton(tree, "Save CSV").onClick as () => void)();
      await flush();
      expect(exportMocks.save).toHaveBeenCalledTimes(1);
      expect(append).not.toHaveBeenCalled();
      const { tree: cancelled, markup: cancelledMarkup } = show();
      expect(withProp(cancelled, "data-tso-fit-saved")).toEqual([]);
      expect(cancelledMarkup).not.toContain('role="alert"');

      exportMocks.save.mockRejectedValue(new Error("disk full"));
      (fitActionButton(tree, "Save CSV").onClick as () => void)();
      await flush();
      expect(append).not.toHaveBeenCalled();
      const { tree: failed, markup: failedMarkup } = show();
      expect(failedMarkup.match(/role="alert"/g)).toHaveLength(1);
      expect(failedMarkup).toContain('role="alert">Could not save the file: disk full</div>');
      expect(withProp(fitActions(failed)[0], "message").map((element) => element.props.message)).toEqual(["Could not save the file: disk full"]);
      expect(withProp(failed, "data-tso-fit-saved")).toEqual([]);

      exportMocks.save.mockReset();
      exportMocks.save.mockResolvedValue(FIT_SAVED);
      const before = new Date().toISOString();
      (fitActionButton(failed, "Save CSV").onClick as () => void)();
      await flush();
      expect(exportMocks.save).toHaveBeenCalledTimes(1);
      expect(exportMocks.save).toHaveBeenCalledWith("FIT", "fit.csv", "Save fitted spectrum");
      expect(exportMocks.csv).not.toHaveBeenCalled();
      const [spectrum, quick, context] = fitExports.csvArgs as [unknown, unknown, { software: string; exportedUtc: string; files: unknown }];
      expect(spectrum).toBe(FITTED);
      expect(quick).toBe(QUICK);
      expect(context.software).toBe(`AstroBurst ${APP_VERSION}`);
      expect(context.files).toEqual(e1Set().files);
      expect(context.exportedUtc >= before && context.exportedUtc <= new Date().toISOString()).toBe(true);
      expect(fitExports.nameArgs).toEqual([FITTED, "e1_x1dints.fits"]);
      expect(fitExports.entriesArgs?.[0]).toBe(FITTED);
      expect(fitExports.entriesArgs?.slice(1)).toEqual([e1Set().files, FIT_SAVED]);
      expect(append.mock.calls).toEqual([[{ draft: "bin0", path: FIT_SAVED }], [{ draft: "bin1", path: FIT_SAVED }]]);
      const { tree: saved, markup } = show();
      expect(withProp(saved, "data-tso-fit-saved").map(textOf)).toEqual([`Saved ${FIT_SAVED}`]);
      expect(withProp(saved, "data-tso-saved")).toEqual([]);
      expect(markup).not.toContain('role="alert"');
      show();
      expect(append).toHaveBeenCalledTimes(2);
    });

    it("logs the fitted bands without saving", async () => {
      typeOrbit();
      const { tree } = await fitWith(FITTED);
      (fitActionButton(tree, "Log").onClick as () => void)();
      expect(exportMocks.save).not.toHaveBeenCalled();
      expect(fitExports.entriesArgs?.[0]).toBe(FITTED);
      expect(fitExports.entriesArgs?.slice(1)).toEqual([e1Set().files, null]);
      expect(append.mock.calls).toEqual([[{ draft: "bin0", path: null }], [{ draft: "bin1", path: null }]]);
      const { tree: logged } = show();
      expect(withProp(logged, "data-tso-fit-logged").map(textOf)).toEqual(["Added 2 rows to the measurement log"]);
      expect(withProp(logged, "data-tso-logged")).toEqual([]);

      fitExports.drafts = 1;
      (fitActionButton(logged, "Log").onClick as () => void)();
      expect(append).toHaveBeenCalledTimes(3);
      expect(withProp(show().tree, "data-tso-fit-logged").map(textOf)).toEqual(["Added 1 row to the measurement log"]);
    });
  });
});
