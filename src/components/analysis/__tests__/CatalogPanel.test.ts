import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";
import type { ConeSearchResult, CrossMatchEntry, CrossMatchResult, ZeroPointFit, ZeroPointStatus } from "../../../shared/types/catalog";
import type { PlotReferenceLine, ProfileSeries } from "../../regions/ProfilePlot";
import { APERTURE_RANGE_HINT } from "../../../utils/photometryTable";
import { APP_VERSION } from "../../../utils/constants";
import { catalogueCsvText, coneCsvHeaderLines, crossMatchCsvHeaderLines } from "../../../utils/catalogProvenance";
import plotsSource from "../CrossMatchPlots.tsx?raw";
import panelSource from "../CatalogPanel.tsx?raw";

const WCS_PATH = "C:/data/m16.fits";

const backend = vi.hoisted(() => ({
  cone: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  crossMatch: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  exported: [] as string[],
  headers: [] as (string[] | null)[],
}));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../services/catalog", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/catalog")>()),
  catalogConeSearch: backend.cone,
  catalogCrossMatch: backend.crossMatch,
  catalogExportCsv: async (_path: string, outputPath: string, _items: unknown, headerLines?: readonly string[]) => {
    backend.exported.push(outputPath);
    backend.headers.push(headerLines ? [...headerLines] : null);
    return { output_path: outputPath, n: 1 };
  },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: async (dialog: { defaultPath: string }) => `C:/out/${dialog.defaultPath}` }));

import { hooks } from "../../../test/hookRuntime";

const { default: CatalogPanel } = await import("../CatalogPanel");
const { default: TargetsPanel } = await import("../TargetsPanel");
const { default: CrossMatchPlots } = await import("../CrossMatchPlots");

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

const CONE: ConeSearchResult = {
  rows: [
    {
      id: "Gaia DR3 4146599224155426304", ra: 274.7, dec: -13.8, ra_epoch: 274.7, dec_epoch: -13.8, pm_ra_masyr: null,
      pm_dec_masyr: null, g: 12.1, bp: 12.5, rp: 11.6, bp_rp: 0.9, parallax_mas: null, x: 100, y: 120, on_image: true,
    },
  ],
  epoch_year: 2024.5,
  n_total: 1,
  n_on_image: 1,
  radius_arcmin: 12,
  center_ra: 274.7,
  center_dec: -13.8,
  elapsed_ms: 4,
  warnings: [],
};

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

type Panel = (props: { filePath: string | null }) => unknown;
const panel = (CatalogPanel as unknown as { type: Panel }).type;
const show = () => hooks.render(panel, { filePath: WCS_PATH }) as ReactNode;
const notice = () => elements(show()).find((p) => typeof p.className === "string" && p.className.includes("text-emerald-400/90"));

function exportCatalogCsv(): void {
  const button = elements(show()).find((p) => p.children !== undefined && textOf(p.children as ReactNode).trim() === "Catalog CSV");
  expect(button?.disabled, "Catalog CSV is enabled").toBe(false);
  (button!.onClick as () => void)();
}

async function searched(): Promise<void> {
  const search = elements(show()).find((p) => p.label === "Search Gaia DR3");
  (search!.onClick as () => void)();
  await flush();
}

describe("CatalogPanel Saved notice", () => {
  beforeEach(() => {
    hooks.mount();
    backend.cone.mockReset().mockResolvedValue(CONE);
    backend.exported = [];
    backend.headers = [];
  });

  afterEach(() => {
    hooks.mount();
    vi.useRealTimers();
  });

  it("cancels the notice timer when the panel closes", async () => {
    await searched();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"], shouldAdvanceTime: true });
    exportCatalogCsv();
    await vi.waitFor(() => expect(backend.exported).toHaveLength(1));
    expect(textOf(notice()?.children as ReactNode)).toBe(`Saved ${backend.exported[0]}`);
    expect(vi.getTimerCount(), "the notice timer runs while the panel is open").toBe(1);
    hooks.mount();
    expect(vi.getTimerCount(), "no timer outlives the panel").toBe(0);
  });

  it("shows each export in one timed notice that a later export restarts", async () => {
    await searched();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"], shouldAdvanceTime: true });
    exportCatalogCsv();
    await vi.waitFor(() => expect(backend.exported).toHaveLength(1));
    await vi.advanceTimersByTimeAsync(3_000);
    exportCatalogCsv();
    await vi.waitFor(() => expect(backend.exported).toHaveLength(2));
    await vi.advanceTimersByTimeAsync(5_000);
    expect(notice(), "the second export is still shown 5 s after it").toBeDefined();
    await vi.advanceTimersByTimeAsync(1_500);
    expect(notice()).toBeUndefined();
  });
});

