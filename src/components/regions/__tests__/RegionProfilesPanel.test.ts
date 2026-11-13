import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LineCut, RadialBin, RadialProfile, Region, RegionShape, SbBin, SbProfile } from "../../../shared/types/regions";
import type { ImagePsfFwhm, ResolutionSettings } from "../../../shared/types/resolution";
import type { MeasurementSource } from "../../../utils/analysisTarget";
import type { PlotReferenceLine, ProfileSeries } from "../ProfilePlot";

const live = vi.hoisted(() => ({
  regions: [] as Region[],
  selectedId: null as string | null,
  fetch: { result: null as unknown, error: null as string | null },
  source: null as MeasurementSource | null,
  unit: null as string | null,
  unitCalls: [] as [string | null, boolean][],
  settings: null as unknown,
  imagePsf: null as unknown,
  plotProps: [] as Record<string, unknown>[],
  cardProps: [] as Record<string, unknown>[],
}));
const append = vi.hoisted(() => vi.fn());

vi.mock("react", async (importOriginal) => ({
  ...(await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()),
  useReducer: () => [live.fetch, () => {}],
}));
vi.mock("../../../hooks/useRegionStore", () => ({
  useRegionDoc: () => ({ regions: live.regions, selectedId: live.selectedId, version: 1 }),
}));
vi.mock("../../../context/PreviewContext", () => ({ useDqContext: () => ({ excludeDq: false }) }));
vi.mock("../../../hooks/useMeasurementLog", () => ({ useMeasurementProvenance: () => ({ file: "sky.fits", image: "original" }) }));
vi.mock("../../../hooks/useAnalysisTarget", () => ({ useMeasurementSource: () => live.source }));
vi.mock("../../../hooks/useImageUnit", () => ({
  useImageUnit: (path: string | null, enabled: boolean) => {
    live.unitCalls.push([path, enabled]);
    return enabled ? live.unit : null;
  },
}));
vi.mock("../../../hooks/useResolutionSettings", () => ({
  useResolutionSettings: () => live.settings,
  useImagePsf: () => ({ psf: live.imagePsf, loading: false, error: null }),
  useHeaderSeeing: () => [],
}));
vi.mock("../../../utils/measurementLog", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../utils/measurementLog")>()),
  measurementLog: { append },
}));
vi.mock("../ProfilePlot", () => ({
  default: (props: Record<string, unknown>) => {
    live.plotProps.push(props);
    return null;
  },
}));
vi.mock("../LineSkyReadout", () => ({ default: () => null }));
vi.mock("../../analysis/ResolutionCard", () => ({
  default: (props: Record<string, unknown>) => {
    live.cardProps.push(props);
    return null;
  },
}));
vi.mock("../../analysis/MeasurementBadge", () => ({ default: () => null }));
vi.mock("../../../services/regions", () => ({
  radialProfile: () => new Promise(() => {}),
  lineCut: () => new Promise(() => {}),
  sbProfile: () => new Promise(() => {}),
}));

import RegionProfilesPanel, { RegionProfilesPanelInner } from "../RegionProfilesPanel";
import { hooks } from "../../../test/hookRuntime";
import { elementsOf, findElement, textOf } from "../../../test/elementTree";
import { CopyCsvButton } from "../../ui";
import { sbProfileCsv } from "../../../utils/sbProfile";
import { shapeSummary } from "../../../utils/regionGeometry";
import { seriesToCsv } from "../../../utils/plotInteraction";
import { DEFAULT_RESOLUTION_SETTINGS } from "../../../utils/resolutionText";

