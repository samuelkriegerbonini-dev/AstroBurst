import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { TsoSetState } from "../../../hooks/useTsoStore";
import type { TsoExposureCompleteness, TsoFile, TsoOpenResult, TsoSeriesSummary } from "../../../shared/types/tso";
import type { TsoLightCurvesPanelProps } from "../TsoLightCurvesPanel";

const state = vi.hoisted(() => ({
  key: null as string | null,
  set: { paths: [], loading: false, result: null, error: null } as TsoSetState,
  chosen: [] as readonly string[],
  index: 0,
  files: [] as { id: string; path: string; name: string; series: boolean }[],
  open: vi.fn(),
  setChosen: vi.fn(),
  setSeriesIndex: vi.fn(),
  children: new Map<string, Record<string, unknown>>(),
}));

vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../hooks/useTsoStore", () => ({
  tsoStore: { open: state.open, setChosen: state.setChosen, setSeriesIndex: state.setSeriesIndex },
  useTsoSet: (key: string | null) => (key !== null && key === state.key ? state.set : { paths: [], loading: false, result: null, error: null }),
  useTsoChosen: (key: string | null) => (key !== null && key === state.key ? state.chosen : []),
  useTsoSeriesIndex: (key: string | null) => (key !== null && key === state.key ? state.index : 0),
}));
vi.mock("../../../hooks/useFileStore", () => ({ fileStore: { getFiles: () => state.files, subscribe: () => () => {} } }));
vi.mock("../../../utils/fileKinds", () => ({ isSpectrumSeries: (file: { series?: boolean }) => file.series === true }));
vi.mock("../TsoCurvesSection", () => ({
  default: (props: Record<string, unknown>) => {
    state.children.set("curves", props);
    return "[curves]";
  },
}));
vi.mock("../TsoWhtltSection", () => ({
  default: (props: Record<string, unknown>) => {
    state.children.set("whtlt", props);
    return "[whtlt]";
  },
}));
vi.mock("../TsoDepthSpectrum", () => ({
  default: (props: Record<string, unknown>) => {
    state.children.set("depth", props);
    return "[depth]";
  },
}));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";

const { default: TsoLightCurvesPanel } = await import("../TsoLightCurvesPanel");

type Panel = (props: TsoLightCurvesPanelProps) => ReactNode;
const panel = (TsoLightCurvesPanel as unknown as { type: Panel }).type;

const E1_PATH = "C:/d/e1_x1dints.fits";
const E1_KEY = `f1|${E1_PATH}`;
const EXPOSURE = "jw02327001001_03101_00001";
const SEG1 = `C:/d/${EXPOSURE}-seg001_nrs1_x1dints.fits`;
const SEG2 = `C:/d/${EXPOSURE}-seg002_nrs1_x1dints.fits`;
const SEG3 = `C:/d/${EXPOSURE}-seg003_nrs1_x1dints.fits`;
const SEG_KEY = `s1|${SEG1}`;
const LEVEL3_PATH = "C:/d/jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048_x1dints.fits";
const LEVEL_MIX = "a.fits is a level 3 product and b.fits is a level 2b product, and both hold the series NRS1: their FLUX differs";

type Props = Record<string, unknown>;