describe("Sky panels without an image", () => {
  beforeEach(() => hooks.mount());
  afterEach(() => hooks.mount());

  const runReason = (nodes: Props[], label: string) => {
    const run = nodes.find((p) => p.label === label);
    const reason = nodes.find((p) => typeof p.id === "string" && p.id === run?.describedBy);
    return { run, reason: reason ? textOf(reason.children as ReactNode) : null, wrapper: nodes.find((p) => p.title !== undefined && p.title === (reason ? textOf(reason.children as ReactNode) : null)) };
  };

  it("says why Search Gaia DR3 and the cross-match are disabled", () => {
    const nodes = elements(hooks.render(panel, { filePath: null }) as ReactNode);
    const search = runReason(nodes, "Search Gaia DR3");
    expect(search.run?.disabled).toBe(true);
    expect(search.reason).toMatch(/^Select an image first\..*WCS.*Stars/);
    expect(search.wrapper).toBeDefined();
    expect(nodes.find((p) => p.label === "Cross-match detected stars")?.describedBy).toBe(search.run?.describedBy);
  });

  it("drops the reason once an image is selected", () => {
    const nodes = elements(show());
    const search = runReason(nodes, "Search Gaia DR3");
    expect(search.run?.disabled).toBe(false);
    expect(search.run?.describedBy).toBeUndefined();
    expect(nodes.some((p) => typeof p.children === "string" && p.children.startsWith("Select an image first."))).toBe(false);
  });

  it("says why Place is disabled, for a missing image and for an empty target list", () => {
    const targets = (TargetsPanel as unknown as { type: Panel }).type;
    const noImage = runReason(elements(hooks.render(targets, { filePath: null }) as ReactNode), "Place");
    expect(noImage.run?.disabled).toBe(true);
    expect(noImage.reason).toMatch(/^Select an image first\..*WCS.*Stars/);
    expect(noImage.wrapper).toBeDefined();
    hooks.mount();
    const noTargets = runReason(elements(hooks.render(targets, { filePath: WCS_PATH }) as ReactNode), "Place");
    expect(noTargets.run?.disabled).toBe(true);
    expect(noTargets.reason).toBe("Enter at least one target to place.");
  });
});

const ZERO_POINT = 25;
const AUTO_APERTURE = { kind: "auto", fwhm_factor: 1.5, median_radius_px: 7.06, min_radius_px: 4.62, max_radius_px: 9.06 } as const;

function zeroPointMatch(index: number, residual: number, zpStatus: ZeroPointStatus | null, bpRp: number | null = null): CrossMatchEntry {
  const g = 12 + index / 8;
  return {
    star: {
      x: 10 + index, y: 20, ra: 274.7, dec: -13.8, flux: 1000, fwhm: 3.2, snr: 80, mag_inst: g - ZERO_POINT - residual, mag_ab: null,
      saturated: zpStatus === "saturated", aperture_radius: 4.8,
    },
    row: { id: `Gaia DR3 ${index}`, ra: 274.7, dec: -13.8, g, bp: null, rp: null, bp_rp: bpRp },
    sep_arcsec: 0.2,
    d_ra_arcsec: 0.01,
    d_dec_arcsec: 0.004,
    zp_status: zpStatus,
  };
}

function fitOf(matches: CrossMatchEntry[], rms: number, zpErr: number | null = 0.012): ZeroPointFit {
  const count = (status: ZeroPointStatus) => matches.filter((m) => m.zp_status === status).length;
  return {
    zp: ZERO_POINT, zp_err: zpErr, colour_coeff: null, rms, n_used: count("used"), n_rejected: count("clipped"), n_without_colour: count("no_colour"),
    band: "G", colour_term_used: false,
  };
}