const PROPS = { filePath: "sky.fits", measurePath: "sky.fits", measureKey: "k" };
const REGION_PROPS = { color: null, width: null, text: null, dash: null, include: true };
const ELLIPSE_SHAPE: RegionShape = { shape: "ellipse", x: 50, y: 50, rx: 12, ry: 7, angle: 135.2 };
const CIRCLE_SHAPE: RegionShape = { shape: "circle", x: 10.5, y: 20.5, r: 30 };
const LINE_SHAPE: RegionShape = { shape: "line", x1: 1, y1: 2, x2: 4, y2: 6 };
const ELLIPSE: Region = { id: "e1", shape: ELLIPSE_SHAPE, props: REGION_PROPS, backgroundId: null };
const CIRCLE: Region = { id: "c1", shape: CIRCLE_SHAPE, props: REGION_PROPS, backgroundId: null };
const LINE: Region = { id: "l1", shape: LINE_SHAPE, props: REGION_PROPS, backgroundId: null };

const KEY = JSON.stringify({ kind: "sb", shape: ELLIPSE_SHAPE, background: null, binWidth: 1, exclude: [] });
const CIRCLE_KEY = JSON.stringify({ kind: "radial", x: 10.5, y: 20.5, maxRadius: 30, background: null, exclude: [] });
const LINE_KEY = JSON.stringify({ kind: "cut", x1: 1, y1: 2, x2: 4, y2: 6, exclude: [] });

const PENDING_TITLE = "Waiting for the profile that matches the current region and DQ setting";
const COPY_TITLE = "Copy every bin as CSV, with how the profile was measured";
const LOG_TITLE = "Log the profile summary on screen";
const DQ_NOTE = "DQ exclusion requested but not applied: this image has no DQ plane";
const PA_TEXT = "PA 135.2 deg image, 213.4 deg E of N";

function bin(overrides: Partial<SbBin> & { sma: number }): SbBin {
  const { sma } = overrides;
  return {
    sma_inner: sma - 0.5,
    sma_outer: sma + 0.5,
    count: 10,
    cumulative_count: 10,
    mean: 100 / sma,
    median: 90 / sma,
    std: 4,
    cumulative_sum: 100,
    sma_arcsec: sma / 20,
    mu_ab: 20 + sma / 10,
    mu_err: 0.01,
    mag_ab_cumulative: 15,
    ...overrides,
  };
}

function profile(overrides: Partial<SbProfile> = {}): SbProfile {
  return {
    x: 50,
    y: 50,
    sma_max: 3,
    ellipticity: 0.4,
    angle_deg: 30,
    bin_width: 1,
    background: null,
    bins: [
      bin({ sma: 0.5, cumulative_sum: 50, cumulative_count: 4 }),
      bin({ sma: 1.5, cumulative_sum: 80, cumulative_count: 14 }),
      bin({ sma: 2.5, cumulative_sum: 100, cumulative_count: 30 }),
    ],
    r50_px: 0.5,
    r80_px: 1.5,
    r90_px: 2.0,
    petrosian_radius_px: 2.4,
    pixel_scale_arcsec: 0.05,
    pixel_area_arcsec2: 0.0025,
    sky_pa_deg: 120,
    photcal: null,
    calibration_warnings: [],
    total_mag_ab: 14.5,
    notes: [],
    masked: false,
    dq_excluded: null,
    region_excluded: 0,
    elapsed_ms: 1,
    ...overrides,
  };
}

const SB = profile({ r50_px: 4.2, pixel_scale_arcsec: 0.262, ellipticity: 0.42, angle_deg: 135.2, sky_pa_deg: 213.4 });

const RADIAL_BINS: RadialBin[] = [
  { r: 0, count: 4, mean: 10, median: 9, std: 2, cumulative_sum: 40 },
  { r: 1, count: 1, mean: 3, median: 3, std: null, cumulative_sum: 43 },
];
const RADIAL: RadialProfile = {
  x: 10.5,
  y: 20.5,
  max_radius: 30,
  background: null,
  bins: RADIAL_BINS,
  masked: false,
  dq_excluded: null,
  region_excluded: 0,
  elapsed_ms: 2,
};
const CUT: LineCut = {
  x1: 1,
  y1: 2,
  x2: 4,
  y2: 6,
  length: 5,
  n_samples: 2,
  distance: [0, 5],
  xs: [],
  ys: [],
  values: [2, 9],
  masked: false,
  dq_excluded: null,
  region_excluded: 0,
  elapsed_ms: 1,
};

