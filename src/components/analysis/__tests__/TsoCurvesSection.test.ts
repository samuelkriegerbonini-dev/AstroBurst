import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from "vitest";
import type { ReactElement, ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { TsoCurvesDraft, TsoCurvesState, TsoSectionProps } from "../../../hooks/useTsoStore";
import type { LightCurveDoc } from "../../../shared/types/lightCurve";
import type { TsoCurve, TsoFile, TsoLightCurves, TsoLightCurvesRequest, TsoOpenResult, TsoSeriesSummary } from "../../../shared/types/tso";

const GROUP = "tso:4b385da81d5c70b8:NRS1:0";

const state = vi.hoisted(() => ({
  draft: null as TsoCurvesDraft | null,
  curves: { loading: false, result: null, error: null } as TsoCurvesState,
  request: null as TsoLightCurvesRequest | null,
  keys: [] as (string | null)[],
  patchDraft: vi.fn(),
  compute: vi.fn(),
}));
const view = vi.hoisted(() => ({ props: null as Record<string, unknown> | null }));
const selection = vi.hoisted(() => ({ fn: vi.fn() }));

vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../hooks/useTsoStore", async (original) => {
  const actual = await original<typeof import("../../../hooks/useTsoStore")>();
  return {
    DEFAULT_TSO_DRAFT: actual.DEFAULT_TSO_DRAFT,
    tsoStore: {
      patchDraft: state.patchDraft,
      compute: state.compute,
      getRequest: (group: string) => (group === "tso:4b385da81d5c70b8:NRS1:0" ? state.request : null),
    },
    useTsoDraft: (group: string | null) => {
      state.keys.push(group);
      return group !== null && state.draft ? state.draft : actual.DEFAULT_TSO_DRAFT;
    },
    useTsoCurves: (group: string | null) => {
      state.keys.push(group);
      return group !== null ? state.curves : { loading: false, result: null, error: null };
    },
  };
});
vi.mock("../LightCurveView", () => ({
  default: (props: Record<string, unknown>) => {
    view.props = props;
    return "[view]";
  },
}));
vi.mock("../../../hooks/useActiveSpectrumDocument", () => ({ spectrumSelectionStore: { set: selection.fn } }));
const save = vi.hoisted(() => ({ fn: vi.fn(), csv: vi.fn(() => "CSV") }));
vi.mock("../../../utils/spectrumExport", async (original) => ({
  ...(await original<typeof import("../../../utils/spectrumExport")>()),
  saveCsvDialog: save.fn,
}));
vi.mock("../../../utils/tsoExports", () => ({ tsoCurvesCsv: save.csv, tsoCurvesCsvFileName: () => "name.csv" }));
vi.mock("../../../utils/measurementLogTso", () => ({
  tsoLightCurveEntry: (_result: unknown, _files: unknown, path: string | null) => ({ draft: "curves", path }),
}));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import { DEFAULT_TSO_DRAFT } from "../../../hooks/useTsoStore";
import { equalEdgesText } from "../../../utils/tsoCurveInputs";
import { measurementLog } from "../../../utils/measurementLog";
import { APP_VERSION } from "../../../utils/constants";

const { default: TsoCurvesSection } = await import("../TsoCurvesSection");

type Section = (props: TsoSectionProps) => ReactNode;
const section = (TsoCurvesSection as unknown as { type: Section }).type;

type Props = Record<string, unknown>;

const E1_PATH = "C:/d/e1_x1dints.fits";
const DEFAULT_REQUEST: TsoLightCurvesRequest = {
  mode: "pipeline",
  wavelength_min_um: null,
  wavelength_max_um: null,
  bins: [],
  exclude_um: [],
  reject_elements_sigma: null,
};
const STALE_LINE = "These curves were computed with other settings: compute again to apply the changes.";
const HINT = "Compute the light curves of this series to see the white light and the bins.";
const DOC = { id: "white" } as LightCurveDoc;

