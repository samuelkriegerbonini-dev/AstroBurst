import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { TsoCurvesState, TsoSectionProps } from "../../../hooks/useTsoStore";
import type { LightCurveDoc } from "../../../shared/types/lightCurve";
import type {
  TsoFile,
  TsoLightCurves,
  TsoOpenResult,
  TsoSeriesSummary,
  WhtltComparison,
  WhtltImport,
} from "../../../shared/types/tso";

const state = vi.hoisted(() => ({
  curves: { loading: false, result: null, error: null } as TsoCurvesState,
  curvesKey: null as string | null,
  whtltImport: vi.fn(),
  tsoWhtltCompare: vi.fn(),
  setExtraDocs: vi.fn(),
  open: vi.fn(),
}));

vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../services/tso", () => ({ whtltImport: state.whtltImport, tsoWhtltCompare: state.tsoWhtltCompare }));
vi.mock("../../../hooks/useTsoStore", () => ({
  tsoStore: { setExtraDocs: state.setExtraDocs },
  useTsoCurves: (group: string | null) => {
    state.curvesKey = group;
    return state.curves;
  },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: state.open }));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";

const { default: TsoWhtltSection } = await import("../TsoWhtltSection");

type Section = (props: TsoSectionProps) => ReactNode;
type Props = Record<string, unknown>;
const section = (TsoWhtltSection as unknown as { type: Section }).type;

const E1_PATH = "C:/d/e1_x1dints.fits";
const E1_WHTLT = "C:/d/e1_whtlt.ecsv";
const OTHER_WHTLT = "C:/x/other_whtlt.ecsv";
const GROUP = "tso:4b385da81d5c70b8:NRS1:0";
const OVERLAY_HINT = 'Choose "white light and whtlt" in the series list of the plot to see both curves.';
const NOT_DRAWN =
  "This table cannot be drawn over the curves: the series has no time axis, no single column of the table belongs to it, or the table has no times on the time scale of the series.";
const CAPTION = "The comparison uses the pipeline mode with the pipeline's limits; rows are paired on time, never on position.";
const LEVEL_2B_NOTE =
  "these are level 2b spectra: the archived white light comes from the level 3 product, extracted after outlier detection, so a difference is expected";
const NOT_A_TABLE = "x_whtlt.ecsv: no whitelight_flux column: this is not a white-light table (columns: a, b)";

function e1File(whtltPath: string | null): TsoFile {
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
    whtlt_path: whtltPath,
    n_extract1d: 1,
    data_bytes: 14702400,
  };
}

const E1_SERIES: TsoSeriesSummary = {
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
  completeness: [
    { exposure: "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048", nints: 240, found: 240, missing: [], complete: true },
  ],
  notes: [],
  warnings: [],
};

function e1Set(whtltPath: string | null): TsoOpenResult {
  return { set_id: "4b385da81d5c70b8", files: [e1File(whtltPath)], series: [E1_SERIES], notes: [], warnings: [] };
}

function props(whtltPath: string | null, fileKey = "f1|p"): TsoSectionProps {
  return { fileKey, paths: [E1_PATH], set: e1Set(whtltPath), series: E1_SERIES, group: GROUP };
}

const E1_ROW: WhtltComparison = {
  series: { detector: "NRS1", sporder: 0 },
  label: "NRS1",
  column: "whitelight_flux_NRS1",
  n_whtlt: 240,
  n_joined: 240,
  max_rel_diff: 1.863700988171452e-16,
  within_1e4: true,
  limits_um: [0.5530469695008425, 5.370352185327354],
  limits_origin: "open",
  notes: [],
};

function imported(path: string): WhtltImport {
  return {
    path,
    file_name: path.split("/").pop() ?? path,
    schema: "current",
    series: [
      {
        column: "whitelight_flux_NRS1",
        detector: "NRS1",
        sporder: null,
        mjd_utc: [60000.098, 60000.198],
        bmjd_tdb: [60000.1, 60000.2],
        flux: [10, 11],
        flux_unit: "Jy",
      },
    ],
    meta: null,
    notes: [],
  };
}

const LEGACY: WhtltImport = {
  path: E1_WHTLT,
  file_name: "e1_whtlt.ecsv",
  schema: "legacy",
  series: [
    {
      column: "whitelight_flux",
      detector: "MIRIMAGE",
      sporder: null,
      mjd_utc: [60000.098, 60000.198],
      bmjd_tdb: [null, null],
      flux: [10, 11],
      flux_unit: "Jy",
    },
  ],
  meta: null,
  notes: [],
};