function imagePsf(halfLightRadiusArcsec: number): ImagePsfFwhm {
  return {
    fwhm_px: 4.2,
    fwhm_px_scatter: 0.1,
    fwhm_arcsec: 1.1,
    fwhm_arcsec_scatter: 0.03,
    half_light_radius_px: halfLightRadiusArcsec / 0.262,
    half_light_radius_arcsec: halfLightRadiusArcsec,
    n_stars: 102,
    n_measured: 194,
    n_candidates: 200,
    n_detected: 8092,
    max_stars: 200,
    model: "Moffat fit",
    pixel_scale_arcsec: 0.262,
    pixels_per_fwhm: 4.2,
    undersampled: false,
    header_values: [],
    stars: [],
    notes: [],
    warnings: [],
    elapsed_ms: 10,
  };
}

function select(region: Region, result: unknown): void {
  live.regions = [region];
  live.selectedId = region.id;
  live.fetch = { result, error: null };
}

function markup(): string {
  return renderToStaticMarkup(createElement(RegionProfilesPanel, PROPS));
}

function button(html: string, text: string): string {
  const chunk = html.split("<button").find((part) => part.includes(`${text}</button>`));
  expect(chunk, `${text} button`).toBeDefined();
  return chunk!.slice(0, chunk!.indexOf("</button>"));
}

function lastPlot(): { yLabel: string; series: ProfileSeries[]; referenceLines: PlotReferenceLine[] } {
  return live.plotProps.at(-1) as unknown as { yLabel: string; series: ProfileSeries[]; referenceLines: PlotReferenceLine[] };
}

function clickButton(predicate: (props: Record<string, unknown>, text: string) => boolean): void {
  const tree = hooks.render(RegionProfilesPanelInner, PROPS);
  const found = findElement<Record<string, unknown>>(tree, (e) => e.type === "button" && predicate(e.props, textOf(e).trim()));
  expect(found, "button").toBeDefined();
  expect(found!.props.disabled, "button is enabled").not.toBe(true);
  (found!.props.onClick as () => void)();
}

beforeEach(() => {
  hooks.mount();
  live.source = null;
  live.unit = null;
  live.unitCalls = [];
  live.settings = { ...DEFAULT_RESOLUTION_SETTINGS } satisfies ResolutionSettings;
  live.imagePsf = null;
  live.plotProps = [];
  live.cardProps = [];
  append.mockClear();
  select(ELLIPSE, { kind: "sb", requestKey: KEY, excludeDq: false, data: SB });
});

afterEach(() => vi.unstubAllGlobals());

describe("RegionProfilesPanel Copy CSV (trust-10, trust-12)", () => {
  it("Copy CSV waits for the profile of the current region like Log does", () => {
    select(ELLIPSE, { kind: "sb", requestKey: "stale", excludeDq: false, data: SB });
    const copy = button(markup(), "Copy CSV");
    expect(copy).toContain('disabled=""');
    expect(copy).toContain(`title="${PENDING_TITLE}"`);
  });

  it("Copy CSV is enabled for the profile on screen and copies the provenance", () => {
    const tree = hooks.render(RegionProfilesPanelInner, PROPS);
    const copy = elementsOf<Record<string, unknown>>(tree).find((e) => e.type === CopyCsvButton);
    expect(copy, "CopyCsvButton").toBeDefined();
    expect(copy!.props.disabled).toBe(false);
    expect(copy!.props.title).toBe(COPY_TITLE);
    const copied = (copy!.props.text as () => string | null)();
    expect(typeof copied).toBe("string");
    expect(copied!.startsWith("# software: AstroBurst ")).toBe(true);
    expect(copied!.endsWith(sbProfileCsv(SB))).toBe(true);
  });
});