function tsoFile(): TsoFile {
  return {
    path: E1_PATH,
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
  return { set_id: "4b385da81d5c70b8", files: [tsoFile()], series: [e1Series()], notes: [], warnings: [] };
}

function curve(flux: (number | null)[], extra: Partial<TsoCurve> = {}): TsoCurve {
  return {
    band_um: [1, 2],
    limits_origin: "open",
    flux,
    flux_err: flux.map(() => 0.5),
    n_used: flux.map(() => 5),
    npixels_mean: flux.map(() => 7),
    ...extra,
  };
}

function result(extra: Partial<TsoLightCurves> = {}): TsoLightCurves {
  const time = [60000.1, 60000.2, 60000.3, 60000.4];
  return {
    set_id: "abc",
    label: "NRS1",
    series: { detector: "NRS1", sporder: 0 },
    mode: "pipeline",
    time_scale: "bmjd_tdb",
    time,
    mjd_utc: time.map((t) => t - 0.002),
    int_num: [1, 2, 3, 4],
    row_index: [0, 1, 2, 3],
    flux_unit: "Jy",
    white: curve([10, 11, 12, 13]),
    bins: [],
    dropped: { nan_time: 0, repeated_time: 0 },
    rejected_elements: [],
    notes: [],
    warnings: [],
    ...extra,
  };
}

function props(extra: Partial<TsoSectionProps> = {}): TsoSectionProps {
  const set = e1Set();
  return { fileKey: "f1|p", paths: [E1_PATH], set, series: set.series[0], group: GROUP, ...extra };
}

function show(p: TsoSectionProps = props()): { tree: ReactNode; markup: string } {
  const tree = hooks.render(section, p) as ReactElement | null;
  return { tree, markup: tree ? renderToStaticMarkup(tree) : "" };
}

function byProp(tree: ReactNode, name: string, value?: unknown): ReactElement<Props>[] {
  return elementsOf<Props>(tree).filter((element) => (value === undefined ? element.props[name] !== undefined : element.props[name] === value));
}

function only(tree: ReactNode, name: string, value?: unknown): Props {
  const found = byProp(tree, name, value);
  expect(found).toHaveLength(1);
  return found[0].props;
}

function labelled(tree: ReactNode, text: string): Props {
  const label = elementsOf<Props>(tree).find((element) => element.type === "label" && textOf(element) === text);
  expect(label).toBeDefined();
  return only(tree, "id", label?.props.htmlFor);
}

function button(tree: ReactNode, text: string): Props {
  const found = elementsOf<Props>(tree).filter((element) => element.type === "button" && textOf(element) === text);
  expect(found).toHaveLength(1);
  return found[0].props;
}

function runButton(tree: ReactNode): Props {
  return only(tree, "label", "Compute light curves");
}

function typeText(field: Props, text: string): void {
  (field.onChange as (event: { target: { value: string } }) => void)({ target: { value: text } });
}

beforeEach(() => {
  hooks.mount();
  state.draft = null;
  state.curves = { loading: false, result: null, error: null };
  state.request = null;
  state.keys = [];
  state.patchDraft.mockReset();
  state.compute.mockReset();
  state.compute.mockImplementation((_group: string, _paths: readonly string[], _key: unknown, request: TsoLightCurvesRequest) => {
    state.request = request;
    return Promise.resolve();
  });
  view.props = null;
  selection.fn.mockClear();
});

afterEach(() => hooks.mount());

describe("TsoCurvesSection", () => {
  it("starts with the pipeline mode, blank limits and a hint, and no masked options", () => {
    const { tree, markup } = show();
    const mode = only(tree, "aria-label", "Light-curve mode");
    const options = elementsOf<Props>(tree).filter((element) => element.type === "option");
    expect(options.map(textOf)).toEqual(["as the pipeline (pipeline mode)", "valid elements only (masked mode)"]);
    expect(options.map((option) => option.props.value)).toEqual(["pipeline", "masked"]);
    expect(mode.value).toBe("pipeline");
    expect(byProp(tree, "data-tso-mode-note").map(textOf)).toEqual([
      "Sum of FLUX with NaN skipped, as the white-light step of the JWST pipeline does: it matches the archived whtlt.",
    ]);
    expect(only(tree, "aria-label", "White-light lower limit in micrometres")).toMatchObject({ value: "", optional: true, placeholder: "default" });
    expect(only(tree, "aria-label", "White-light upper limit in micrometres")).toMatchObject({ value: "", optional: true, placeholder: "default" });
    expect(markup).toContain("Blank uses the pipeline&#x27;s limits for this instrument mode.");
    expect(only(tree, "aria-label", "Bin edges in micrometres")).toMatchObject({ value: "", placeholder: "0.6, 1.0, 2.0, 3.0" });
    expect(only(tree, "aria-label", "Number of equal bins")).toMatchObject({ value: "5", integer: true, min: 1, max: 512 });
    expect(byProp(tree, "data-tso-masked-options")).toEqual([]);
    expect(byProp(tree, "data-tso-extra-bin")).toEqual([]);
    expect(textOf(tree)).toContain(HINT);
    expect(markup).toContain("Light curves");
    expect(markup).toContain("data-tso-curves");
    expect(markup).not.toContain("[view]");
    expect(markup).not.toContain('role="alert"');
    expect(byProp(tree, "data-field-error")).toEqual([]);
    expect(runButton(tree)).toMatchObject({ running: false, disabled: false, runningLabel: "Computing light curves" });
    expect(state.keys.every((key) => key === GROUP)).toBe(true);
  });

  it("writes every control into the draft of the group", () => {
    const { tree } = show();
    typeText(only(tree, "aria-label", "Light-curve mode"), "masked");
    expect(state.patchDraft).toHaveBeenLastCalledWith(GROUP, { mode: "masked" });
    typeText(only(tree, "aria-label", "Light-curve mode"), "pipeline");
    expect(state.patchDraft).toHaveBeenLastCalledWith(GROUP, { mode: "pipeline" });
    (only(tree, "aria-label", "White-light lower limit in micrometres").onChange as (text: string) => void)("1.0");
    expect(state.patchDraft).toHaveBeenLastCalledWith(GROUP, { minText: "1.0" });
    (only(tree, "aria-label", "White-light upper limit in micrometres").onChange as (text: string) => void)("4");
    expect(state.patchDraft).toHaveBeenLastCalledWith(GROUP, { maxText: "4" });
    typeText(only(tree, "aria-label", "Bin edges in micrometres"), "0.6, 1");
    expect(state.patchDraft).toHaveBeenLastCalledWith(GROUP, { binsText: "0.6, 1" });

    state.draft = { ...DEFAULT_TSO_DRAFT, mode: "masked", minText: "1.0", binsText: "0.6, 1" };
    const { tree: masked } = show();
    expect(only(masked, "aria-label", "Light-curve mode").value).toBe("masked");
    expect(only(masked, "aria-label", "White-light lower limit in micrometres").value).toBe("1.0");
    expect(only(masked, "aria-label", "Bin edges in micrometres").value).toBe("0.6, 1");
    expect(textOf(byProp(masked, "data-tso-mode-note")[0])).toBe(
      "Only spectral elements that are valid in every integration are summed, so every point adds the same elements.",
    );
    const options = byProp(masked, "data-tso-masked-options");
    expect(options).toHaveLength(1);
    expect(textOf(options[0])).toContain("An element is rejected when its time-median lies that many robust sigma from the median of its neighbours.");
    const leaveOut = labelled(masked, "Leave out (um)");
    expect(leaveOut).toMatchObject({ "aria-label": "Wavelength ranges to leave out", placeholder: "3.72-3.82; 4.2-4.4", value: "" });
    typeText(leaveOut, "3.72-3.82");
    expect(state.patchDraft).toHaveBeenLastCalledWith(GROUP, { excludeText: "3.72-3.82" });
    const toggle = only(masked, "label", "Reject isolated elements");
    expect(toggle.checked).toBe(true);
    (toggle.onChange as (value: boolean) => void)(false);
    expect(state.patchDraft).toHaveBeenLastCalledWith(GROUP, { rejectOn: false });
    const sigma = labelled(masked, "robust sigma");
    expect(sigma).toMatchObject({ value: "100", disabled: false });
    (sigma.onChange as (text: string) => void)("50");
    expect(state.patchDraft).toHaveBeenLastCalledWith(GROUP, { rejectSigmaText: "50" });

    state.draft = { ...state.draft, rejectOn: false };
    expect(labelled(show().tree, "robust sigma").disabled).toBe(true);
  });

  it("fills equal bins over the range of the series", () => {
    const { tree } = show();
    const fill = button(tree, "Fill equal bins");
    expect(fill.disabled).toBe(false);
    (fill.onClick as () => void)();
    expect(state.patchDraft).toHaveBeenCalledWith(GROUP, { binsText: equalEdgesText(0.5530469695008425, 5.370352185327354, 5) });

    (only(tree, "aria-label", "Number of equal bins").onChange as (text: string) => void)("3");
    const { tree: three } = show();
    expect(only(three, "aria-label", "Number of equal bins").value).toBe("3");
    (button(three, "Fill equal bins").onClick as () => void)();
    expect(state.patchDraft).toHaveBeenLastCalledWith(GROUP, { binsText: equalEdgesText(0.5530469695008425, 5.370352185327354, 3) });

    (only(three, "aria-label", "Number of equal bins").onChange as (text: string) => void)("0");
    expect(button(show().tree, "Fill equal bins").disabled).toBe(true);

    hooks.mount();
    const series = { ...e1Series(), wavelength_min_um: null };
    expect(button(show(props({ series })).tree, "Fill equal bins").disabled).toBe(true);
  });

  it("lists the bands added as bins and removes one", () => {
    state.draft = { ...DEFAULT_TSO_DRAFT, extraBins: [[4.15, 4.5], [2.5, 3]] };
    const { tree } = show();
    const rows = byProp(tree, "data-tso-extra-bin");
    expect(rows).toHaveLength(2);
    expect(textOf(rows[0])).toContain("band 4.15 to 4.5 um");
    expect(textOf(rows[1])).toContain("band 2.5 to 3 um");
    const removes = elementsOf<Props>(tree).filter((element) => element.type === "button" && textOf(element) === "Remove");
    expect(removes.map((remove) => remove.props["aria-label"])).toEqual(["Remove band 4.15 to 4.5 um", "Remove band 2.5 to 3 um"]);
    (removes[0].props.onClick as () => void)();
    expect(state.patchDraft).toHaveBeenLastCalledWith(GROUP, { extraBins: [[2.5, 3]] });
    (removes[1].props.onClick as () => void)();
    expect(state.patchDraft).toHaveBeenLastCalledWith(GROUP, { extraBins: [[4.15, 4.5]] });
  });

  it("computes with the request of the draft and blocks a draft that cannot be read", () => {
    const { tree } = show();
    (runButton(tree).onClick as () => void)();
    expect(state.compute).toHaveBeenCalledTimes(1);
    expect(state.compute).toHaveBeenCalledWith(GROUP, [E1_PATH], { detector: "NRS1", sporder: 0 }, DEFAULT_REQUEST);

    state.draft = { ...DEFAULT_TSO_DRAFT, minText: "2", maxText: "1" };
    const { tree: wrong } = show();
    expect(runButton(wrong).disabled).toBe(true);
    expect(byProp(wrong, "data-field-error").map(textOf)).toEqual(["the lower limit must be below the upper one"]);

    state.draft = null;
    state.curves = { loading: true, result: null, error: null };
    const { tree: busy, markup } = show();
    expect(runButton(busy).running).toBe(true);
    expect(markup).toContain("Computing light curves");
    expect(textOf(busy)).not.toContain(HINT);
  });

  it("shows the limits in use, what was dropped or rejected, the warnings and the curves", () => {
    const shown = result({
      white: curve([10, 11, 12, 13], { limits_origin: "file log", band_um: [2.47, 3.94] }),
      dropped: { nan_time: 3, repeated_time: 0 },
      rejected_elements: [{ index: 230, wavelength_um: 3.0178602767154326, reason: "r" }],
      warnings: ["w"],
      notes: ["n"],
    });
    state.curves = { loading: false, result: shown, error: null };
    const { tree, markup } = show();
    expect(byProp(tree, "data-tso-limits").map(textOf)).toEqual([
      "White light summed over 2.47 to 3.94 um (limits from the processing log of the file)",
    ]);
    expect(byProp(tree, "data-tso-dropped").map(textOf)).toEqual([
      "The pipeline rule left out 3 integration(s) with no MJD-AVG and 0 with a repeated MJD-AVG.",
    ]);
    expect(byProp(tree, "data-tso-rejected").map(textOf)).toEqual(["Rejected element at 3.01786 um (channel 230)"]);
    expect(markup).toContain(">w<");
    const notes = byProp(tree, "data-tso-curve-notes");
    expect(notes).toHaveLength(1);
    expect(notes[0].type).toBe("details");
    expect(textOf(notes[0])).toBe("notes (1)n");
    expect(markup).toContain("[view]");
    expect(textOf(tree)).not.toContain(HINT);
    expect(byProp(tree, "data-tso-point")).toEqual([]);
    expect(byProp(tree, "data-tso-stale")).toEqual([]);
    expect(view.props).toMatchObject({ group: GROUP, csvName: "tso_lightcurve", selectedPointRef: null });
    expect(typeof view.props?.onSelectPoint).toBe("function");

    (view.props?.onSelectPoint as (doc: LightCurveDoc, ref: number) => void)(DOC, 2);
    const { tree: picked } = show();
    expect(byProp(picked, "data-tso-point").map(textOf)).toEqual([
      "integration 2 of the series (INT_NUM 2), BMJD (TDB) 60000.2, white light 11 Jy, 5 elements, mean NPIXELS 7",
    ]);
    expect(view.props?.selectedPointRef).toBe(2);

    state.curves = { loading: false, result: result(), error: null };
    const { tree: plain } = show();
    expect(byProp(plain, "data-tso-dropped")).toEqual([]);
    expect(byProp(plain, "data-tso-rejected")).toEqual([]);
    expect(byProp(plain, "data-tso-curve-notes")).toEqual([]);
    expect(byProp(plain, "data-tso-limits").map(textOf)).toEqual(["White light summed over 1 to 2 um (open limits)"]);

    (view.props?.onSelectPoint as (doc: LightCurveDoc, ref: number) => void)(DOC, 3);
    (only(plain, "aria-label", "Number of equal bins").onChange as (text: string) => void)("7");
    show();
    expect(view.props?.selectedPointRef).toBe(3);
    const { tree: other } = show(props({ group: "tso:4b385da81d5c70b8:NRS2:0" }));
    expect(view.props).toMatchObject({ group: "tso:4b385da81d5c70b8:NRS2:0", selectedPointRef: null });
    expect(byProp(other, "data-tso-point")).toEqual([]);
    expect(only(other, "aria-label", "Number of equal bins").value).toBe("5");
  });

  it("says when the curves on screen no longer match the settings", () => {
    const { tree } = show();
    (runButton(tree).onClick as () => void)();
    state.curves = { loading: false, result: result(), error: null };
    const { tree: same } = show();
    expect(byProp(same, "data-tso-stale")).toEqual([]);

    state.draft = { ...DEFAULT_TSO_DRAFT, mode: "masked" };
    const { tree: changed } = show();
    expect(byProp(changed, "data-tso-stale").map(textOf)).toEqual([STALE_LINE]);
  });

  it("shows the failure of a computation and keeps the last curves", () => {
    const error = "the masked mode needs one wavelength grid for every integration ...";
    state.curves = { loading: false, result: result(), error };
    const { markup } = show();
    expect(markup.match(/role="alert"/g)).toHaveLength(1);
    expect(markup).toContain(`role="alert">${error}</div>`);
    expect(markup).toContain("[view]");
  });

  it("a click on a point selects that integration in the Spectra panel by its series position", () => {
    state.curves = { loading: false, result: result({ row_index: [0, 2, 3, 5] }), error: null };
    show();
    (view.props?.onSelectPoint as (doc: LightCurveDoc, ref: number) => void)(DOC, 2);
    expect(selection.fn).toHaveBeenCalledTimes(1);
    expect(selection.fn).toHaveBeenCalledWith("f1|p", {
      kind: "series",
      detector: "NRS1",
      sporder: 0,
      pick: { kind: "integration", index: 2 },
    });
    expect(byProp(show().tree, "data-tso-point").map(textOf)).toEqual([
      "integration 3 of the series (INT_NUM 2), BMJD (TDB) 60000.2, white light 11 Jy, 5 elements, mean NPIXELS 7",
    ]);

    (view.props?.onSelectPoint as (doc: LightCurveDoc, ref: number) => void)(DOC, 99);
    expect(selection.fn).toHaveBeenCalledTimes(1);
    expect(byProp(show().tree, "data-tso-point")).toEqual([]);

    (view.props?.onSelectPoint as (doc: LightCurveDoc, ref: number) => void)(DOC, 4);
    expect(selection.fn).toHaveBeenCalledTimes(2);
    expect(selection.fn).toHaveBeenLastCalledWith("f1|p", {
      kind: "series",
      detector: "NRS1",
      sporder: 0,
      pick: { kind: "integration", index: 5 },
    });
  });
});

describe("TsoCurvesSection export and log", () => {
  const SAVED = "C:/out/name.csv";
  let append: MockInstance<typeof measurementLog.append>;

  const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

  beforeEach(() => {
    save.fn.mockReset();
    save.csv.mockClear();
    append = vi.spyOn(measurementLog, "append").mockImplementation(() => ({}) as never);
  });

  afterEach(() => append.mockRestore());

  function actions(tree: ReactNode): ReactElement<Props>[] {
    return byProp(tree, "data-tso-curves-actions");
  }

  function actionButton(tree: ReactNode, text: string): Props {
    const block = actions(tree);
    expect(block).toHaveLength(1);
    const found = elementsOf<Props>(block[0]).filter((element) => element.type === "button" && textOf(element) === text);
    expect(found).toHaveLength(1);
    return found[0].props;
  }

  it("offers Save CSV and Log only when curves exist", () => {
    expect(actions(show().tree)).toEqual([]);

    state.curves = { loading: false, result: result(), error: null };
    const { tree } = show();
    expect(actionButton(tree, "Save CSV").disabled).toBe(false);
    expect(actionButton(tree, "Log").disabled).toBe(false);

    state.curves = { loading: true, result: result(), error: null };
    const { tree: busy } = show();
    expect(actionButton(busy, "Save CSV").disabled).toBe(true);
    expect(actionButton(busy, "Log").disabled).toBe(true);
  });

  it("saves the curves with their provenance and logs the saved path", async () => {
    state.curves = { loading: false, result: result(), error: null };
    save.fn.mockResolvedValue(SAVED);
    const before = new Date().toISOString();
    (actionButton(show().tree, "Save CSV").onClick as () => void)();
    await flush();
    expect(save.fn).toHaveBeenCalledTimes(1);
    expect(save.fn).toHaveBeenCalledWith("CSV", "name.csv", "Save light curves");
    expect(save.csv).toHaveBeenCalledTimes(1);
    const [shown, context] = save.csv.mock.calls[0] as unknown as [TsoLightCurves, { software: string; exportedUtc: string; files: unknown }];
    expect(shown).toBe((state.curves as TsoCurvesState).result);
    expect(context.software).toBe(`AstroBurst ${APP_VERSION}`);
    expect(context.files).toEqual(e1Set().files);
    expect(context.exportedUtc >= before && context.exportedUtc <= new Date().toISOString()).toBe(true);
    expect(append).toHaveBeenCalledTimes(1);
    expect(append).toHaveBeenCalledWith({ draft: "curves", path: SAVED });
    const { tree, markup } = show();
    expect(byProp(tree, "data-tso-saved").map(textOf)).toEqual([`Saved ${SAVED}`]);
    expect(byProp(tree, "data-tso-logged")).toEqual([]);
    expect(markup).not.toContain('role="alert"');
    show();
    expect(append).toHaveBeenCalledTimes(1);
  });

  it("logs nothing when the dialog is cancelled and says when the save failed", async () => {
    state.curves = { loading: false, result: result(), error: null };
    save.fn.mockResolvedValue(null);
    (actionButton(show().tree, "Save CSV").onClick as () => void)();
    await flush();
    expect(save.fn).toHaveBeenCalledTimes(1);
    expect(append).not.toHaveBeenCalled();
    const { tree: cancelled, markup: cancelledMarkup } = show();
    expect(byProp(cancelled, "data-tso-saved")).toEqual([]);
    expect(byProp(cancelled, "data-tso-logged")).toEqual([]);
    expect(cancelledMarkup).not.toContain('role="alert"');

    save.fn.mockRejectedValue(new Error("disk full"));
    (actionButton(cancelled, "Save CSV").onClick as () => void)();
    await flush();
    expect(append).not.toHaveBeenCalled();
    const { tree: failed, markup } = show();
    expect(markup.match(/role="alert"/g)).toHaveLength(1);
    expect(markup).toContain('role="alert">Could not save the file: disk full</div>');
    expect(byProp(failed, "data-tso-saved")).toEqual([]);
  });

  it("logs the curves without saving", () => {
    state.curves = { loading: false, result: result(), error: null };
    (actionButton(show().tree, "Log").onClick as () => void)();
    expect(save.fn).not.toHaveBeenCalled();
    expect(append).toHaveBeenCalledTimes(1);
    expect(append).toHaveBeenCalledWith({ draft: "curves", path: null });
    const { tree } = show();
    expect(byProp(tree, "data-tso-logged").map(textOf)).toEqual(["Added to the measurement log"]);
    expect(byProp(tree, "data-tso-saved")).toEqual([]);
  });
});