function crossMatchResult(matches: CrossMatchEntry[], overrides: Partial<CrossMatchResult> = {}): CrossMatchResult {
  return {
    matches,
    sources: matches.map((m) => m.star),
    astrometry: { median_d_ra_arcsec: 0.01, median_d_dec_arcsec: 0.004, rms_arcsec: 0.052, n: matches.length },
    zero_point: fitOf(matches, 0.02),
    photcal_present: false,
    epoch_year: 2024.5,
    n_detected: matches.length,
    n_catalog: 120,
    band: "G",
    match_radius_arcsec: 2,
    elapsed_ms: 9,
    warnings: [],
    aperture: AUTO_APERTURE,
    ...overrides,
  };
}

const QUERY = { radius_arcmin: 3.017, field_radius_arcmin: 3.017, mag_limit: null, max_rows: 5000, row_cap_reached: false, cone_smaller_than_field: false };
const LISTED = { catalog_query: QUERY, catalog_table: "I/355/gaiadr3", mag_system: "vega" as const, detection_sigma: 5, max_stars: 500, catalog_source: "network" as const };

const withoutExportTime = (lines: readonly string[]) => lines.filter((l) => !l.startsWith("# exported_utc:"));
const CONTEXT = { appVersion: APP_VERSION, exportedUtc: "x", fileName: "x_calibrated.fits" };

const pairs = (value: number, count: number) => Array.from({ length: count }, () => [value, -value]).flat();
const catalogueG = (matches: CrossMatchEntry[]) => matches.map((m) => m.row.g);

interface PlotProps {
  series: ProfileSeries[];
  referenceLines?: PlotReferenceLine[];
  csvName?: string;
  xLabel?: string;
}

const plotsType = (CrossMatchPlots as unknown as { type: (props: { cross: CrossMatchResult }) => unknown }).type;

function plotsOf(cross: CrossMatchResult): { residuals: PlotProps; zeroPoint: PlotProps } {
  hooks.mount();
  const nodes = elements(hooks.render(plotsType, { cross }) as ReactNode);
  const plot = (csvName: string) => nodes.find((p) => p.csvName === csvName) as unknown as PlotProps;
  return { residuals: plot("residuals"), zeroPoint: plot("zero_point") };
}

const pointGroups = (plot: PlotProps) => Object.fromEntries(plot.series.filter((s) => s.mode === "points").map((s) => [s.label, s.x]));