function curves(extra: Partial<TsoLightCurves> = {}): TsoLightCurves {
  const time = [60000.1, 60000.2, 60000.3, 60000.4];
  const flux = [10, 11, 12, 13];
  return {
    set_id: "4b385da81d5c70b8",
    label: "NRS1",
    series: { detector: "NRS1", sporder: 0 },
    mode: "pipeline",
    time_scale: "bmjd_tdb",
    time,
    mjd_utc: time.map((t) => t - 0.002),
    int_num: [1, 2, 3, 4],
    row_index: [0, 1, 2, 3],
    flux_unit: "Jy",
    white: {
      band_um: [1, 2],
      limits_origin: "open",
      flux,
      flux_err: flux.map(() => 0.5),
      n_used: flux.map(() => 5),
      npixels_mean: flux.map(() => 7),
    },
    bins: [],
    dropped: { nan_time: 0, repeated_time: 0 },
    rejected_elements: [],
    notes: [],
    warnings: [],
    ...extra,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

async function show(input: TsoSectionProps): Promise<{ tree: ReactNode; markup: string; text: string }> {
  const first = hooks.render(section, input) as ReactElement | null;
  if (first) renderToStaticMarkup(first);
  await tick();
  const tree = hooks.render(section, input) as ReactElement | null;
  return { tree, markup: tree ? renderToStaticMarkup(tree) : "", text: textOf(tree) };
}

function withProp(tree: ReactNode, name: string): ReactElement<Props>[] {
  return elementsOf<Props>(tree).filter((element) => element.props[name] !== undefined);
}

function button(tree: ReactNode, label: string): ReactElement<Props> | undefined {
  return elementsOf<Props>(tree).find((element) => element.type === "button" && textOf(element) === label);
}

async function click(tree: ReactNode, label: string): Promise<void> {
  const target = button(tree, label);
  expect(target).toBeDefined();
  const opened = state.open.mock.calls.length;
  (target?.props.onClick as () => void)();
  await vi.waitFor(() => expect(state.open.mock.calls.length).toBeGreaterThan(opened));
  await tick();
  await tick();
}

function cells(tree: ReactNode, type: "th" | "td"): ReactElement<Props>[] {
  return elementsOf<Props>(tree).filter((element) => element.type === type);
}

function lastDocs(): LightCurveDoc[] {
  const calls = state.setExtraDocs.mock.calls;
  expect(calls.length).toBeGreaterThan(0);
  const [group, docs] = calls[calls.length - 1] as [string, LightCurveDoc[]];
  expect(group).toBe(GROUP);
  return docs;
}

function resolveWith(rows: WhtltComparison[], table: (path: string) => WhtltImport = imported): void {
  state.tsoWhtltCompare.mockImplementation(() => Promise.resolve(rows));
  state.whtltImport.mockImplementation((path: string) => Promise.resolve(table(path)));
}

beforeEach(() => {
  hooks.mount();
  state.curves = { loading: false, result: null, error: null };
  state.curvesKey = null;
  state.whtltImport.mockReset();
  state.tsoWhtltCompare.mockReset();
  state.setExtraDocs.mockReset();
  state.open.mockReset();
  resolveWith([E1_ROW]);
});

afterEach(() => hooks.mount());

describe("TsoWhtltSection", () => {
  it("reads the table found beside the file without a click", async () => {
    const { tree, markup, text } = await show(props(E1_WHTLT));

    expect(state.tsoWhtltCompare).toHaveBeenCalledTimes(1);
    expect(state.tsoWhtltCompare).toHaveBeenCalledWith([E1_PATH], E1_WHTLT);
    expect(state.whtltImport).toHaveBeenCalledTimes(1);
    expect(state.whtltImport).toHaveBeenCalledWith(E1_WHTLT);
    expect(markup).toContain("data-tso-whtlt");
    expect(text).toContain("Pipeline white light (whtlt)");
    expect(text).toContain("Compared with e1_whtlt.ecsv");
    expect(text).not.toContain("No _whtlt.ecsv was found beside this file.");
    expect(button(tree, "Choose another file")).toBeDefined();
    expect(button(tree, "Choose whtlt file")).toBeUndefined();
    expect(text).toContain(CAPTION);
    expect(markup).not.toContain('role="alert"');
    expect(markup).not.toContain("animate-spin");

    expect(withProp(tree, "data-tso-whtlt-table")).toHaveLength(1);
    expect(cells(tree, "th").map(textOf)).toEqual(["column", "series", "rows paired", "largest relative difference", "verdict"]);
    expect(cells(tree, "td").map(textOf).slice(0, 5)).toEqual([
      "whitelight_flux_NRS1",
      "NRS1",
      "240 of 240",
      "1.86e-16",
      "matches (within 1e-4)",
    ]);
    expect(String(cells(tree, "td")[4].props.className)).not.toContain("amber");
    expect(withProp(tree, "data-tso-whtlt-note").map(textOf)).toEqual(["summed over 0.553 to 5.37 um (open)"]);
    expect(String(withProp(tree, "data-tso-whtlt-note")[0].props.className)).toContain("text-[9px] text-zinc-500");

    await show(props(E1_WHTLT));
    expect(state.tsoWhtltCompare).toHaveBeenCalledTimes(1);
  });

  it("asks for the table when none lies beside the file", async () => {
    const { tree, text } = await show(props(null));

    expect(state.tsoWhtltCompare).not.toHaveBeenCalled();
    expect(state.whtltImport).not.toHaveBeenCalled();
    expect(text).toContain("No _whtlt.ecsv was found beside this file.");
    expect(text).not.toContain("Compared with");
    expect(withProp(tree, "data-tso-whtlt-table")).toEqual([]);

    state.open.mockResolvedValueOnce(null);
    await click(tree, "Choose whtlt file");
    expect(state.open).toHaveBeenCalledTimes(1);
    expect(state.open).toHaveBeenCalledWith({
      multiple: false,
      title: "Choose the whtlt table of this observation",
      filters: [{ name: "White-light table", extensions: ["ecsv"] }],
    });
    const { tree: unchanged, text: same } = await show(props(null));
    expect(state.tsoWhtltCompare).not.toHaveBeenCalled();
    expect(state.whtltImport).not.toHaveBeenCalled();
    expect(same).toContain("No _whtlt.ecsv was found beside this file.");

    state.open.mockResolvedValueOnce(OTHER_WHTLT);
    await click(unchanged, "Choose whtlt file");
    const { tree: chosen, text: compared } = await show(props(null));
    expect(state.tsoWhtltCompare).toHaveBeenCalledWith([E1_PATH], OTHER_WHTLT);
    expect(state.whtltImport).toHaveBeenCalledWith(OTHER_WHTLT);
    expect(compared).toContain("Compared with other_whtlt.ecsv");
    expect(button(chosen, "Choose another file")).toBeDefined();
    expect(withProp(chosen, "data-tso-whtlt-table")).toHaveLength(1);

    const { tree: other, text: otherFile } = await show(props(null, "f2|q"));
    expect(otherFile).toContain("No _whtlt.ecsv was found beside this file.");
    expect(withProp(other, "data-tso-whtlt-table")).toEqual([]);
  });

  it("marks a column that differs and shows the notes of each row", async () => {
    resolveWith([
      {
        ...E1_ROW,
        series: { detector: "NRS1", sporder: -1 },
        n_whtlt: 44,
        n_joined: 44,
        max_rel_diff: 723.837953015402,
        within_1e4: false,
        limits_um: [2.848429652675818, 5.182052338975267],
        notes: [LEVEL_2B_NOTE],
      },
      { ...E1_ROW, column: "whitelight_flux_NRS2", series: null, label: null, n_whtlt: 8, n_joined: 0, max_rel_diff: null, within_1e4: false, limits_um: null, limits_origin: null, notes: ["column whitelight_flux_NRS2: 0 series of these files match it (detector NRS2, order any); nothing compared"] },
    ]);

    const { tree } = await show(props(E1_WHTLT));
    const verdicts = cells(tree, "td").filter((cell) => ["differs", "not compared"].includes(textOf(cell)));
    expect(verdicts.map(textOf)).toEqual(["differs", "not compared"]);
    expect(String(verdicts[0].props.className)).toContain("text-amber-300");
    expect(String(verdicts[1].props.className)).not.toContain("amber");
    expect(cells(tree, "td").map(textOf)).toContain("724");
    expect(cells(tree, "td").map(textOf)).toContain("no single series");
    expect(cells(tree, "td").map(textOf)).toContain("--");

    const notes = withProp(tree, "data-tso-whtlt-note").map(textOf);
    expect(notes.filter((line) => line === LEVEL_2B_NOTE)).toHaveLength(1);
    expect(notes).toEqual([
      "summed over 2.848 to 5.182 um (open)",
      LEVEL_2B_NOTE,
      "column whitelight_flux_NRS2: 0 series of these files match it (detector NRS2, order any); nothing compared",
    ]);
  });

  it("adds the archived curve to the group once curves exist and removes it when they do not", async () => {
    const { tree: bare } = await show(props(E1_WHTLT));
    expect(state.curvesKey).toBe(GROUP);
    expect(lastDocs()).toEqual([]);
    expect(withProp(bare, "data-tso-whtlt-overlay")).toEqual([]);

    state.curves = { loading: false, result: curves(), error: null };
    const { tree } = await show(props(E1_WHTLT));
    const docs = lastDocs();
    expect(docs.map((doc) => doc.id)).toEqual(["white:app", "whtlt:whitelight_flux_NRS1"]);
    expect(docs.map((doc) => doc.overlay)).toEqual(["white light and whtlt", "white light and whtlt"]);
    expect(docs[1].time).toEqual([60000.1, 60000.2]);
    expect(withProp(tree, "data-tso-whtlt-overlay").map(textOf)).toEqual([OVERLAY_HINT]);

    state.curves = { loading: false, result: null, error: null };
    const { tree: cleared } = await show(props(E1_WHTLT));
    expect(lastDocs()).toEqual([]);
    expect(withProp(cleared, "data-tso-whtlt-overlay")).toEqual([]);

    state.curves = { loading: false, result: curves({ time_scale: "index" }), error: null };
    const { tree: indexed } = await show(props(E1_WHTLT));
    expect(lastDocs()).toEqual([]);
    expect(withProp(indexed, "data-tso-whtlt-overlay").map(textOf)).toEqual([NOT_DRAWN]);

    hooks.mount();
    state.setExtraDocs.mockReset();
    resolveWith([E1_ROW], () => LEGACY);
    state.curves = { loading: false, result: curves({ series: { detector: "MIRIMAGE", sporder: 1 } }), error: null };
    const { tree: legacy } = await show(props(E1_WHTLT));
    expect(lastDocs()).toEqual([]);
    expect(state.setExtraDocs.mock.calls.every(([, docs]) => (docs as LightCurveDoc[]).length === 0)).toBe(true);
    expect(withProp(legacy, "data-tso-whtlt-overlay").map(textOf)).toEqual([NOT_DRAWN]);
    expect(textOf(legacy)).not.toContain(OVERLAY_HINT);
  });

  it("shows a failed read and drops a late answer", async () => {
    state.tsoWhtltCompare.mockImplementation(() => Promise.reject(new Error(NOT_A_TABLE)));
    const { tree, markup } = await show(props(E1_WHTLT));
    expect(markup.match(/role="alert"/g)).toHaveLength(1);
    expect(markup).toContain(NOT_A_TABLE);
    expect(withProp(tree, "data-tso-whtlt-table")).toEqual([]);

    hooks.mount();
    const first = deferred<WhtltComparison[]>();
    const second = deferred<WhtltComparison[]>();
    state.tsoWhtltCompare.mockReset();
    state.tsoWhtltCompare.mockImplementation((_paths: string[], path: string) => (path === "C:/x/a_whtlt.ecsv" ? first.promise : second.promise));
    const { tree: start } = await show(props(null));

    state.open.mockResolvedValueOnce("C:/x/a_whtlt.ecsv");
    await click(start, "Choose whtlt file");
    const { tree: loading, markup: spinning } = await show(props(null));
    expect(spinning).toContain("animate-spin");

    state.open.mockResolvedValueOnce("C:/x/b_whtlt.ecsv");
    await click(loading, "Choose another file");
    await show(props(null));
    expect(state.tsoWhtltCompare.mock.calls.map((call) => call[1])).toEqual(["C:/x/a_whtlt.ecsv", "C:/x/b_whtlt.ecsv"]);

    second.resolve([{ ...E1_ROW, column: "whitelight_flux_B" }]);
    await tick();
    first.resolve([{ ...E1_ROW, column: "whitelight_flux_A" }]);
    await tick();
    const { tree: settled, markup: done, text } = await show(props(null));
    expect(text).toContain("Compared with b_whtlt.ecsv");
    expect(cells(settled, "td").map(textOf)).toContain("whitelight_flux_B");
    expect(cells(settled, "td").map(textOf)).not.toContain("whitelight_flux_A");
    expect(done).not.toContain('role="alert"');
    expect(done).not.toContain("animate-spin");
  });
});
