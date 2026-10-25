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
}));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../context/PreviewContext", () => ({
  useDqContext: () => ({ excludeDq: false }),
  useFileContext: () => ({ file: { path: REF, result: { dimensions: [1600, 1600] } } }),
  useRenderContext: () => ({ processed: null }),
}));
vi.mock("../../../hooks/useRegionStore", () => ({ useRegionDoc: (key: string | null) => ({ regions: backend.regions.get(key ?? "") ?? [] }) }));
vi.mock("../../../hooks/useProgress", () => ({ useProgress: () => ({ percent: 0, active: false, reset: () => {} }) }));
vi.mock("../../../hooks/useAnalysisTarget", () => ({
  useAnalysisTarget: () => ({ composite: false, displayed: { isProcessed: false, label: "502nmos.fits" } }),
}));
vi.mock("../../../hooks/useFrameSet", () => {
  const frames = [
    { id: "a", path: "C:/data/502nmos.fits" },
    { id: "b", path: "C:/data/502nmos_2.fits" },
  ];
  return {
    useMatchingFrames: () => frames,
    orderWithReferenceFirst: (all: { path: string }[], reference: string | null) => [
      ...all.filter((f) => f.path === reference),
      ...all.filter((f) => f.path !== reference),
    ],
    pathOf: (f: { path: string }) => f.path,
  };
});
vi.mock("../../../hooks/useFileStore", () => ({ fileStore: { selectFile: () => {} } }));
vi.mock("../../../services/analysis", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/analysis")>()),
  photometryGainModel: () => new Promise(() => {}),
  timeSeriesPhotometry: backend.series,
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: async (dialog: { defaultPath: string }) => `C:/out/${dialog.defaultPath}` }));
vi.mock("@tauri-apps/plugin-fs", () => ({ writeTextFile: async (path: string) => void backend.saved.push(path) }));

import { hooks } from "../../../test/hookRuntime";
import { fluxCellText } from "../../../utils/photometryTable";

const { default: TimeSeriesPanel } = await import("../TimeSeriesPanel");

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

describe("TimeSeriesPanel after a run", () => {
  beforeEach(() => {
    vi.stubGlobal("window", { localStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {} } });
    hooks.mount();
    backend.regions = new Map([[REF, [point("r1", 10, 10), point("r2", 40, 40)]]]);
    backend.series.mockReset().mockResolvedValue(RESULT);
    backend.saved = [];
  });

  afterEach(() => {
    hooks.mount();
    vi.useRealTimers();
    vi.unstubAllGlobals();
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
    const blockers = (tree: ReactNode) => elements(tree).filter((p) => p["data-export-blocker"]).map((p) => textOf(p.children as ReactNode));
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
