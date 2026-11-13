import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { GAIN_TITLE } from "../../../utils/photometryGain";
import type { StarPhotometry } from "../../../services/analysis";
import type { TimeSeriesFrame, TimeSeriesResult } from "../../../shared/types/analysis";
import type { FrameGeometry } from "../../../shared/types/geometry";

const REF = "C:/data/502nmos.fits";
const SECOND = "C:/data/502nmos_2.fits";

const backend = vi.hoisted(() => ({
  regions: new Map<string, unknown[]>(),
  series: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  saved: [] as string[],
  written: [] as string[],
  stored: [] as unknown[],
  visible: [] as unknown[],
  selected: vi.fn(),
  selectedRegion: new Map<string, string>(),
  progress: { percent: 0, active: false },
}));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../context/PreviewContext", () => ({
  useDqContext: () => ({ excludeDq: false }),
  useFileContext: () => ({ file: { path: REF, result: { dimensions: [1600, 1600] } } }),
  useRenderContext: () => ({ processed: null }),
  useDoneFilesContext: () => ({ doneFiles: backend.visible }),
}));
vi.mock("../../../hooks/useRegionStore", () => ({
  useRegionDoc: (key: string | null) => ({
    regions: backend.regions.get(key ?? "") ?? [],
    selectedId: backend.selectedRegion.get(key ?? "") ?? null,
  }),
}));
vi.mock("../../../hooks/useProgress", () => ({ useProgress: () => ({ ...backend.progress, reset: () => {} }) }));
vi.mock("../../../hooks/useAnalysisTarget", () => ({
  useAnalysisTarget: () => ({ composite: false, displayed: { isProcessed: false, label: "502nmos.fits" } }),
}));
vi.mock("../../../hooks/useFileStore", () => ({ useDoneFiles: () => backend.stored, fileStore: { selectFile: backend.selected } }));
vi.mock("../../../services/analysis", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/analysis")>()),
  photometryGainModel: () => new Promise(() => {}),
  timeSeriesPhotometry: backend.series,
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: async (dialog: { defaultPath: string }) => `C:/out/${dialog.defaultPath}` }));
vi.mock("@tauri-apps/plugin-fs", () => ({
  writeTextFile: async (path: string, text: string) => {
    backend.saved.push(path);
    backend.written.push(text);
  },
}));
vi.mock("../../../hooks/usePhotometryStatus", () => ({ usePhotometryStatus: () => ({ loading: false, status: null, error: null }) }));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf } from "../../../test/elementTree";
import { fluxCellText } from "../../../utils/photometryTable";
import { regionStore } from "../../../utils/regionStore";
import { measurementLog, type MeasurementLogEntry } from "../../../utils/measurementLog";
import { lightCurveStore } from "../../../hooks/useLightCurveStore";
import { docTimeLabel, frameGroupKey, tableGroupKey, timeOffset } from "../../../utils/lightCurveDoc";
import type { LightCurveDoc } from "../../../shared/types/lightCurve";
import { lightCurveTableLoader } from "../../../hooks/useLightCurveTable";
import { APP_VERSION } from "../../../utils/constants";
import { tableRouteStore } from "../../../hooks/useTableRouteStore";
import type { TableProbe } from "../../../shared/types/tables";
import timeSeriesPanelSource from "../TimeSeriesPanel.tsx?raw";
import { localSmallButtonLines } from "../../../test/panelRules";
import { CopyCsvButton, ErrorAlert } from "../../ui";

const { default: TimeSeriesPanel } = await import("../TimeSeriesPanel");
const { default: LightCurveTablePanel } = await import("../LightCurveTablePanel");
const { default: PhotometryStatusLine } = await import("../PhotometryStatusLine");

const LIGHT_HEADER = { IMAGETYP: "'Light Frame'", FILTER: "'V'" };

function doneFrame(id: string, path: string, header: Record<string, string>) {
  return { id, path, name: path.split("/").pop(), status: "done", result: { dimensions: [1600, 1600], header } };
}

function lights() {
  return [doneFrame("a", REF, { ...LIGHT_HEADER, BUNIT: "'ADU'" }), doneFrame("b", SECOND, LIGHT_HEADER)];
}

backend.stored = lights();
backend.visible = backend.stored;

function render(): string {
  return renderToStaticMarkup(createElement(TimeSeriesPanel, { filePath: REF }));
}

describe("TimeSeriesPanel fields (followups-9, followups-10, science0925-19)", () => {
  it("renders the aperture, sky and gain fields as decimal text boxes, never native number inputs", () => {
    const markup = render();
    expect(markup).not.toContain('type="number"');
    expect(markup.match(/inputMode="decimal"/g)).toHaveLength(6);
    expect(markup).toContain('value="5"');
  });

  it("marks the gain field with the photometry gain title and shows the header-gain caption slot under it", () => {
    const markup = render();
    expect(markup).toContain(`title="${GAIN_TITLE}"`);
    expect(markup).toContain("data-gain-source");
    expect(markup).toContain("data-gain-caption");
  });
});

type Props = Record<string, unknown> & { children?: ReactNode };