describe("CrossMatchPlots zero point plot", () => {
  afterEach(() => hooks.mount());

  it("draws the saturated star the fit left out apart from the fitted stars, although it lies on the line", () => {
    const matches = [
      zeroPointMatch(0, 0, "saturated"),
      zeroPointMatch(1, 0, "used"),
      zeroPointMatch(2, 0.02, "used"),
      zeroPointMatch(3, -0.02, "used"),
      zeroPointMatch(4, 1.5, "clipped"),
    ];
    const { zeroPoint } = plotsOf(crossMatchResult(matches, { zero_point: fitOf(matches, 0.016329931618554522) }));
    expect(zeroPoint.series[0].x, "the first series draws only the stars of the fit").toEqual(catalogueG(matches.slice(1, 4)));
    expect(pointGroups(zeroPoint)).toEqual({
      "in fit": catalogueG(matches.slice(1, 4)),
      "rejected by clipping": catalogueG(matches.slice(4)),
      "saturated (not fitted)": catalogueG(matches.slice(0, 1)),
    });
  });

  it("draws the stars the fit clipped as rejected although they lie inside three times the final rms", () => {
    const residuals = [0.046, -0.046, 0, ...pairs(0.01, 10), ...pairs(0.04, 4)];
    const matches = residuals.map((r, i) => zeroPointMatch(i, r, i < 2 ? "clipped" : "used"));
    const { zeroPoint } = plotsOf(crossMatchResult(matches, { zero_point: fitOf(matches, 0.022590812902) }));
    expect(zeroPoint.series[0].x, "29 stars in the fit, 3 x rms = 0.0678 mag").toEqual(catalogueG(matches.slice(2)));
    expect(pointGroups(zeroPoint)["rejected by clipping"]).toEqual(catalogueG(matches.slice(0, 2)));
  });

  it("draws the stars the fit kept as in the fit although they lie outside three times the final rms", () => {
    const residuals = [0.085, -0.085, 0, ...pairs(0.02, 30)];
    const matches = residuals.map((r, i) => zeroPointMatch(i, r, "used"));
    const { zeroPoint } = plotsOf(crossMatchResult(matches, { zero_point: fitOf(matches, 0.024704604031) }));
    expect(zeroPoint.series[0].x, "63 stars in the fit, 3 x rms = 0.0741 mag").toEqual(catalogueG(matches));
    expect(pointGroups(zeroPoint)["rejected by clipping"]).toBeUndefined();
  });

  it("keeps the stars without BP-RP in their own series and leaves out a match without a status", () => {
    const matches = [zeroPointMatch(0, 0.04, "used", 0.4), zeroPointMatch(1, 0.09, "used", 0.8), zeroPointMatch(2, 0, "no_colour"), zeroPointMatch(3, 0, null)];
    const cross = crossMatchResult(matches, { zero_point: { ...fitOf(matches, 0.005), colour_coeff: 0.1, colour_term_used: true } });
    const { zeroPoint } = plotsOf(cross);
    expect(pointGroups(zeroPoint)).toEqual({
      "in fit": catalogueG(matches.slice(0, 2)),
      "no BP-RP (not in colour fit)": catalogueG(matches.slice(2, 3)),
    });
    expect(zeroPoint.series.find((s) => s.mode === "line")?.label).toBe("mag - ZP (25.000)");
  });

  it("labels the catalogue axis as a Vega magnitude", () => {
    const matches = [zeroPointMatch(0, 0, "used")];
    expect(plotsOf(crossMatchResult(matches)).zeroPoint.xLabel).toBe("Gaia G (Vega mag)");
    expect(plotsOf(crossMatchResult(matches, { band: "RP" })).zeroPoint.xLabel).toBe("Gaia RP (Vega mag)");
  });
});

describe("CrossMatchPlots source", () => {
  it("takes the zero point groups from the catalogue helpers, not from the plot helpers", () => {
    expect(plotsSource).toMatch(/from "\.\.\/\.\.\/utils\/zeroPointPlot"/);
    expect(plotsSource).not.toMatch(/plotInteraction/);
    expect(plotsSource).not.toMatch(/function catalogueMag\b/);
  });
});

describe("CrossMatchPlots residual plot", () => {
  afterEach(() => hooks.mount());

  it("labels both median lines as medians with their values and keeps the rms in the legend", () => {
    const { residuals } = plotsOf(crossMatchResult([zeroPointMatch(0, 0, "used")]));
    expect((residuals.referenceLines ?? []).map((l) => [l.axis, l.value, l.label])).toEqual([
      ["x", 0.01, 'median 0.010"'],
      ["y", 0.004, 'median 0.004"'],
    ]);
    expect(residuals.series.map((s) => s.label)).toEqual(['star - Gaia, rms 0.052"']);
  });
});

