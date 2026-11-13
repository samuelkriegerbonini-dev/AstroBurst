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
}));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../context/PreviewContext", () => ({
  useDqContext: () => ({ excludeDq: false }),
  useFileContext: () => ({ file: { path: REF, result: { dimensions: [1600, 1600] } } }),
  useRenderContext: () => ({ processed: null }),
  useDoneFilesContext: () => ({ doneFiles: backend.visible }),
}));
vi.mock("../../../hooks/useRegionStore", () => ({ useRegionDoc: (key: string | null) => ({ regions: backend.regions.get(key ?? "") ?? [] }) }));
vi.mock("../../../hooks/useProgress", () => ({ useProgress: () => ({ percent: 0, active: false, reset: () => {} }) }));
vi.mock("../../../hooks/useAnalysisTarget", () => ({
  useAnalysisTarget: () => ({ composite: false, displayed: { isProcessed: false, label: "502nmos.fits" } }),
}));
vi.mock("../../../hooks/useFileStore", () => ({ useDoneFiles: () => backend.stored, fileStore: { selectFile: () => {} } }));
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

import { hooks } from "../../../test/hookRuntime";
import { fluxCellText } from "../../../utils/photometryTable";
import { regionStore } from "../../../utils/regionStore";

const { default: TimeSeriesPanel } = await import("../TimeSeriesPanel");

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
    expect(markup.match(/inputMode="decimal"/g)).toHaveLength(4);
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

function frame(index: number, path: string, targetFlux: number): TimeSeriesFrame {
  return {
    index, path, file_name: path.split("/").pop()!, jd_mid: 2460310.6 + index * 0.001,
    time_source: "DATE-OBS + EXPTIME/2 (60 s); UTC assumed (no TIMESYS)", exptime: 60, filter: "V", airmass: 1.2,
    geometry: geometry(2460310.6 + index * 0.001), offset: null, photcal_label: null,
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
const exportButton = (tree: ReactNode, format: string) => elements(tree).find((p) => p["data-export"] === format);
const buttonNamed = (tree: ReactNode, name: string) =>
  elements(tree).find((p) => typeof p.onClick === "function" && p.children !== undefined && textOf(p.children as ReactNode).trim() === name);
const runButton = (tree: ReactNode) => elements(tree).find((p) => typeof p.label === "string" && p.label.startsWith("Measure "));

function withGeometry(result: TimeSeriesResult, patch: Partial<FrameGeometry>): TimeSeriesResult {
  return { ...result, frames: result.frames.map((f) => ({ ...f, geometry: { ...f.geometry!, ...patch } })) };
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

  it("flags a saturated comparison star and leaves it out of the ensemble for the whole series on one click (front-photometry/B1)", async () => {
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
    expect(blockers(tree)).toEqual(["ETD and ExoClock: comparison star P2 saturated on 502nmos_2.fits (leave it out of the ensemble)"]);
    expect(exportButton(tree, "etd")?.disabled).toBe(true);
    expect(textOf(tree)).toContain("comparison star P2 saturated on 1 frame");
    expect(textOf(tree)).toContain(`${(-2.5 * Math.log10(480 / 1900)).toFixed(4)} ±`);
    (buttonNamed(tree, "Leave P2 out")!.onClick as () => void)();
    const leftOut = show();
    expect(chips(leftOut)).toHaveLength(0);
    expect(blockers(leftOut)).toEqual([]);
    expect(exportButton(leftOut, "etd")?.disabled).toBe(false);
    expect(textOf(leftOut)).toContain("comparison star P2 saturated on 1 frame, left out of the ensemble");
    expect(textOf(leftOut)).toContain(`${(-2.5 * Math.log10(0.48)).toFixed(4)} ±`);
    (buttonNamed(leftOut, "Save CSV")!.onClick as () => void)();
    await vi.waitFor(() => expect(backend.written).toHaveLength(1));
    expect(backend.written[0].split("\n").slice(1, 3)).toEqual(["# comparison stars: P3", "# comparison stars left out: P2"]);
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
    expect(elements(show()).find((p) => typeof p.xLabel === "string")?.xLabel).toBe("JD (header time, barycentric) - 2460310");
  });

  it("saves the light-curve CSV with its provenance, the reference frame's BUNIT and the sky at full precision (trust-2)", async () => {
    await measured();
    (buttonNamed(show(), "Save CSV")!.onClick as () => void)();
    await vi.waitFor(() => expect(backend.written).toHaveLength(1));
    const lines = backend.written[0].split("\n");
    expect(lines.slice(0, 3)).toEqual(["# target: P1", "# comparison stars: P2", "# reference frame: 502nmos.fits"]);
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