function elements(node: ReactNode): Props[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<Props>(node)) return [];
  return [node.props, ...elements(node.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<Props>(node) ? textOf(node.props.children) : "";
}

function ofType(node: ReactNode, type: string): Props[] {
  if (Array.isArray(node)) return node.flatMap((child) => ofType(child, type));
  if (!isValidElement<Props>(node)) return [];
  return [...(node.type === type ? [node.props] : []), ...ofType(node.props.children, type)];
}

function star(flux: number, bg: number): StarPhotometry {
  return {
    x: 10, y: 10, peak: 100, net_flux: flux, flux_err: 5, mag_inst: -2.5 * Math.log10(flux), snr: flux / 5, fwhm: 3,
    aperture_radius: 5, aperture_pixels: 78, aperture_area: 78.5, bg_mean: bg, bg_sigma: 1, bg_pixels: 300,
    saturated: false, saturation_source: "none", n_masked: 0, n_saturated: 0, err_used: false, aperture_correction: null,
    flux_total: null, plateau_radius: null, flux_jy: null, flux_err_jy: null, mag_ab: null, mag_ab_err: null,
    mag_ab_total: null, st_mag: null, sky_inner: 10, sky_outer: 15, growth_curve: [], ee50_radius: null, ee80_radius: null,
    n_nonfinite: 0, ee_in_aperture: null, warnings: [],
  } as StarPhotometry;
}

function geometry(jdUtc: number): FrameGeometry {
  return {
    jd_utc: jdUtc, jd_tt: jdUtc + 69.184 / 86400, jd_tdb: jdUtc + 69.184 / 86400, bjd_tdb: jdUtc + 0.0041234,
    hjd_utc: jdUtc + 0.0033211, bjd_source: "computed", lst_deg: 100, hour_angle_deg: 10, altitude_deg: 60, azimuth_deg: 200,
    airmass_computed: 1.15, airmass_formula: "Kasten & Young 1989", parallactic_angle_deg: 0, sun_altitude_deg: -30,
    moon_altitude_deg: 5, moon_illumination: 0.5, moon_separation_deg: 80, time_scale_notes: [],
  } as FrameGeometry;
}

function frame(index: number, path: string, targetFlux: number, bunit: string | null = "ADU"): TimeSeriesFrame {
  return {
    index, path, file_name: path.split("/").pop()!, jd_mid: 2460310.6 + index * 0.001,
    time_source: "DATE-OBS + EXPTIME/2 (60 s); UTC assumed (no TIMESYS)", exptime: 60, filter: "V", airmass: 1.2,
    geometry: geometry(2460310.6 + index * 0.001), offset: null, photcal_label: null, bunit,
    targets: [star(targetFlux, 1234.5678), star(1000, 10)], errors: [null, null], skipped: null,
  };
}

const RESULT: TimeSeriesResult = {
  reference_path: REF,
  targets: [
    { x: 10, y: 10, label: "P1", role: "target" },
    { x: 40, y: 40, label: "P2", role: "comp" },
  ],
  frames: [frame(0, REF, 500), frame(1, SECOND, 480)],
  n_frames: 2,
  n_skipped: 0,
  warnings: [],
  area_mixed: false,
  geometry_target: null,
  geometry_notes: [],
  elapsed_ms: 5,
};

const point = (id: string, x: number, y: number, text = "") => ({ id, shape: { shape: "point", x, y }, props: { text } });

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

type Panel = (props: { filePath: string }) => unknown;
const panel = (TimeSeriesPanel as unknown as { type: Panel }).type;
const show = (filePath = REF) => hooks.render(panel, { filePath }) as ReactNode;

async function measured(): Promise<ReactNode> {
  const run = elements(show()).find((p) => typeof p.label === "string" && p.label.startsWith("Measure 2"));
  expect(run?.disabled, "Measure is enabled").toBe(false);
  (run!.onClick as () => void)();
  await flush();
  return show();
}

const blockers = (tree: ReactNode) => elements(tree).filter((p) => p["data-export-blocker"]).map((p) => textOf(p.children as ReactNode));
const leftOutNotes = (tree: ReactNode) => elements(tree).filter((p) => p["data-left-out-note"]).map((p) => textOf(p.children as ReactNode));
const exportButton = (tree: ReactNode, format: string) => elements(tree).find((p) => p["data-export"] === format);
const buttonNamed = (tree: ReactNode, name: string) =>
  elements(tree).find((p) => typeof p.onClick === "function" && p.children !== undefined && textOf(p.children as ReactNode).trim() === name);
const runButton = (tree: ReactNode) => elements(tree).find((p) => typeof p.label === "string" && p.label.startsWith("Measure "));
const viewOf = (tree: ReactNode) => elements(tree).find((p) => typeof p.group === "string" && p.group.startsWith("frames:"));
const storedDocs = () => lightCurveStore.get(frameGroupKey(REF));
const roleOf = (tree: ReactNode, label: string) => elements(tree).find((p) => p["aria-label"] === `Role of ${label}`)?.value;
const fieldLabelled = (tree: ReactNode, label: string) => {
  const caption = elements(tree).find((p) => typeof p.htmlFor === "string" && textOf(p.children as ReactNode).trim() === label);
  return elements(tree).find((p) => p.id !== undefined && p.id === caption?.htmlFor);
};
const typeInto = (label: string, text: string) => (fieldLabelled(show(), label)!.onChange as (t: string) => void)(text);
const fieldErrorText = (tree: ReactNode) => textOf(elements(tree).find((p) => p["data-field-error"])?.children as ReactNode);

function withGeometry(result: TimeSeriesResult, patch: Partial<FrameGeometry>): TimeSeriesResult {
  return { ...result, frames: result.frames.map((f) => ({ ...f, geometry: { ...f.geometry!, ...patch } })) };
}

const TABLE_PATH = "C:/d/a.csv";
const TABLE_GROUP = tableGroupKey(TABLE_PATH);

function tableDoc(): LightCurveDoc {
  return {
    id: "flux",
    label: "flux (a.csv)",
    quantity: "flux",
    unit: null,
    timeScale: "mjd_utc",
    time: [60061.1, 60061.2],
    value: [1, 0.99],
    err: [null, null],
    pointRef: [0, 1],
    flags: [[], []],
    source: { kind: "table", path: TABLE_PATH, time_column: "mjd", value_column: "flux" },
    notes: [],
  };
}

function probeOf(kind: TableProbe["guess"]["kind"]): TableProbe {
  return {
    format: "ecsv",
    n_rows: 2,
    columns: [],
    tables: null,
    meta: null,
    comments: [],
    guess: { kind, reason: "time column MJD_UTC_NRS1 and value column whitelight_flux_NRS1" },
    notes: [],
  };
}

describe("TimeSeriesPanel after a run", () => {
  beforeEach(() => {
    vi.stubGlobal("window", { localStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {} } });
    hooks.mount();
    backend.regions = new Map([[REF, [point("r1", 10, 10), point("r2", 40, 40)]]]);
    backend.series.mockReset().mockResolvedValue(RESULT);
    backend.saved = [];
    backend.written = [];
    backend.stored = lights();
    backend.visible = backend.stored;
    lightCurveStore.remove(frameGroupKey(REF));
    lightCurveStore.remove(frameGroupKey(SECOND));
    lightCurveStore.setActiveTable(null);
    lightCurveStore.remove(TABLE_GROUP);
    tableRouteStore.reset();
    backend.selected.mockReset();
    backend.selectedRegion.clear();
    backend.progress = { percent: 0, active: false };
  });

  afterEach(() => {
    hooks.mount();
    vi.restoreAllMocks();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("names a drawn Point region from the Stars list so ETD and ExoClock unlock (flows-2)", async () => {
    const update = vi.spyOn(regionStore, "update").mockImplementation((key, id, patch) => {
      const regions = (backend.regions.get(key) ?? []) as { id: string }[];
      backend.regions.set(key, regions.map((r) => (r.id === id ? { ...r, ...patch } : r)));
    });
    const tree = await measured();
    const field = elements(tree).find((p) => p["aria-label"] === "Name of P1");
    expect(field, "the name field of P1").toBeDefined();
    expect(exportButton(tree, "etd")?.disabled).toBe(true);
    const blur = vi.fn();
    (field!.onKeyDown as (e: unknown) => void)({ key: "Enter", currentTarget: { blur } });
    expect(blur).toHaveBeenCalledTimes(1);
    (field!.onBlur as (e: unknown) => void)({ currentTarget: { value: "  " } });
    expect(update).not.toHaveBeenCalled();
    (field!.onBlur as (e: unknown) => void)({ currentTarget: { value: " WASP-12 " } });
    expect(update).toHaveBeenCalledWith(REF, "r1", { props: expect.objectContaining({ text: "WASP-12" }) });
    const named = show();
    expect(blockers(named).join(" ")).not.toContain("no target name");
    expect(exportButton(named, "etd")?.disabled).toBe(false);
    expect(exportButton(named, "exoclock")?.disabled).toBe(false);
    expect(elements(named).find((p) => p["aria-label"] === "Name of WASP-12")?.defaultValue).toBe("WASP-12");
  });

  it("flags a saturated comparison star: its frame is left out of ETD and ExoClock while flagged points are left out, the exports are blocked when they are not, and one click leaves the star out for the whole series (front-photometry/B1, timeseries/U6)", async () => {
    const saturatedRun: TimeSeriesResult = {
      ...RESULT,
      targets: [...RESULT.targets, { x: 70, y: 70, label: "P3", role: "comp" }],
      frames: [
        { ...RESULT.frames[0], targets: [star(500, 10), star(1000, 10), star(1000, 10)], errors: [null, null, null] },
        { ...RESULT.frames[1], targets: [star(480, 10), { ...star(900, 10), saturated: true }, star(1000, 10)], errors: [null, null, null] },
      ],
    };
    backend.regions = new Map([[REF, [point("r1", 10, 10, "WASP-12"), point("r2", 40, 40), point("r3", 70, 70)]]]);
    backend.series.mockReset().mockResolvedValue(saturatedRun);
    const chips = (tree: ReactNode) => elements(tree).filter((p) => p.children === "comp P2 saturated");
    const tree = await measured();
    expect(chips(tree)).toHaveLength(1);
    expect(blockers(tree)).toEqual([]);
    expect(exportButton(tree, "etd")?.disabled).toBe(false);
    expect(leftOutNotes(tree)).toEqual(["ETD and ExoClock leave out 1 frame (1 flagged)"]);
    expect(textOf(tree)).toContain("comparison star P2 saturated on 1 frame");
    expect(textOf(tree)).toContain(`${(-2.5 * Math.log10(480 / 1900)).toFixed(4)} ±`);
    lightCurveStore.patchView(frameGroupKey(REF), { excludeFlagged: false });
    const kept = show();
    expect(blockers(kept)).toEqual(["ETD and ExoClock: comparison star P2 saturated on 502nmos_2.fits (leave it out of the ensemble)"]);
    expect(exportButton(kept, "etd")?.disabled).toBe(true);
    expect(leftOutNotes(kept)).toEqual([]);
    (buttonNamed(kept, "Leave P2 out")!.onClick as () => void)();
    const leftOut = show();
    expect(chips(leftOut)).toHaveLength(0);
    expect(blockers(leftOut)).toEqual([]);
    expect(exportButton(leftOut, "etd")?.disabled).toBe(false);
    expect(leftOutNotes(leftOut)).toEqual([]);
    expect(textOf(leftOut)).toContain("comparison star P2 saturated on 1 frame, left out of the ensemble");
    expect(textOf(leftOut)).toContain(`${(-2.5 * Math.log10(0.48)).toFixed(4)} ±`);
    (buttonNamed(leftOut, "Save CSV")!.onClick as () => void)();
    await vi.waitFor(() => expect(backend.written).toHaveLength(1));
    expect(backend.written[0].split("\n").slice(4, 6)).toEqual(["# comparison stars: P3", "# comparison stars left out: P2"]);
    (buttonNamed(leftOut, "Put P2 back")!.onClick as () => void)();
    expect(chips(show())).toHaveLength(1);
  });

  it("explains a missing BJD_TDB axis with the frames' own note and lists the reference frame's time notes (timeseries/D1)", async () => {
    const hst =
      "TELESCOP HST is a spacecraft: BJD_TDB and HJD_UTC need the spacecraft ephemeris and the header carries no barycentric mid-exposure time (TDB-MID or BMIDTIME)";
    backend.series.mockReset().mockResolvedValue(withGeometry(RESULT, { bjd_tdb: null, hjd_utc: null, bjd_source: null, time_scale_notes: [hst] }));
    const tree = await measured();
    expect(textOf(tree)).toContain(`no BJD_TDB for 502nmos.fits (${hst})`);
    expect(textOf(tree)).not.toContain("BJD_TDB needs a time, a target and no skipped-geometry frame");
    expect(textOf(tree)).toContain("Geometry notes");
    expect(elements(tree).filter((p) => p.children === hst)).toHaveLength(1);
  });

  it("labels the JD axis of a series whose header times are barycentric as barycentric (timeseries/D2)", async () => {
    backend.series.mockReset().mockResolvedValue(withGeometry(RESULT, { bjd_source: "header", hjd_utc: null }));
    const tree = await measured();
    const axis = elements(tree).find((p) => typeof p.onChange === "function" && p.value === "bjd");
    (axis!.onChange as (e: unknown) => void)({ target: { value: "jd" } });
    show();
    const doc = storedDocs()[0];
    expect(doc.timeLabel).toBe("JD (header time, barycentric)");
    expect(docTimeLabel(doc, timeOffset(doc))).toBe("JD (header time, barycentric) - 2460310");
  });

  it("hands the measured curves to the light-curve store, one document per series choice", async () => {
    const tree = await measured();
    const docs = storedDocs();
    expect(docs.map((d) => d.id)).toEqual(["target", "raw:1"]);
    expect(docs[0].label).toBe("target P1");
    expect(docs[0].timeScale).toBe("bjd_tdb");
    expect(docs[0].time[0]).toBeCloseTo(2460310.6041234, 7);
    expect(docs[0].value[0]).toBeCloseTo(0.7525749891599529, 12);
    expect(docs[0].pointRef).toEqual([0, 1]);
    expect(docs[1].label).toBe("P2");
    expect(docs[1].overlay).toBe("raw comps");
    expect(docs[1].value).toEqual([1000, 1000]);
    expect(viewOf(tree)?.group).toBe("frames:C:/data/502nmos.fits");
    expect(viewOf(tree)?.csvName).toBe("lightcurve");
  });

  it("leaving a comparison star out rebuilds the stored curve on the smaller ensemble", async () => {
    const threeStarRun: TimeSeriesResult = {
      ...RESULT,
      targets: [...RESULT.targets, { x: 70, y: 70, label: "P3", role: "comp" }],
      frames: [
        { ...RESULT.frames[0], targets: [star(500, 10), star(1000, 10), star(1000, 10)], errors: [null, null, null] },
        { ...RESULT.frames[1], targets: [star(480, 10), { ...star(900, 10), saturated: true }, star(1000, 10)], errors: [null, null, null] },
      ],
    };
    backend.regions = new Map([[REF, [point("r1", 10, 10, "WASP-12"), point("r2", 40, 40), point("r3", 70, 70)]]]);
    backend.series.mockReset().mockResolvedValue(threeStarRun);
    const tree = await measured();
    const sourceComps = () => (storedDocs()[0].source as { comps: string[] }).comps;
    const compIds = () => storedDocs().map((d) => d.id).filter((id) => id.startsWith("comp:"));
    expect(sourceComps()).toEqual(["P2", "P3"]);
    expect(compIds()).toEqual(["comp:1", "comp:2"]);
    (buttonNamed(tree, "Leave P2 out")!.onClick as () => void)();
    show();
    expect(sourceComps()).toEqual(["P3"]);
    expect(compIds()).toEqual([]);
    expect(storedDocs()[0].value[1]).toBeCloseTo(-2.5 * Math.log10(480 / 1000), 12);
  });

  it("a click on a point of the view shows that frame, and the frame on screen is the marked point", async () => {
    const tree = await measured();
    const onSelectPoint = viewOf(tree)!.onSelectPoint as (doc: LightCurveDoc, pointRef: number) => void;
    onSelectPoint(storedDocs()[0], 1);
    expect(backend.selected).toHaveBeenCalledWith("b");
    expect(viewOf(show())?.selectedPointRef).toBe(0);
    expect(viewOf(show(SECOND))?.selectedPointRef).toBe(1);
  });

  it("shows a curve measured earlier in the session when the panel starts again on its reference frame", async () => {
    await measured();
    hooks.mount();
    const tree = show();
    expect(textOf(tree)).toContain("Measured earlier in this session. Measure again for the table and the exports.");
    expect(viewOf(tree)?.group).toBe(frameGroupKey(REF));
    expect(viewOf(tree)?.onSelectPoint).toBeUndefined();
    expect(textOf(tree)).not.toContain("Table");
    hooks.mount();
    expect(textOf(show(SECOND))).not.toContain("Measured earlier in this session.");
  });

  it("saves the light-curve CSV with its provenance, the reference frame's BUNIT and the sky at full precision (trust-2)", async () => {
    await measured();
    (buttonNamed(show(), "Save CSV")!.onClick as () => void)();
    await vi.waitFor(() => expect(backend.written).toHaveLength(1));
    const lines = backend.written[0].split("\n");
    expect(lines[0]).toBe(`# software: AstroBurst ${APP_VERSION}`);
    expect(lines[1].startsWith("# exported_utc: ")).toBe(true);
    expect(lines[2]).toBe("# file: 502nmos.fits");
    expect(lines.slice(3, 6)).toEqual(["# target: P1", "# comparison stars: P2", "# reference frame: 502nmos.fits"]);
    expect(lines.find((l) => l.startsWith("# flux:"))?.endsWith("; pixel values in BUNIT ADU")).toBe(true);
    const header = lines.findIndex((l) => l.startsWith("index,"));
    const columns = lines[header].split(",");
    expect(lines[header + 1].split(",")[columns.indexOf("bg_mean")]).toBe("1234.5678");
  });

  it("prints the sky level through the shared flux formatter", async () => {
    const sky = elements(await measured()).filter((p) => p["data-column"] === "bg_mean");
    expect(sky.map((p) => textOf(p.children as ReactNode))).toEqual([fluxCellText(1234.5678), fluxCellText(1234.5678)]);
  });

  it("writes the differential magnitude error with ± on one line, as the photometry table does", async () => {
    const tree = await measured();
    expect(textOf(tree)).not.toContain("+/-");
    const cells = elements(tree).filter((p) => typeof p.children === "string" && p.children.includes(" ± "));
    expect(cells.length).toBe(2);
    for (const cell of cells) expect(String(cell.className).split(" ")).toContain("whitespace-nowrap");
  });

  it("keeps the target name typed on the reference frame for ETD and ExoClock when another frame of the run is shown", async () => {
    await measured();
    expect(blockers(show()).join(" ")).toContain('no target name: label the target\'s Point region with the star\'s designation (now "P1")');
    backend.regions.set(REF, [point("r1", 10, 10, "WASP-12"), point("r2", 40, 40)]);
    expect(blockers(show()).join(" ")).not.toContain("no target name");
    expect(blockers(show(SECOND)).join(" ")).not.toContain("no target name");
  });

  it("shows each save in one timed notice that a later save restarts", async () => {
    await measured();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"], shouldAdvanceTime: true });
    const saveCsv = () => (elements(show()).find((p) => p.children !== undefined && textOf(p.children as ReactNode).trim() === "Save CSV")!.onClick as () => void)();
    const notice = () => elements(show()).find((p) => typeof p.className === "string" && p.className.includes("text-emerald-400/90"));
    saveCsv();
    await vi.waitFor(() => expect(backend.saved).toHaveLength(1));
    expect(textOf(notice()?.children as ReactNode)).toBe(`Saved ${backend.saved[0]}`);
    await vi.advanceTimersByTimeAsync(3_000);
    saveCsv();
    await vi.waitFor(() => expect(backend.saved).toHaveLength(2));
    await vi.advanceTimersByTimeAsync(5_000);
    expect(notice(), "the second save is still shown 5 s after it").toBeDefined();
    await vi.advanceTimersByTimeAsync(1_500);
    expect(notice()).toBeUndefined();
  });

  it("seventy stars from the table do not block Measure", () => {
    backend.regions = new Map([[REF, Array.from({ length: 70 }, (_, i) => point(`s${i + 1}`, i + 1, i + 1, `star ${i + 1}`))]]);
    const tree = show();
    expect(roleOf(tree, "star 1")).toBe("target");
    expect(roleOf(tree, "star 64")).toBe("comp");
    expect(roleOf(tree, "star 65")).toBe("ignore");
    expect(roleOf(tree, "star 70")).toBe("ignore");
    const note = elements(tree).find((p) => p["data-ignored-note"]);
    expect(textOf(note?.children as ReactNode)).toBe("6 stars beyond the limit of 64 start as ignore; change a role to measure one.");
    expect(runButton(tree)?.disabled).toBe(false);
  });

  it("the star selected in the viewer is marked and one choice makes it the target", () => {
    backend.regions = new Map([[REF, [point("r1", 10, 10), point("r2", 40, 40), point("r3", 70, 70)]]]);
    backend.selectedRegion.set(REF, "r2");
    const tree = show();
    const marked = elements(tree).filter((p) => p["data-selected-star"]);
    expect(marked).toHaveLength(1);
    expect(String(marked[0].className)).toContain("bg-zinc-800/60 rounded");
    expect(elements(marked[0].children as ReactNode).some((p) => p["aria-label"] === "Name of P2")).toBe(true);
    const role = elements(tree).find((p) => p["aria-label"] === "Role of P2");
    (role!.onChange as (e: unknown) => void)({ target: { value: "target" } });
    const chosen = show();
    expect(roleOf(chosen, "P2")).toBe("target");
    expect(roleOf(chosen, "P1")).toBe("comp");
  });

  it("while another frame of the run is shown the Stars list is the reference frame's and Measure stays enabled", async () => {
    await measured();
    const tree = show(SECOND);
    const names = elements(tree).map((p) => p["aria-label"]).filter((label) => typeof label === "string" && label.startsWith("Name of "));
    expect(names).toEqual(["Name of P1", "Name of P2"]);
    expect(textOf(tree)).toContain("Stars of 502nmos.fits");
    expect(textOf(tree)).not.toContain(
      "Draw Point regions on the target and comparison stars, or use Add as Point regions in the photometry table.",
    );
    expect(runButton(tree)?.disabled).toBe(false);
    (runButton(tree)!.onClick as () => void)();
    await flush();
    expect(backend.series).toHaveBeenCalledTimes(2);
    expect(backend.series.mock.calls[1][0]).toEqual([REF, SECOND]);
    (buttonNamed(show(SECOND), "Show reference frame")!.onClick as () => void)();
    expect(backend.selected).toHaveBeenCalledWith("a");
  });

  it("a file outside the run still shows its own stars", async () => {
    await measured();
    show("C:/data/other.fits");
    const tree = show("C:/data/other.fits");
    expect(viewOf(tree)).toBeUndefined();
    expect(elements(tree).some((p) => p.children === "Stars")).toBe(true);
    expect(textOf(tree)).not.toContain("Stars of");
    expect(textOf(tree)).toContain(
      "Draw Point regions on the target and comparison stars, or use Add as Point regions in the photometry table.",
    );
  });

  it("typed target coordinates go to the command and a wrong value blocks Measure", async () => {
    (runButton(show())!.onClick as () => void)();
    await flush();
    expect(backend.series.mock.calls[0][2]).toMatchObject({ targetRa: null, targetDec: null });
    typeInto("target RA deg", "290.5");
    typeInto("target Dec deg", "53.5");
    (runButton(show())!.onClick as () => void)();
    await flush();
    expect(backend.series.mock.calls[1][2]).toMatchObject({ targetRa: 290.5, targetDec: 53.5 });
    typeInto("target Dec deg", "");
    expect(fieldErrorText(show())).toBe("target override needs both RA and Dec");
    expect(runButton(show())?.disabled).toBe(true);
    typeInto("target RA deg", "400");
    typeInto("target Dec deg", "10");
    expect(fieldErrorText(show())).toBe("RA 400 must be a decimal of at least 0 and below 360 degrees");
    expect(runButton(show())?.disabled).toBe(true);
  });

  it("the light-time target that was used is shown in the time card", async () => {
    backend.series.mockReset().mockResolvedValue({ ...RESULT, geometry_target: { ra_deg: 290.123456789, dec_deg: 53.5, source: "target override" } });
    const line = elements(await measured()).find((p) => p["data-light-time-target"]);
    const text = "light-time target RA 290.123457 Dec 53.500000 deg (target override)";
    expect(textOf(line?.children as ReactNode)).toBe(text);
    expect(line?.title).toBe(text);
    hooks.mount();
    backend.series.mockReset().mockResolvedValue(RESULT);
    expect(elements(await measured()).some((p) => p["data-light-time-target"])).toBe(false);
  });

  it("progress and the end of a run are exposed to assistive technology", async () => {
    backend.progress = { percent: 42.4, active: true };
    let finish: (value: unknown) => void = () => {};
    backend.series.mockReset().mockReturnValue(new Promise((resolve) => (finish = resolve)));
    (runButton(show())!.onClick as () => void)();
    const bars = elements(show()).filter((p) => p.role === "progressbar");
    expect(bars).toHaveLength(1);
    expect(bars[0]).toMatchObject({
      "aria-valuenow": 42,
      "aria-valuemin": 0,
      "aria-valuemax": 100,
      "aria-label": "Time series progress",
    });
    finish(RESULT);
    await flush();
    const done = show();
    const status = elements(done).find((p) => p["data-series-status"]);
    expect(status?.role).toBe("status");
    expect(textOf(status?.children as ReactNode)).toBe("Measured 2 frames, 0 skipped");
    (buttonNamed(done, "Save CSV")!.onClick as () => void)();
    await vi.waitFor(() => expect(backend.saved).toHaveLength(1));
    const saved = elements(show()).find((p) => textOf(p.children as ReactNode) === "Saved C:/out/502nmos_lightcurve.csv");
    expect(saved?.role).toBe("status");
  });

  it("leaving a comparison star out writes a new log row for the smaller ensemble, and putting it back another", async () => {
    const appended = vi.spyOn(measurementLog, "append").mockImplementation((draft) => draft as MeasurementLogEntry);
    const threeStarRun: TimeSeriesResult = {
      ...RESULT,
      targets: [...RESULT.targets, { x: 70, y: 70, label: "P3", role: "comp" }],
      frames: [
        { ...RESULT.frames[0], targets: [star(500, 10), star(1000, 10), star(1000, 10)], errors: [null, null, null] },
        { ...RESULT.frames[1], targets: [star(480, 10), { ...star(900, 10), saturated: true }, star(1000, 10)], errors: [null, null, null] },
      ],
    };
    backend.regions = new Map([[REF, [point("r1", 10, 10, "WASP-12"), point("r2", 40, 40), point("r3", 70, 70)]]]);
    backend.series.mockReset().mockResolvedValue(threeStarRun);
    const row = (call: number) => appended.mock.calls[call][0];
    const tree = await measured();
    expect(appended).toHaveBeenCalledTimes(1);
    expect(row(0).params.n_comp).toBe(2);
    expect(row(0).params.comps_left_out).toBeNull();
    (buttonNamed(tree, "Leave P2 out")!.onClick as () => void)();
    const leftOut = show();
    expect(appended).toHaveBeenCalledTimes(2);
    expect(row(1).params.n_comp).toBe(1);
    expect(row(1).params.comps_left_out).toBe("P2");
    expect(row(1).notes).toContain("comparison star P2 saturated on 1 frame, left out of the ensemble");
    expect(row(1).notes).toContain("ensemble: 1 of 2 comparison stars (left out: P2)");
    (buttonNamed(leftOut, "Put P2 back")!.onClick as () => void)();
    show();
    expect(appended).toHaveBeenCalledTimes(3);
    expect(row(2).params.comps_left_out).toBeNull();
  });

  it("the Geometry notes list what only some frames say", async () => {
    const local = "TIMESYS LOCAL unknown: treated as UTC";
    const second = RESULT.frames[1];
    backend.series.mockReset().mockResolvedValue({ ...RESULT, frames: [RESULT.frames[0], { ...second, geometry: { ...second.geometry!, time_scale_notes: [local] } }] });
    const tree = await measured();
    expect(textOf(tree)).toContain("Geometry notes");
    expect(elements(tree).filter((p) => p.children === `${local} (1 of 2 frames)`)).toHaveLength(1);
  });

  const headers = (tree: ReactNode) => ofType(tree, "th").map((p) => textOf(p.children as ReactNode));
  const tableRows = (tree: ReactNode) => ofType(ofType(tree, "tbody")[0]?.children as ReactNode, "tr");
  const saturatedOnSecond = (): TimeSeriesResult => ({
    ...RESULT,
    frames: [RESULT.frames[0], { ...RESULT.frames[1], targets: [{ ...star(480, 1234.5678), saturated: true }, star(1000, 10)] }],
  });

  it("the table headers follow the time axis and carry units", async () => {
    const tree = await measured();
    expect(headers(tree)).toEqual([
      "#", "file", "JD [d]", "BJD_TDB - 2460310 [d]", "airmass", "diff mag [mag]", "SNR", "FWHM [px]", "sky [ADU]", "dx/dy [px]", "flags",
    ]);
    const axis = elements(tree).find((p) => typeof p.onChange === "function" && p.value === "bjd");
    (axis!.onChange as (e: unknown) => void)({ target: { value: "jd" } });
    expect(headers(show()).slice(2, 4)).toEqual(["JD - 2460310 [d]", "BJD_TDB [d]"]);
  });

  it("frames in different units are named under the table and the sky header says so", async () => {
    const unitNotes = (tree: ReactNode) => elements(tree).filter((p) => p["data-unit-note"]);
    backend.series.mockReset().mockResolvedValue({ ...RESULT, frames: [frame(0, REF, 500), frame(1, SECOND, 480, "electron")] });
    const mixed = await measured();
    expect(headers(mixed)).toContain("sky [mixed units]");
    expect(unitNotes(mixed).map((p) => textOf(p.children as ReactNode))).toEqual(["frames have different BUNIT: ADU (1), electron (1)"]);
    hooks.mount();
    backend.series.mockReset().mockResolvedValue({ ...RESULT, frames: [frame(0, REF, 500, null), frame(1, SECOND, 480, null)] });
    const none = await measured();
    expect(headers(none)).toContain("sky");
    expect(unitNotes(none)).toEqual([]);
    (buttonNamed(none, "Save CSV")!.onClick as () => void)();
    await vi.waitFor(() => expect(backend.written).toHaveLength(1));
    expect(backend.written[0].split("\n").find((l) => l.startsWith("# flux:"))?.endsWith("; pixel values in BUNIT ADU")).toBe(true);
  });

  it("every row has a button that shows its frame (a11y-2)", async () => {
    const tree = await measured();
    const buttons = elements(tree).filter((p) => typeof p["aria-label"] === "string" && p["aria-label"].startsWith("Show frame "));
    expect(buttons.map((p) => p["aria-label"])).toEqual(["Show frame 0", "Show frame 1"]);
    expect(buttons.map((p) => p.type)).toEqual(["button", "button"]);
    expect(buttons[1].title).toBe("Show this frame in the viewer");
    (buttons[1].onClick as () => void)();
    expect(backend.selected).toHaveBeenCalledWith("b");
  });

  it("flag chips are as large as the table text (visual-10)", async () => {
    backend.series.mockReset().mockResolvedValue(saturatedOnSecond());
    const chip = elements(await measured()).find((p) => p.children === "saturated");
    expect(String(chip?.className)).toContain("text-[9px]");
    expect(String(chip?.className)).not.toContain("text-[8px]");
  });

  it("frames the view leaves out are marked in the table", async () => {
    const chipTexts = (row: Props) => elements(row.children as ReactNode).map((p) => p.children).filter((c) => typeof c === "string");
    await measured();
    lightCurveStore.patchView(frameGroupKey(REF), { excluded: [1] });
    const byHand = tableRows(show());
    expect(byHand[1]["data-left-out"]).toBe("manual");
    expect(chipTexts(byHand[1])).toContain("left out by hand");
    expect(byHand[0]["data-left-out"]).toBeUndefined();
    hooks.mount();
    lightCurveStore.remove(frameGroupKey(REF));
    backend.series.mockReset().mockResolvedValue(saturatedOnSecond());
    const flagged = tableRows(await measured());
    expect(flagged[1]["data-left-out"]).toBe("flagged");
    expect(chipTexts(flagged[1])).not.toContain("left out by hand");
    expect(chipTexts(flagged[1])).not.toContain("clipped");
    lightCurveStore.patchView(frameGroupKey(REF), { excludeFlagged: false });
    expect(tableRows(show())[1]["data-left-out"]).toBeUndefined();
  });

  const named = () => new Map([[REF, [point("r1", 10, 10, "WASP-12"), point("r2", 40, 40)]]]);

  it("the ETD tooltip says the comparison is the sum of the stars", async () => {
    backend.regions = named();
    const etd = exportButton(await measured(), "etd");
    expect(etd?.disabled).toBe(false);
    expect(String(etd?.title)).toContain("against the sum of the comparison stars, 2.5 log10(N) mag above Siril's mean-of-comparisons value");
  });

  it("the time card states the accuracy of a computed BJD_TDB", async () => {
    const accuracy = (tree: ReactNode) => elements(tree).filter((p) => p["data-bjd-accuracy"]);
    const computed = accuracy(await measured());
    expect(computed).toHaveLength(1);
    expect(computed[0].title).toBe("BJD_TDB: computed values use a low-precision ephemeris, within 1 s of astropy");
    hooks.mount();
    backend.series.mockReset().mockResolvedValue(withGeometry(RESULT, { bjd_source: "TDB-MID" }));
    expect(accuracy(await measured())).toEqual([]);
  });

  it("a frame left out by hand in the view is not written to ExoClock", async () => {
    backend.regions = named();
    await measured();
    lightCurveStore.patchView(frameGroupKey(REF), { excluded: [1] });
    const tree = show();
    expect(leftOutNotes(tree)).toEqual(["ETD and ExoClock leave out 1 frame (1 by hand)"]);
    (exportButton(tree, "exoclock")!.onClick as () => void)();
    await vi.waitFor(() => expect(backend.written).toHaveLength(1));
    const lines = backend.written[0].trimEnd().split("\n");
    const header = lines.findIndex((l) => l.includes(",rel_flux,"));
    expect(lines.slice(header + 1)).toHaveLength(1);
    expect(lines).toContain("# left out: 1 frame (1 by hand)");
  });

  it("the table shows -- for an unmeasured FWHM", async () => {
    const unmeasured: TimeSeriesResult = {
      ...RESULT,
      frames: [{ ...RESULT.frames[0], targets: [{ ...star(500, 1234.5678), fwhm: null }, star(1000, 10)] }, RESULT.frames[1]],
    };
    backend.series.mockReset().mockResolvedValue(unmeasured);
    const tree = await measured();
    const column = headers(tree).indexOf("FWHM [px]");
    const cells = ofType(tableRows(tree)[0].children as ReactNode, "td");
    expect(textOf(cells[column].children as ReactNode)).toBe("--");
  });

  it("an active table replaces the frame series in the Series section", () => {
    lightCurveStore.put(TABLE_GROUP, [tableDoc()]);
    lightCurveStore.setActiveTable(TABLE_GROUP);
    const tree = show();
    expect(isValidElement(tree) ? tree.type : tree).toBe(LightCurveTablePanel);
    expect(runButton(tree)).toBeUndefined();
    lightCurveStore.setActiveTable(null);
    const frames = show();
    expect(isValidElement(frames) ? frames.type : frames).toBe("div");
    expect(runButton(frames)).toBeDefined();
  });

  it("opened tables are listed above the Stars block", () => {
    const activate = vi.spyOn(lightCurveTableLoader, "activate").mockResolvedValue();
    lightCurveStore.put(TABLE_GROUP, [tableDoc()]);
    const tree = show();
    const all = elements(tree);
    const line = all.findIndex((p) => p["data-open-tables"]);
    expect(line).toBeGreaterThanOrEqual(0);
    expect(line).toBeLessThan(all.findIndex((p) => p.children === "Stars"));
    const saved = all[line].children as ReactNode;
    expect(textOf(saved)).toContain("Saved light curves:");
    (buttonNamed(saved, "a.csv")!.onClick as () => void)();
    expect(activate).toHaveBeenCalledWith(TABLE_GROUP);
    lightCurveStore.remove(TABLE_GROUP);
    expect(elements(show()).some((p) => p["data-open-tables"])).toBe(false);
  });

  it("a light-curve route left by Files is opened once and cleared", () => {
    const open = vi.spyOn(lightCurveTableLoader, "open").mockResolvedValue();
    const whtlt = { path: "C:/d/jw02358_whtlt.ecsv", name: "jw02358_whtlt.ecsv", probe: probeOf("light_curve") };
    tableRouteStore.request("light_curve", whtlt);
    show();
    expect(open).toHaveBeenCalledTimes(1);
    expect(open).toHaveBeenCalledWith(whtlt.path);
    expect(tableRouteStore.get("light_curve")).toBeNull();
    show();
    expect(open).toHaveBeenCalledTimes(1);
    tableRouteStore.request("light_curve", whtlt);
    show();
    expect(open).toHaveBeenCalledTimes(2);
    expect(open).toHaveBeenLastCalledWith(whtlt.path);
    const catalogue = tableRouteStore.request("catalogue", { path: "C:/d/cat.csv", name: "cat.csv", probe: probeOf("catalogue") });
    show();
    expect(tableRouteStore.get("catalogue")).toBe(catalogue);
    expect(open).toHaveBeenCalledTimes(2);
  });

  it("the saved CSV carries the units and marks a frame left out in the view", async () => {
    await measured();
    lightCurveStore.patchView(frameGroupKey(REF), { excluded: [1] });
    (buttonNamed(show(), "Save CSV")!.onClick as () => void)();
    await vi.waitFor(() => expect(backend.written).toHaveLength(1));
    const lines = backend.written[0].trimEnd().split("\n");
    expect(lines).toContain("# unit bg_mean: ADU");
    expect(lines).toContain("# unit target_flux: ADU");
    const header = lines.findIndex((l) => l.startsWith("index,"));
    const columns = lines[header].split(",");
    const cell = (row: number, name: string) => lines[header + row].split(",")[columns.indexOf(name)];
    expect(cell(1, "left_out")).toBe("");
    expect(cell(2, "left_out")).toBe("by hand");
    expect(cell(1, "rel_flux")).toBe("0.5");
  });

  it("shows the photometry status of the reference frame above the Stars block", async () => {
    const statusLines = (tree: ReactNode) => elementsOf(tree).filter((e) => e.type === PhotometryStatusLine).map((e) => e.props);
    const order = elementsOf(show());
    const line = order.findIndex((e) => e.type === PhotometryStatusLine);
    const nameField = order.findIndex((e) => e.props["aria-label"] === "Name of P1");
    expect(statusLines(show())).toEqual([{ filePath: REF, measureKey: null, excludeDq: false }]);
    expect(nameField, "the name field of P1").toBeGreaterThanOrEqual(0);
    expect(line).toBeLessThan(nameField);
    await measured();
    expect(statusLines(show(SECOND)).map((p) => p.filePath)).toEqual([REF]);
  });
});

describe("TimeSeriesPanel frame set (flows-1)", () => {
  const FLAT = doneFrame("c", "C:/data/flat_001.fits", { IMAGETYP: "'Flat Field'", FILTER: "'V'" });
  const STACK = doneFrame("d", "C:/data/stacked.fits", { ...LIGHT_HEADER, ABPROC: "'stacked'", ABCOMB: "'mean'" });
  const LEFT_OUT = "1 frame left out by IMAGETYP: 1 flat; 1 stacked result left out of the analysis";

  beforeEach(() => {
    vi.stubGlobal("window", { localStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {} } });
    hooks.mount();
    backend.regions = new Map([
      [REF, [point("r1", 10, 10), point("r2", 40, 40)]],
      [STACK.path, [point("s1", 10, 10), point("s2", 40, 40)]],
    ]);
    backend.series.mockReset().mockResolvedValue(RESULT);
    backend.stored = [...lights(), FLAT, STACK];
    backend.visible = backend.stored;
    lightCurveStore.remove(frameGroupKey(REF));
    lightCurveStore.remove(frameGroupKey(SECOND));
    lightCurveStore.setActiveTable(null);
    lightCurveStore.remove(TABLE_GROUP);
    tableRouteStore.reset();
    backend.selected.mockReset();
    backend.selectedRegion.clear();
    backend.progress = { percent: 0, active: false };
  });

  afterEach(() => {
    hooks.mount();
    vi.unstubAllGlobals();
  });

  it("measures only the lights shown in Files, leaving calibration frames and stacked results out", async () => {
    const tree = show();
    expect(runButton(tree)?.label).toBe("Measure 2 frames");
    expect(textOf(tree)).toContain(LEFT_OUT);
    (runButton(tree)!.onClick as () => void)();
    await flush();
    expect(backend.series).toHaveBeenCalledTimes(1);
    expect(backend.series.mock.calls[0][0]).toEqual([REF, SECOND]);
  });

  it("counts only the frames the Files filter shows", () => {
    backend.visible = [lights()[0], FLAT, STACK];
    const tree = show();
    expect(runButton(tree)?.label).toBe("Measure 1 frame");
    expect(textOf(tree)).toContain(LEFT_OUT);
  });

  it("refuses a stacked result or a calibration frame as the reference and says why", () => {
    const tree = show(STACK.path);
    expect(runButton(tree)?.label).toBe("Measure 2 frames");
    expect(runButton(tree)?.disabled).toBe(true);
    expect(textOf(tree)).toContain("The current file is a calibration frame or a stacked result, so it cannot be the reference.");
    expect(textOf(tree)).not.toContain("is not a done frame");
  });
});

describe("TimeSeriesPanel shared buttons (visual-14, flows-14)", () => {
  beforeEach(() => {
    vi.stubGlobal("window", { localStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {} } });
    hooks.mount();
    backend.regions = new Map([[REF, [point("r1", 10, 10), point("r2", 40, 40)]]]);
    backend.series.mockReset().mockResolvedValue(RESULT);
    backend.saved = [];
    backend.written = [];
    backend.stored = lights();
    backend.visible = backend.stored;
    lightCurveStore.remove(frameGroupKey(REF));
    lightCurveStore.remove(frameGroupKey(SECOND));
    lightCurveStore.setActiveTable(null);
    tableRouteStore.reset();
    backend.selectedRegion.clear();
    backend.progress = { percent: 0, active: false };
  });

  afterEach(() => {
    hooks.mount();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  const copyButton = (tree: ReactNode) => elementsOf<Props>(tree).filter((el) => el.type === CopyCsvButton).map((el) => el.props);
  const withoutStamp = (csv: string) => csv.split("\n").filter((line) => !line.startsWith("# exported_utc: ")).join("\n");

  it("keeps no private copy of the small button class", () => {
    expect(localSmallButtonLines(timeSeriesPanelSource)).toEqual([]);
    expect(timeSeriesPanelSource).toMatch(/import \{[^}]*\bSMALL_BUTTON_CLASS\b[^}]*\} from "\.\.\/ui";/);
  });

  it("hands the light curve to the shared copy button", async () => {
    const tree = await measured();
    const copies = copyButton(tree);
    expect(copies).toHaveLength(1);
    const text = (copies[0].text as () => string | null)();
    expect(typeof text).toBe("string");
    expect(text).not.toBe("");
    (buttonNamed(tree, "Save CSV")!.onClick as () => void)();
    await vi.waitFor(() => expect(backend.written).toHaveLength(1));
    expect(withoutStamp(backend.written[0])).toBe(withoutStamp(text!));
  });

  it("has nothing to copy without a target", async () => {
    backend.series.mockReset().mockResolvedValue({ ...RESULT, targets: RESULT.targets.map((t) => ({ ...t, role: "comp" as const })) });
    const tree = await measured();
    const copies = copyButton(tree);
    expect(copies).toHaveLength(1);
    expect(copies[0].disabled).toBe(true);
    expect((copies[0].text as () => string | null)()).toBeNull();
  });

  it("shows a refused clipboard write in the panel's alert", async () => {
    const copies = copyButton(await measured());
    expect(copies).toHaveLength(1);
    (copies[0].onError as (m: string) => void)("Clipboard copy failed: Document is not focused.");
    const messages = elementsOf<Props>(show()).filter((el) => el.type === ErrorAlert).map((el) => el.props.message);
    expect(messages).toContain("Clipboard copy failed: Document is not focused.");
  });
});