describe("RegionProfilesPanel units on the y axis (viewer-12)", () => {
  function labels(): string[] {
    const out: string[] = [];
    select(ELLIPSE, { kind: "sb", requestKey: KEY, excludeDq: false, data: SB });
    markup();
    out.push(lastPlot().yLabel);
    select(CIRCLE, { kind: "radial", requestKey: CIRCLE_KEY, excludeDq: false, data: { ...RADIAL, background: { median: 1, sigma: 0.1, count: 50 } } });
    markup();
    out.push(lastPlot().yLabel);
    select(LINE, { kind: "cut", requestKey: LINE_KEY, excludeDq: false, data: CUT });
    markup();
    out.push(lastPlot().yLabel);
    return out;
  }

  it("the y title of the three profile kinds carries the unit of an unprocessed image", () => {
    live.unit = "MJy/sr";
    expect(labels()).toEqual(["mean (MJy/sr)", "value - bg (MJy/sr)", "value (MJy/sr)"]);
    expect(live.unitCalls.at(-1)).toEqual(["sky.fits", true]);
    live.unit = null;
    expect(labels()).toEqual(["mean", "value - bg", "value"]);
    live.unit = "MJy/sr";
    live.source = { tone: "processed", text: "p", title: "t" };
    live.unitCalls = [];
    expect(labels()).toEqual(["mean", "value - bg", "value"]);
    expect(live.unitCalls.length).toBeGreaterThan(0);
    expect(live.unitCalls.every(([path, enabled]) => path === "sky.fits" && enabled === false)).toBe(true);
  });
});

describe("RegionProfilesPanel cards (visual-4)", () => {
  it("cards show their whole value", () => {
    const html = markup();
    expect(html).toContain(PA_TEXT);
    const start = html.indexOf('<div class="grid grid-cols-2 gap-1">');
    expect(start).toBeGreaterThan(-1);
    const grid = html.slice(start, html.indexOf('<div class="flex flex-wrap items-center justify-between gap-1">', start));
    expect(grid).not.toContain("truncate");
    expect(grid).toContain(`title="${PA_TEXT}"`);
    expect(grid).toContain('title="First semi-major axis where I(r) / &lt;I(&lt;r)&gt; falls below 0.2"');
  });
});

describe("RegionProfilesPanel size against the PSF (resolution-psf/U8, G5)", () => {
  it("R50 is handed to the rule as a half-light radius", () => {
    markup();
    expect(live.cardProps).toHaveLength(1);
    const card = live.cardProps[0];
    expect(Object.keys(card).sort()).toEqual(["context", "filePath", "halfLightRadiusArcsec", "sourceLabel"]);
    expect(card.filePath).toBe("sky.fits");
    expect(card.context).toBe("profile");
    expect(card.sourceLabel).toBe(shapeSummary(ELLIPSE_SHAPE));
    expect(Math.abs((card.halfLightRadiusArcsec as number) - 4.2 * 0.262)).toBeLessThan(1e-12);

    live.cardProps = [];
    select(ELLIPSE, { kind: "sb", requestKey: KEY, excludeDq: false, data: { ...SB, r50_px: null } });
    markup();
    select(ELLIPSE, { kind: "sb", requestKey: KEY, excludeDq: false, data: { ...SB, pixel_scale_arcsec: null } });
    markup();
    expect(live.cardProps).toHaveLength(0);
  });

  it("the plot marks the PSF", () => {
    live.settings = { ...DEFAULT_RESOLUTION_SETTINGS, psfSource: "typed", psfFwhmArcsec: 1.0 };
    markup();
    let lines = lastPlot().referenceLines;
    expect(lines.map((l) => l.label)).toEqual(["R50", "R80", "R_P", "PSF FWHM/2"]);
    expect(lines.at(-1)).toEqual({ axis: "x", value: 0.5 / 0.262, label: "PSF FWHM/2", color: "#c4b5fd", dashed: true });

    live.settings = { ...DEFAULT_RESOLUTION_SETTINGS, psfSource: "image" };
    live.imagePsf = imagePsf(0.52);
    markup();
    lines = lastPlot().referenceLines;
    expect(lines.at(-1)).toEqual({ axis: "x", value: 0.52 / 0.262, label: "PSF R50", color: "#c4b5fd", dashed: true });

    live.imagePsf = null;
    markup();
    expect(lastPlot().referenceLines.map((l) => l.label)).toEqual(["R50", "R80", "R_P"]);
  });
});