function tsoFile(path: string, level: number, layout: TsoFile["layout"]): TsoFile {
  return {
    path,
    file_name: path.split("/").pop() ?? path,
    sha256: "a5e7022d22e779fdf43b14fba6e7d23644c9547c920b6a52c13435c63706bfbc",
    level,
    datamodel: "TSOMultiSpecModel",
    cal_ver: "1.18.0",
    exp_type: "NRS_BRIGHTOBJ",
    layout,
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

function e1Series(completeness: TsoExposureCompleteness[] = [
  { exposure: "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048", nints: 240, found: 240, missing: [], complete: true },
]): TsoSeriesSummary {
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
    completeness,
    notes: [],
    warnings: [],
  };
}

function a5Series(detector: string, channels: number): TsoSeriesSummary {
  return {
    ...e1Series([{ exposure: EXPOSURE, nints: 8, found: 8, missing: [], complete: true }]),
    key: { detector, sporder: -1 },
    label: detector,
    n_integrations: 8,
    n_channels: channels,
  };
}

function e1Set(extra: Partial<TsoOpenResult> = {}): TsoOpenResult {
  return {
    set_id: "4b385da81d5c70b8",
    files: [tsoFile(E1_PATH, 3, "rows")],
    series: [e1Series()],
    notes: ["n1", "n2"],
    warnings: ["w"],
    ...extra,
  };
}

function a5Set(): TsoOpenResult {
  return {
    set_id: "a5a5a5a5a5a5a5a5",
    files: [tsoFile(`C:/d/jw03375001001_03101_00001-seg001_nrs1_x1dints.fits`, 2, "rows"), tsoFile(`C:/d/jw03375001001_03101_00001-seg001_nrs2_x1dints.fits`, 2, "rows")],
    series: [a5Series("NRS1", 1271), a5Series("NRS2", 2048)],
    notes: [],
    warnings: [],
  };
}

function loadedSet(result: TsoOpenResult, paths: string[] = [E1_PATH]): TsoSetState {
  return { paths, loading: false, result, error: null };
}

function seriesFile(path: string, series = true) {
  return { id: path, path, name: path.split("/").pop() ?? path, series };
}

async function show(props: TsoLightCurvesPanelProps): Promise<{ tree: ReactNode; markup: string }> {
  const first = hooks.render(panel, props) as ReactElement | null;
  if (first) renderToStaticMarkup(first);
  await new Promise((resolve) => setTimeout(resolve, 0));
  const tree = hooks.render(panel, props) as ReactElement | null;
  return { tree, markup: tree ? renderToStaticMarkup(tree) : "" };
}

function withProp(tree: ReactNode, name: string): ReactElement<Props>[] {
  return elementsOf<Props>(tree).filter((element) => element.props[name] !== undefined);
}

function seriesSelect(tree: ReactNode): Props | undefined {
  return elementsOf<Props>(tree).find((element) => element.type === "select" && element.props["aria-label"] === "Time series")?.props;
}

beforeEach(() => {
  hooks.mount();
  state.key = E1_KEY;
  state.set = { paths: [], loading: false, result: null, error: null };
  state.chosen = [];
  state.index = 0;
  state.files = [seriesFile(E1_PATH)];
  state.open.mockReset();
  state.setChosen.mockReset();
  state.setSeriesIndex.mockReset();
  state.children.clear();
});

afterEach(() => hooks.mount());

describe("TsoLightCurvesPanel", () => {
  it("renders nothing without a file", async () => {
    expect(hooks.render(panel, { filePath: null, fileKey: null })).toBeNull();
    expect(hooks.render(panel, { filePath: E1_PATH, fileKey: null })).toBeNull();
    expect(hooks.render(panel, { filePath: null, fileKey: E1_KEY })).toBeNull();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(state.open).not.toHaveBeenCalled();
  });

  it("opens the set of the selected file and shows a spinner while it loads", async () => {
    state.set = { paths: [E1_PATH], loading: true, result: null, error: null };

    const { tree, markup } = await show({ filePath: E1_PATH, fileKey: E1_KEY });

    expect(state.open).toHaveBeenCalledTimes(1);
    expect(state.open).toHaveBeenCalledWith(E1_KEY, [E1_PATH]);
    expect(markup).toContain("Time-series spectra");
    expect(markup).toContain("animate-spin");
    expect(markup).toContain("data-tso-panel");
    expect(markup).not.toContain('role="alert"');
    expect(withProp(tree, "data-tso-segments")).toEqual([]);
    expect(withProp(tree, "data-tso-file")).toEqual([]);
    expect(state.children.size).toBe(0);
    expect(markup).not.toContain("[curves]");

    state.set = loadedSet(e1Set());
    const { markup: loaded } = await show({ filePath: E1_PATH, fileKey: E1_KEY });
    expect(loaded).not.toContain("animate-spin");
    expect(state.open).toHaveBeenCalledTimes(1);
  });

  it("shows files, facts, completeness, warnings and notes of a loaded set, and mounts the three sub-sections", async () => {
    const set = e1Set();
    state.set = loadedSet(set);

    const { tree, markup } = await show({ filePath: E1_PATH, fileKey: E1_KEY });
    const text = textOf(tree);

    expect(withProp(tree, "data-tso-file").map(textOf)).toEqual(["e1_x1dints.fits, level 3, one row per integration"]);
    expect(withProp(tree, "data-tso-facts").map(textOf)).toEqual([
      "240 integrations x 431 channels, flux in Jy, time axis BMJD (TDB) (TDB-MID column)",
    ]);
    expect(withProp(tree, "data-tso-exposure").map(textOf)).toEqual([
      "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048: 240 of 240 integrations",
    ]);
    expect(String(withProp(tree, "data-tso-exposure")[0].props.className)).not.toContain("amber");
    expect(markup).toContain(">w<");
    const notes = elementsOf<Props>(tree).filter((element) => element.props["data-tso-notes"] !== undefined);
    expect(notes).toHaveLength(1);
    expect(notes[0].type).toBe("details");
    expect(textOf(notes[0])).toBe("notes (2)n1n2");
    expect(elementsOf(notes[0]).some((element) => element.type === "summary" && textOf(element) === "notes (2)")).toBe(true);
    expect(text).toContain("notes (2)");
    expect(seriesSelect(tree)).toBeUndefined();

    expect(markup).toContain("[curves][whtlt][depth]");
    const expected = { fileKey: E1_KEY, paths: [E1_PATH], set, series: set.series[0], group: "tso:4b385da81d5c70b8:NRS1:0" };
    expect(state.children.get("curves")).toEqual(expected);
    expect(state.children.get("whtlt")).toEqual(expected);
    expect(state.children.get("depth")).toEqual(expected);

    state.set = loadedSet(e1Set({ notes: [], warnings: [] }));
    const { tree: bare } = await show({ filePath: E1_PATH, fileKey: E1_KEY });
    expect(withProp(bare, "data-tso-notes")).toEqual([]);
    expect(textOf(bare)).not.toContain("notes (");
  });

  it("lets the user pick a series when the set holds several", async () => {
    const set = a5Set();
    state.set = loadedSet(set);

    const { tree } = await show({ filePath: E1_PATH, fileKey: E1_KEY });
    const select = seriesSelect(tree);
    expect(select).toBeDefined();
    const options = elementsOf<Props>(tree).filter((element) => element.type === "option");
    expect(options.map(textOf)).toEqual(["NRS1 (8 integrations x 1271 channels)", "NRS2 (8 integrations x 2048 channels)"]);
    expect(options.map((option) => String(option.props.value))).toEqual(["0", "1"]);
    expect(String(select?.value)).toBe("0");
    const label = elementsOf<Props>(tree).find((element) => element.type === "label" && textOf(element) === "Series");
    expect(label?.props.htmlFor).toBe(select?.id);
    expect(state.children.get("curves")).toMatchObject({ series: set.series[0], group: "tso:a5a5a5a5a5a5a5a5:NRS1:-1" });

    (select?.onChange as (event: { target: { value: string } }) => void)({ target: { value: "1" } });
    expect(state.setSeriesIndex).toHaveBeenCalledWith(E1_KEY, 1);

    state.index = 1;
    const { tree: second } = await show({ filePath: E1_PATH, fileKey: E1_KEY });
    expect(String(seriesSelect(second)?.value)).toBe("1");
    expect(withProp(second, "data-tso-facts").map(textOf)).toEqual([
      "8 integrations x 2048 channels, flux in Jy, time axis BMJD (TDB) (TDB-MID column)",
    ]);
    for (const name of ["curves", "whtlt", "depth"]) {
      expect(state.children.get(name)).toMatchObject({ series: set.series[1], group: "tso:a5a5a5a5a5a5a5a5:NRS2:-1" });
    }

    state.index = 5;
    const { tree: third } = await show({ filePath: E1_PATH, fileKey: E1_KEY });
    expect(String(seriesSelect(third)?.value)).toBe("1");
    expect(state.children.get("depth")).toMatchObject({ series: set.series[1], group: "tso:a5a5a5a5a5a5a5a5:NRS2:-1" });
  });

  it("offers the other segments of the exposure and stitches the ticked ones", async () => {
    state.key = SEG_KEY;
    state.files = [seriesFile(SEG1), seriesFile(SEG2), seriesFile(`C:/d/${EXPOSURE}-seg001_nrs1_calints.fits`, false), seriesFile(SEG3)];

    const { tree, markup } = await show({ filePath: SEG1, fileKey: SEG_KEY });
    const blocks = elementsOf<Props>(tree).filter((element) => element.props["data-tso-segments"] !== undefined);
    expect(blocks).toHaveLength(1);
    expect(textOf(blocks[0])).toContain("Segments of this exposure in Files");
    expect(textOf(blocks[0])).toContain(
      "Ticked segments are read with this file as one series; drop the other segment files in Files to list them here.",
    );
    const boxes = elementsOf<Props>(blocks[0]).filter((element) => element.type === "input" && element.props.type === "checkbox");
    expect(boxes.map((box) => box.props["aria-label"])).toEqual([
      `Stitch ${EXPOSURE}-seg002_nrs1_x1dints.fits`,
      `Stitch ${EXPOSURE}-seg003_nrs1_x1dints.fits`,
    ]);
    expect(boxes.map((box) => box.props.checked)).toEqual([false, false]);
    expect(textOf(blocks[0])).toContain(`${EXPOSURE}-seg002_nrs1_x1dints.fits`);
    expect(markup).not.toContain("calints");
    expect(state.open).toHaveBeenLastCalledWith(SEG_KEY, [SEG1]);

    (boxes[0].props.onChange as (event: { target: { checked: boolean } }) => void)({ target: { checked: true } });
    expect(state.setChosen).toHaveBeenCalledWith(SEG_KEY, [SEG2]);

    state.chosen = [SEG2];
    const { tree: stitched } = await show({ filePath: SEG1, fileKey: SEG_KEY });
    expect(state.open).toHaveBeenLastCalledWith(SEG_KEY, [SEG1, SEG2]);
    const ticked = elementsOf<Props>(stitched).filter((element) => element.type === "input" && element.props.type === "checkbox");
    expect(ticked.map((box) => box.props.checked)).toEqual([true, false]);
    (ticked[1].props.onChange as (event: { target: { checked: boolean } }) => void)({ target: { checked: true } });
    expect(state.setChosen).toHaveBeenLastCalledWith(SEG_KEY, [SEG2, SEG3]);
    (ticked[0].props.onChange as (event: { target: { checked: boolean } }) => void)({ target: { checked: false } });
    expect(state.setChosen).toHaveBeenLastCalledWith(SEG_KEY, []);

    state.set = loadedSet(e1Set(), [SEG1, SEG2]);
    await show({ filePath: SEG1, fileKey: SEG_KEY });
    expect(state.children.get("curves")).toMatchObject({ paths: [SEG1, SEG2] });

    hooks.mount();
    state.open.mockReset();
    state.chosen = [`C:/d/${EXPOSURE}-seg009_nrs1_x1dints.fits`, SEG2];
    await show({ filePath: SEG1, fileKey: SEG_KEY });
    expect(state.open).toHaveBeenCalledTimes(1);
    expect(state.open).toHaveBeenCalledWith(SEG_KEY, [SEG1, SEG2]);

    hooks.mount();
    state.key = `l3|${LEVEL3_PATH}`;
    state.chosen = [];
    state.files = [seriesFile(SEG1), seriesFile(LEVEL3_PATH), seriesFile(SEG2)];
    const { tree: level3 } = await show({ filePath: LEVEL3_PATH, fileKey: `l3|${LEVEL3_PATH}` });
    expect(withProp(level3, "data-tso-segments")).toEqual([]);
    expect(state.open).toHaveBeenLastCalledWith(`l3|${LEVEL3_PATH}`, [LEVEL3_PATH]);
  });

  it("shows the refusal of the reader and no sub-section", async () => {
    state.set = { paths: [E1_PATH], loading: false, result: null, error: LEVEL_MIX };

    const { tree, markup } = await show({ filePath: E1_PATH, fileKey: E1_KEY });

    expect(markup.match(/role="alert"/g)).toHaveLength(1);
    expect(markup).toContain(`role="alert">${LEVEL_MIX}</div>`);
    expect(markup).not.toContain("animate-spin");
    expect(withProp(tree, "data-tso-file")).toEqual([]);
    expect(state.children.size).toBe(0);
  });

  it("marks an incomplete exposure", async () => {
    state.set = loadedSet(
      e1Set({
        series: [
          e1Series([
            { exposure: EXPOSURE, nints: 44, found: 36, missing: [[17, 24]], complete: false },
            { exposure: "jw02327001001_04101_00001", nints: 44, found: 44, missing: [], complete: true },
            { exposure: "jw02327001001_05101_00001", nints: null, found: 8, missing: [], complete: null },
          ]),
        ],
      }),
    );

    const { tree } = await show({ filePath: E1_PATH, fileKey: E1_KEY });
    const lines = withProp(tree, "data-tso-exposure");

    expect(lines.map(textOf)).toEqual([
      `${EXPOSURE}: 36 of 44 integrations, missing 17 to 24`,
      "jw02327001001_04101_00001: 44 of 44 integrations",
      "jw02327001001_05101_00001: 8 integrations (NINTS unknown)",
    ]);
    expect(String(lines[0].props.className)).toContain("text-amber-300");
    expect(String(lines[1].props.className)).not.toContain("amber");
    expect(String(lines[2].props.className)).not.toContain("amber");
  });
});