describe("CatalogPanel cross-match", () => {
  const SELECTED = "C:/data/x.fits";
  const CALIBRATED = "C:/out/x_calibrated.fits";
  const first = { filePath: SELECTED, measurePath: CALIBRATED, measureKey: `${CALIBRATED}@1` };
  type CatalogProps = typeof first;
  const catalogPanel = panel as unknown as (props: CatalogProps) => unknown;
  const render = (props: CatalogProps) => hooks.render(catalogPanel, props) as ReactNode;
  const matches = [zeroPointMatch(0, 0, "used"), zeroPointMatch(1, 0.01, "used"), zeroPointMatch(2, -0.01, "used")];

  function fieldLabelled(nodes: Props[], text: string): Props | undefined {
    const label = nodes.find((p) => typeof p.htmlFor === "string" && textOf(p.children as ReactNode).trim() === text);
    return nodes.find((p) => label !== undefined && p.id === label.htmlFor && typeof p.onChange === "function");
  }

  async function crossMatched(props: CatalogProps): Promise<void> {
    const run = elements(render(props)).find((p) => p.label === "Cross-match detected stars");
    expect(run?.disabled, "Cross-match detected stars is enabled").toBe(false);
    (run!.onClick as () => void)();
    await flush();
  }

  beforeEach(() => {
    hooks.mount();
    backend.cone.mockReset().mockResolvedValue(CONE);
    backend.crossMatch.mockReset().mockResolvedValue(crossMatchResult(matches));
    backend.exported = [];
    backend.headers = [];
  });

  afterEach(() => hooks.mount());

  it("states the automatic aperture of the zero point: 1.5 x FWHM per star, with the median and range of the radii in the fit", async () => {
    await crossMatched(first);
    const text = textOf(render(first));
    expect(text).toContain("ZP 25.000 ± 0.012");
    expect(text).toContain("aperture 1.5 x FWHM per star, median r = 7.06 px (4.62 to 9.06)");
  });

  it("states a fixed aperture with the radius the backend used", async () => {
    backend.crossMatch.mockResolvedValue(crossMatchResult(matches, { aperture: { kind: "fixed", radius_px: 5 } }));
    await crossMatched(first);
    expect(textOf(render(first))).toContain("aperture r = 5.00 px");
  });

  it("prints a zero point fitted on one star without an error", async () => {
    const single = [zeroPointMatch(0, 0, "used")];
    backend.crossMatch.mockResolvedValue(crossMatchResult(single, { zero_point: fitOf(single, 0, null) }));
    await crossMatched(first);
    expect(textOf(render(first))).toContain("ZP 25.000 ± --  n=1, 0 rejected");
  });

  it("sends the radius typed in Aperture r (px) and no radius when the field is blank", async () => {
    await crossMatched(first);
    expect((backend.crossMatch.mock.calls[0][1] as { apertureRadius?: number }).apertureRadius).toBeUndefined();
    const field = fieldLabelled(elements(render(first)), "Aperture r (px)");
    expect(field, "the panel has an Aperture r (px) field").toBeDefined();
    expect(field?.placeholder).toBe("auto");
    (field!.onChange as (text: string) => void)("6");
    await crossMatched(first);
    expect(backend.crossMatch.mock.calls[1]).toEqual([CALIBRATED, expect.objectContaining({ apertureRadius: 6 })]);
  });

  it("refuses an aperture radius outside 2 to 60 px with the range hint", () => {
    const field = fieldLabelled(elements(render(first)), "Aperture r (px)");
    expect(field, "the panel has an Aperture r (px) field").toBeDefined();
    (field!.onChange as (text: string) => void)("1");
    const nodes = elements(render(first));
    expect(nodes.find((p) => p.label === "Cross-match detected stars")?.disabled).toBe(true);
    expect(nodes.some((p) => textOf(p.children as ReactNode) === APERTURE_RANGE_HINT)).toBe(true);
  });

  it("clears the cross-match when a step re-publishes the measured image under the same path", async () => {
    await crossMatched(first);
    expect(textOf(render(first))).toContain("ZP 25.000");
    expect(elements(render(first)).some((p) => p.cross !== undefined)).toBe(true);
    const republished = { ...first, measureKey: `${CALIBRATED}@2` };
    render(republished);
    const after = render(republished);
    expect(textOf(after)).not.toContain("ZP 25.000");
    expect(elements(after).some((p) => p.cross !== undefined), "no plot of the previous version").toBe(false);
  });

  it("lists the warnings of the cross-match, the Gaia row cap and the cone limit among them", async () => {
    const warnings = [
      "VizieR row cap of 5000 reached: the Gaia list holds only the brightest stars in G within the cone, so fainter detections find no partner",
      "Gaia cone limited to 5 deg: the field reaches 7.7 deg from its centre, so detections beyond the cone find no partner",
    ];
    backend.crossMatch.mockResolvedValue(crossMatchResult(matches, { warnings }));
    await crossMatched(first);
    const nodes = elements(render(first));
    for (const warning of warnings) expect(nodes.some((p) => p.children === warning), warning).toBe(true);
  });

  const SAME_LIST = "Same Gaia list as the search";
  const COPY_TITLE = "Copy the matches (or the catalog when nothing is matched) as CSV, with the header lines of the saved file";

  function sameList(on: boolean): void {
    const toggle = elements(render(first)).find((p) => p.label === SAME_LIST);
    expect(toggle, `the panel has a ${SAME_LIST} toggle`).toBeDefined();
    (toggle!.onChange as (value: boolean) => void)(on);
  }

  function typeIn(label: string, text: string): void {
    const field = fieldLabelled(elements(render(first)), label);
    expect(field, `the panel has a ${label} field`).toBeDefined();
    (field!.onChange as (value: string) => void)(text);
  }

  async function shownAfter(result: CrossMatchResult): Promise<ReactNode> {
    hooks.mount();
    backend.crossMatch.mockResolvedValue(result);
    await crossMatched(first);
    return render(first);
  }

  async function searchedFirst(): Promise<void> {
    (elements(render(first)).find((p) => p.label === "Search Gaia DR3")!.onClick as () => void)();
    await flush();
  }

  const listRule = (nodes: Props[]) => textOf(nodes.find((p) => p["data-list-rule"] !== undefined)?.children as ReactNode);
  const csvLines = (text: string) => withoutExportTime(text.split("\r\n"));

  it("sends no cone and no G limit unless the list of the search is asked for", async () => {
    await crossMatched(first);
    const options = backend.crossMatch.mock.calls[0][1];
    for (const key of ["coneRadiusArcmin", "magLimit", "maxRows"]) expect(options).not.toHaveProperty(key);
    sameList(true);
    typeIn("Radius (arcmin)", "12");
    await crossMatched(first);
    expect(backend.crossMatch.mock.calls[1]).toEqual([CALIBRATED, expect.objectContaining({ coneRadiusArcmin: 12, magLimit: 18, maxRows: 5000 })]);
    typeIn("Radius (arcmin)", "");
    await crossMatched(first);
    expect(backend.crossMatch.mock.calls[2]).toEqual([CALIBRATED, expect.objectContaining({ coneRadiusArcmin: null, magLimit: 18 })]);
  });

  it("blocks the cross-match on a bad search field only while the list of the search is asked for", async () => {
    typeIn("G limit (mag)", "abc");
    const off = elements(render(first));
    expect(off.find((p) => p.label === "Cross-match detected stars")?.disabled).toBe(false);
    expect(listRule(off)).toBe("Cross-match list: every Gaia star in the field, no G limit.");
    sameList(true);
    const on = elements(render(first));
    const run = on.find((p) => p.label === "Cross-match detected stars");
    expect(run?.disabled).toBe(true);
    expect(listRule(on)).toBe("Cross-match list: the radius and G limit typed above.");
    (run!.onClick as () => void)();
    await flush();
    expect(backend.crossMatch).not.toHaveBeenCalled();
  });

  it("says which Gaia list the cross-match used and how the stars were detected", async () => {
    const listed = textOf(await shownAfter(crossMatchResult(matches, { ...LISTED })));
    expect(listed).toContain("Gaia list: 120 rows within 3.0' of the centre, no G limit, row cap 5000, from VizieR");
    expect(listed).toContain("detection at 5 sigma: 3 detections, at most 500 kept, 3 measured");
    const older = textOf(await shownAfter(crossMatchResult(matches)));
    expect(older).not.toContain("Gaia list:");
    expect(older).not.toContain("detection at");
  });

  it("names the system of the zero point and how to apply it", async () => {
    const shown = await shownAfter(crossMatchResult(matches));
    const text = textOf(shown);
    expect(text).toContain("Zero point (Gaia G, Vega)");
    expect(elements(shown).some((p) => p.title === "Gaia magnitudes are in the Vega system: this is not an AB zero point")).toBe(true);
    expect(text).toContain("Not applied to the image. Gaia G = mag_inst + 25.000 for a star measured in this aperture.");
    const red = textOf(await shownAfter(crossMatchResult(matches, { zero_point: { ...fitOf(matches, 0.02), colour_coeff: 0.08, colour_term_used: true } })));
    expect(red).toContain("Gaia G = mag_inst + 25.000 + 0.0800 x (BP-RP) for a star measured in this aperture.");
    const blue = textOf(await shownAfter(crossMatchResult(matches, { zero_point: { ...fitOf(matches, 0.02), colour_coeff: -0.08, colour_term_used: true } })));
    expect(blue).toContain("mag_inst + 25.000 - 0.0800 x (BP-RP)");
    expect(textOf(await shownAfter(crossMatchResult(matches, { zero_point: null })))).not.toContain("Not applied to the image");
    expect(textOf(await shownAfter(crossMatchResult(matches, { photcal_present: true })))).toContain(
      "Zero point (Gaia G, Vega) - informational, image already calibrated",
    );
  });

  it("prints the epoch of the cone as the exports do", async () => {
    backend.cone.mockResolvedValue({ ...CONE, epoch_year: null });
    await searched();
    expect(textOf(show())).toContain("epoch J2016.0 (no observation date in the header)");
    hooks.mount();
    backend.cone.mockResolvedValue(CONE);
    await searched();
    expect(textOf(show())).toContain("epoch J2024.50");
  });

  it("saves each CSV with the header lines of its kind", async () => {
    const result = crossMatchResult(matches, { ...LISTED });
    backend.crossMatch.mockResolvedValue(result);
    await searchedFirst();
    await crossMatched(first);
    for (const [i, label] of ["Matches CSV", "Sources CSV", "Catalog CSV"].entries()) {
      const button = elements(render(first)).find((p) => p.children !== undefined && textOf(p.children as ReactNode).trim() === label);
      expect(button?.disabled, `${label} is enabled`).toBe(false);
      (button!.onClick as () => void)();
      await vi.waitFor(() => expect(backend.exported).toHaveLength(i + 1));
      await flush();
    }
    expect(withoutExportTime(backend.headers[0]!)).toEqual(withoutExportTime(crossMatchCsvHeaderLines("matches", result, CONTEXT)));
    expect(withoutExportTime(backend.headers[1]!)).toEqual(withoutExportTime(crossMatchCsvHeaderLines("sources", result, CONTEXT)));
    expect(withoutExportTime(backend.headers[2]!)).toEqual(withoutExportTime(coneCsvHeaderLines(CONE, CONTEXT)));
    for (const lines of backend.headers) {
      expect(lines!.filter((l) => /^# exported_utc: \d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(l))).toHaveLength(1);
      expect(lines).toContain("# file: x_calibrated.fits");
    }
  });

  it("copies the CSV with the header lines", async () => {
    const copyButton = () => elements(render(first)).find((p) => typeof p.text === "function");
    expect(copyButton(), "no copy button before any search").toBeUndefined();
    const result = crossMatchResult(matches, { ...LISTED });
    backend.crossMatch.mockResolvedValue(result);
    await searchedFirst();
    const catalogCopy = copyButton();
    expect(catalogCopy?.title).toBe(COPY_TITLE);
    expect(csvLines((catalogCopy!.text as () => string | null)()!)).toEqual(
      csvLines(catalogueCsvText({ kind: "catalog", rows: CONE.rows }, coneCsvHeaderLines(CONE, CONTEXT))),
    );
    await crossMatched(first);
    const matchesCopy = copyButton();
    expect(matchesCopy?.title).toBe(COPY_TITLE);
    expect(csvLines((matchesCopy!.text as () => string | null)()!)).toEqual(
      csvLines(catalogueCsvText({ kind: "matches", matches: result.matches }, crossMatchCsvHeaderLines("matches", result, CONTEXT))),
    );
  });
});

describe("CatalogPanel source", () => {
  it("uses the shared small button class", () => {
    expect(panelSource).not.toMatch(/const SMALL_BUTTON_CLASS\s*=/);
    expect(panelSource).toMatch(/import \{[^}]*\bSMALL_BUTTON_CLASS\b[^}]*\} from "\.\.\/ui";/);
  });
});