describe("RegionProfilesPanel notes (PHOT-PANELS request)", () => {
  it("notes of a radial profile and of a line cut are shown", () => {
    select(CIRCLE, { kind: "radial", requestKey: CIRCLE_KEY, excludeDq: false, data: { ...RADIAL, notes: [DQ_NOTE] } });
    let html = markup();
    expect(html).toContain("notes (1)");
    expect(html).toContain(DQ_NOTE);
    select(LINE, { kind: "cut", requestKey: LINE_KEY, excludeDq: false, data: { ...CUT, notes: [DQ_NOTE] } });
    html = markup();
    expect(html).toContain("notes (1)");
    expect(html).toContain(DQ_NOTE);
    select(CIRCLE, { kind: "radial", requestKey: CIRCLE_KEY, excludeDq: false, data: RADIAL });
    expect(markup()).not.toContain("<details");
    select(LINE, { kind: "cut", requestKey: LINE_KEY, excludeDq: false, data: CUT });
    expect(markup()).not.toContain("<details");
  });
});

describe("RegionProfilesPanel radial error bars (trust-9)", () => {
  it("the radial mean carries the error of the mean", () => {
    select(CIRCLE, { kind: "radial", requestKey: CIRCLE_KEY, excludeDq: false, data: RADIAL });
    markup();
    const series = lastPlot().series;
    expect(series[0].label).toBe("mean");
    expect(series[0].yErr).toEqual([1, null]);
    expect(series[1].label).toBe("median");
    expect(series[1].yErr).toBeUndefined();
    expect(seriesToCsv(series, "radius (px)").split("\r\n")[0]).toBe("radius (px),mean,median,mean_err");

    select(LINE, { kind: "cut", requestKey: LINE_KEY, excludeDq: false, data: CUT });
    markup();
    const cut = lastPlot().series;
    expect(cut).toHaveLength(1);
    expect(cut[0].yErr).toBeUndefined();
  });
});

describe("RegionProfilesPanel log rows (W0-F6 request, trust-9 log part)", () => {
  function logged(region: Region, result: unknown): Record<string, unknown> {
    append.mockClear();
    hooks.mount();
    select(region, result);
    clickButton((props) => props.title === LOG_TITLE);
    expect(append).toHaveBeenCalledTimes(1);
    return append.mock.calls[0][0] as Record<string, unknown>;
  }

  it("the log rows of a radial profile and of a line cut carry the unit of the image", () => {
    live.unit = "MJy/sr";
    let radial = logged(CIRCLE, { kind: "radial", requestKey: CIRCLE_KEY, excludeDq: false, data: RADIAL });
    expect(radial.kind).toBe("radial_profile");
    expect(radial.unit).toBe("MJy/sr");
    expect(radial.notes).toEqual(["cumulative_sum_last in MJy/sr x pix"]);
    let cut = logged(LINE, { kind: "cut", requestKey: LINE_KEY, excludeDq: false, data: CUT });
    expect(cut.kind).toBe("line_cut");
    expect(cut.unit).toBe("MJy/sr");

    live.unit = null;
    radial = logged(CIRCLE, { kind: "radial", requestKey: CIRCLE_KEY, excludeDq: false, data: RADIAL });
    expect(radial.unit).toBeNull();
    expect(radial.notes).toEqual([]);
    cut = logged(LINE, { kind: "cut", requestKey: LINE_KEY, excludeDq: false, data: CUT });
    expect(cut.unit).toBeNull();
  });
});
