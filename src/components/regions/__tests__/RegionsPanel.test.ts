import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Region, RegionStats, RegionStatsEntry } from "../../../shared/types/regions";
import type { RegionStatsMeasured } from "../../../utils/measurementLog";
import type { RegionStatsUnits } from "../../../hooks/useRegionStats";

const live = vi.hoisted(() => ({
  regions: [] as Region[],
  stats: new Map<string, RegionStatsEntry>(),
  measured: null as RegionStatsMeasured | null,
  loading: false,
  notes: [] as string[],
  units: { bunit: null, summedUnit: null } as RegionStatsUnits,
  error: null as string | null,
  excludeDq: false,
}));
const useRegionStatsMock = vi.hoisted(() =>
  vi.fn((..._args: unknown[]) => ({
    stats: live.stats,
    loading: live.loading,
    error: live.error,
    measured: live.measured,
    notes: live.notes,
    units: live.units,
    photcal: null,
    calibrationWarnings: [] as string[],
    pixelAreaArcsec2: null,
  })),
);

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

vi.mock("../../../hooks/useRegionStore", () => ({
  useRegionDoc: () => ({ regions: live.regions, selectedId: null, version: 1 }),
}));
vi.mock("../../../hooks/useRegionStats", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../hooks/useRegionStats")>()),
  useRegionStats: useRegionStatsMock,
}));
vi.mock("../../../context/PreviewContext", () => ({ useDqContext: () => ({ excludeDq: live.excludeDq }) }));
vi.mock("../../../hooks/useMeasurementLog", () => ({ useMeasurementProvenance: () => ({ file: "sky.fits", image: null }) }));
vi.mock("../../../hooks/useAnalysisTarget", () => ({ useMeasurementSource: () => null }));
const savedTexts = vi.hoisted(() => [] as string[]);
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: async (dialog: { defaultPath: string }) => `C:/out/${dialog.defaultPath}` }));
vi.mock("@tauri-apps/plugin-fs", () => ({ writeTextFile: async (_path: string, text: string) => void savedTexts.push(text) }));

import RegionsPanel from "../RegionsPanel";
import { hooks } from "../../../test/hookRuntime";
import { elementsOf, findElement, textOf } from "../../../test/elementTree";
import { formatFluxDensityJy, formatWithUncertainty } from "../../../utils/formatUncertainty";
import { EXCLUDED_CELL_TITLE } from "../../../utils/regionExclude";
import { regionsTableCsv } from "../../../utils/regionCsv";
import { APP_VERSION } from "../../../utils/constants";
import { adHocErrorBoxLines } from "../../../test/panelRules";
import { CopyCsvButton } from "../../ui";
import regionsPanelSource from "../RegionsPanel.tsx?raw";

const PROPS = { filePath: "sky.fits", measurePath: "sky.fits", measureKey: "k" };

const BOX: Region = {
  id: "r1",
  shape: { shape: "box", x: 10, y: 10, width: 20, height: 20, angle: 0 },
  props: { color: null, width: null, text: null, dash: null, include: true },
  backgroundId: null,
};

function stats(overrides: Partial<RegionStats> = {}): RegionStats {
  return {
    count: 400,
    n_nan: 0,
    n_excluded: 0,
    n_padding: 0,
    area: 400,
    bounds: { x0: 0, y0: 0, x1: 20, y1: 20 },
    clipped: false,
    sum: 9.36,
    mean: 0.0041,
    median: 0.0234,
    mad: 0.00505,
    sigma: 0.00749,
    std: 0.0081,
    min: -0.02,
    max: 0.05,
    clipped_mean: null,
    clipped_median: null,
    clipped_sigma: null,
    n_rejected: 0,
    background: null,
    net_sum: null,
    net_snr: null,
    sum_err: null,
    weighted_mean: null,
    calibrated: null,
    sky: null,
    ...overrides,
  };
}

function measuredFor(regions: Region[]): RegionStatsMeasured {
  return { regions, excludeDq: false, sigma: 3, maxiters: 5, masked: false, dqExcluded: null, regionExcluded: 0, elapsedMs: 4 };
}

function show(s: RegionStats): string {
  live.stats = new Map([["r1", { id: "r1", stats: s, error: null }]]);
  return renderToStaticMarkup(createElement(RegionsPanel, PROPS));
}

function statCell(html: string, label: string): string {
  const start = html.indexOf(`<span class="text-zinc-600">${label} </span>`);
  expect(start, `${label} cell`).toBeGreaterThan(-1);
  const open = html.lastIndexOf('<span class="whitespace-nowrap"', start);
  return html.slice(open, html.indexOf("</span></span>", start) + "</span></span>".length);
}

function button(html: string, text: string): string {
  const chunk = html.split("<button").find((part) => part.includes(`${text}</button>`));
  expect(chunk, `${text} button`).toBeDefined();
  return chunk!.slice(0, chunk!.indexOf("</button>"));
}

beforeEach(() => {
  live.regions = [BOX];
  live.measured = measuredFor(live.regions);
  live.loading = false;
  live.notes = [];
  live.units = { bunit: null, summedUnit: null };
  live.error = null;
  live.excludeDq = false;
  useRegionStatsMock.mockClear();
});

describe("RegionsPanel row statistics with significant digits (trust-5)", () => {
  it("prints mean, median, sum and σ(MAD) of an HST drc sky box with four significant figures", () => {
    const html = show(stats());
    expect(html).toContain('<span class="text-zinc-300">0.0041</span>');
    expect(html).toContain('<span class="text-zinc-300">0.0234</span>');
    expect(html).toContain('<span class="text-zinc-300">9.36</span>');
    expect(html).toContain('<span class="text-zinc-300">0.00749</span>');
  });

  it("keeps the second significant figure between 0.001 and 0.01 and the same notation below 0.001", () => {
    expect(show(stats({ sigma: 0.00149 }))).toContain('<span class="text-zinc-300">0.00149</span>');
    expect(show(stats({ sigma: 0.00149 }))).not.toContain('<span class="text-zinc-300">0.001</span>');
    expect(show(stats({ sigma: 0.000999 }))).toContain('<span class="text-zinc-300">0.000999</span>');
  });

  it("prints the sum, its error and the weighted mean of a background-subtracted JWST i2d box with four significant figures", () => {
    const html = show(stats({ sum: 0.0936, sum_err: 0.00412, weighted_mean: 0.00398 }));
    expect(statCell(html, "sum")).toContain(`>${formatWithUncertainty(0.0936, 0.00412)}<`);
    expect(statCell(html, "wmean")).toContain(">0.00398<");
  });

  it("keeps the snr at one decimal", () => {
    const html = show(stats({ background: { median: 6, sigma: 1.4826, count: 204 }, net_sum: 585, net_snr: 68.55987143969055 }));
    expect(statCell(html, "snr")).toContain(">68.6<");
  });
});

describe("RegionsPanel net flux error (trust-1, front half)", () => {
  const BACKGROUND = { median: 6, sigma: 1.4826, count: 204 };

  it("prints net ± its error with the terms of the error in the title, from the background scatter without ERR", () => {
    const cell = statCell(
      show(
        stats({
          count: 29,
          background: BACKGROUND,
          net_sum: 585,
          net_err: 8.53268811209195,
          net_err_terms: "background_scatter+background_level",
          net_snr: 68.55987143969055,
        }),
      ),
      "net",
    );
    expect(cell).toContain(`<span class="text-zinc-300">${formatWithUncertainty(585, 8.53268811209195)}</span>`);
    expect(cell).toContain(
      'title="Error of the net sum: no ERR plane, so the pixel noise n σ_bg² from the background scatter, plus the background level term n² σ_bg² / n_bg"',
    );
  });

  it("names the ERR plane in the title when the error comes from it", () => {
    const cell = statCell(
      show(
        stats({
          count: 29,
          background: BACKGROUND,
          sum_err: 5.385164807134504,
          net_sum: 585,
          net_err: 6.169423504529033,
          net_err_terms: "err_plane+background_level",
          net_snr: 94.82247402379589,
        }),
      ),
      "net",
    );
    expect(cell).toContain(`<span class="text-zinc-300">${formatWithUncertainty(585, 6.169423504529033)}</span>`);
    expect(cell).toContain(
      'title="Error of the net sum: ERR plane of the region in quadrature with the background level term n² σ_bg² / n_bg"',
    );
  });

  it("prints the net sum alone and says why when the background has no scatter and there is no ERR noise", () => {
    const cell = statCell(
      show(stats({ background: { median: 5, sigma: 0, count: 204 }, net_sum: 0, net_err: null, net_err_terms: null, net_snr: null })),
      "net",
    );
    expect(cell).toContain('<span class="text-zinc-300">0</span>');
    expect(cell).not.toContain("±");
    expect(cell).toContain('title="No error: the background has zero scatter and the region has no ERR noise"');
  });

  it("explains that the snr is the net sum over its error", () => {
    const html = show(
      stats({ background: BACKGROUND, net_sum: 585, net_err: 8.53268811209195, net_err_terms: "background_scatter+background_level", net_snr: 68.55987143969055 }),
    );
    expect(statCell(html, "snr")).toContain('title="Net sum divided by its error"');
  });
});

describe("RegionsPanel CSV gated like Log (trust-10)", () => {
  const COPY_TITLE = "Copy the region table (statistics and calibrated fluxes) as CSV";
  const SAVE_TITLE = "Save the region table (statistics and calibrated fluxes) as a CSV file";
  const PENDING_TITLE = "Waiting for statistics that match the current regions, clipping and DQ settings";

  it("disables Copy CSV and Save CSV while the statistics on screen belong to the region before it was moved", () => {
    const moved: Region = { ...BOX, shape: { shape: "box", x: 60, y: 10, width: 20, height: 20, angle: 0 } };
    live.measured = measuredFor([BOX]);
    live.regions = [moved];
    const html = show(stats());
    for (const text of ["Copy CSV", "Save CSV", "Log"]) expect(button(html, text)).toContain('disabled=""');
    expect(button(html, "Copy CSV")).toContain(`title="${PENDING_TITLE}"`);
    expect(button(html, "Save CSV")).toContain(`title="${PENDING_TITLE}"`);
  });

  it("disables them while a recompute runs for the same regions", () => {
    live.loading = true;
    const html = show(stats());
    expect(button(html, "Copy CSV")).toContain('disabled=""');
    expect(button(html, "Save CSV")).toContain('disabled=""');
  });

  it("enables them with their own titles once the statistics match the regions, clipping and DQ settings", () => {
    const html = show(stats());
    expect(button(html, "Copy CSV")).not.toContain('disabled=""');
    expect(button(html, "Save CSV")).not.toContain('disabled=""');
    expect(button(html, "Copy CSV")).toContain(`title="${COPY_TITLE}"`);
    expect(button(html, "Save CSV")).toContain(`title="${SAVE_TITLE}"`);
  });

  it("keeps the plain titles on the disabled buttons of an empty region list", () => {
    live.regions = [];
    live.measured = null;
    live.stats = new Map();
    const html = renderToStaticMarkup(createElement(RegionsPanel, PROPS));
    expect(button(html, "Copy CSV")).toContain('disabled=""');
    expect(button(html, "Copy CSV")).toContain(`title="${COPY_TITLE}"`);
    expect(button(html, "Save CSV")).toContain(`title="${SAVE_TITLE}"`);
  });
});

const DQ_NOTE = "DQ exclusion requested but not applied: this image has no DQ plane";
const SIGMA_LABEL = "Sigma clipping threshold";
const ITERS_LABEL = "Sigma clipping iterations";

type PanelFunction = (props: typeof PROPS) => unknown;
const panelFunction = (RegionsPanel as unknown as { type: PanelFunction }).type;

interface ClipFieldProps {
  value: string;
  error: string | null;
  onChange: (text: string) => void;
}

function clipField(tree: unknown, label: string): ClipFieldProps {
  const element = findElement<Record<string, unknown>>(tree, (e) => e.props["aria-label"] === label);
  expect(element, label).toBeDefined();
  return element!.props as unknown as ClipFieldProps;
}

function clipErrorLines(tree: unknown) {
  return elementsOf<Record<string, unknown>>(tree).filter((e) => e.props["data-clip-error"] !== undefined);
}

function emptyListMarkup(): string {
  live.regions = [];
  live.measured = null;
  live.stats = new Map();
  return renderToStaticMarkup(createElement(RegionsPanel, PROPS));
}

function notesBlock(html: string): string | null {
  const start = html.indexOf("data-stats-notes");
  return start < 0 ? null : html.slice(start, html.indexOf("</div></div>", start));
}

function dqChipTitle(html: string): string | undefined {
  return html.match(/<span[^>]*title="([^"]*)"[^>]*>DQ not applied<\/span>/)?.[1];
}

describe("RegionsPanel notes, DQ chip and unit chip (fits-layers/U2)", () => {
  it("shows the notes of the statistics command, such as a DQ exclusion that was not applied", () => {
    live.notes = [DQ_NOTE];
    expect(notesBlock(show(stats()))).toContain(DQ_NOTE);
    live.notes = [];
    expect(show(stats())).not.toContain("data-stats-notes");
    live.notes = [DQ_NOTE];
    expect(emptyListMarkup()).not.toContain("data-stats-notes");
  });

  it("stops saying DQ masked when the mask was not applied", () => {
    live.excludeDq = true;
    live.notes = [DQ_NOTE];
    live.measured = { ...measuredFor([BOX]), excludeDq: true, dqExcluded: null };
    let html = show(stats());
    expect(dqChipTitle(html)).toBe(DQ_NOTE);
    expect(html).not.toContain(">DQ masked<");
    live.measured = { ...measuredFor([BOX]), excludeDq: true, dqExcluded: 12 };
    html = show(stats());
    expect(html).toContain(">DQ masked<");
    expect(html).not.toContain("DQ not applied");
    live.measured = null;
    expect(show(stats())).toContain(">DQ masked<");
    live.excludeDq = false;
    live.measured = { ...measuredFor([BOX]), excludeDq: true, dqExcluded: null };
    html = show(stats());
    expect(html).not.toContain("DQ masked");
    expect(html).not.toContain("DQ not applied");
  });

  it("titles the DQ chip with a plain sentence when no note gives the reason", () => {
    live.excludeDq = true;
    live.measured = { ...measuredFor([BOX]), excludeDq: true, dqExcluded: null };
    expect(dqChipTitle(show(stats()))).toBe("No DQ mask was applied to these statistics");
  });

  it("names the pixel unit of the image and the unit of a sum", () => {
    const unitChip = (html: string) => html.match(/<span[^>]*title="([^"]*)"[^>]*data-region-unit[^>]*>([^<]*)<\/span>/);
    live.units = { bunit: "MJy/sr", summedUnit: "MJy/sr x pix" };
    const chip = unitChip(show(stats()));
    expect(chip?.[2]).toBe("unit MJy/sr");
    expect(chip?.[1]).toBe("mean, median, sigma, min and max are pixel values in MJy/sr; sum and net are sums over pixels in MJy/sr x pix");
    live.units = { bunit: null, summedUnit: null };
    expect(show(stats())).not.toContain("data-region-unit");
    live.units = { bunit: "adu", summedUnit: null };
    expect(unitChip(show(stats()))?.[1].endsWith("in adu")).toBe(true);
  });
});

describe("RegionsPanel clip fields (a11y-3)", () => {
  beforeEach(() => hooks.mount());

  it("says in words when a clip entry is not a usable number and which default is used", () => {
    let tree = hooks.render(panelFunction, PROPS);
    clipField(tree, SIGMA_LABEL).onChange("2,5");
    tree = hooks.render(panelFunction, PROPS);
    expect(clipField(tree, SIGMA_LABEL).value).toBe("2,5");
    expect(clipField(tree, SIGMA_LABEL).error).toBe("Use a dot (.) as the decimal separator");
    let lines = clipErrorLines(tree);
    expect(lines.map((e) => e.props["data-clip-error"])).toEqual(["sigma"]);
    expect(textOf(lines[0])).toBe("sigma: Use a dot (.) as the decimal separator; using the default 3");
    expect(lines[0].props.role).toBe("status");

    clipField(tree, SIGMA_LABEL).onChange("2.5");
    tree = hooks.render(panelFunction, PROPS);
    expect(clipErrorLines(tree)).toEqual([]);
    expect(clipField(tree, SIGMA_LABEL).error).toBeNull();

    clipField(tree, ITERS_LABEL).onChange("7.5");
    tree = hooks.render(panelFunction, PROPS);
    lines = clipErrorLines(tree);
    expect(lines.map((e) => e.props["data-clip-error"])).toEqual(["iters"]);
    expect(textOf(lines[0])).toBe("iters: Use a whole number; using the default 5");

    clipField(tree, SIGMA_LABEL).onChange("2,5");
    tree = hooks.render(panelFunction, PROPS);
    expect(clipErrorLines(tree).map((e) => e.props["data-clip-error"])).toEqual(["sigma", "iters"]);

    const html = show(stats());
    expect(html).not.toContain("data-clip-error");
    expect(html).not.toContain("aria-invalid");
  });

  it("keeps sending the parsed clip to the statistics hook", () => {
    const lastClip = () => useRegionStatsMock.mock.calls.at(-1)?.[3];
    let tree = hooks.render(panelFunction, PROPS);
    clipField(tree, SIGMA_LABEL).onChange("2.5");
    tree = hooks.render(panelFunction, PROPS);
    expect(lastClip()).toEqual({ sigma: 2.5, maxiters: 5 });
    clipField(tree, SIGMA_LABEL).onChange("2,5");
    hooks.render(panelFunction, PROPS);
    expect(lastClip()).toEqual({ sigma: null, maxiters: 5 });
  });
});

describe("RegionsPanel row values (front-photometry/U3, trust-4)", () => {
  const CALIBRATED = {
    flux_source: "net" as const,
    flux_native: 31.2890625,
    flux_err_native: 0.860241559892618,
    flux_jy: 6.2578125e-7,
    flux_err_jy: 1.720483119785236e-8 as number | null,
    mag_ab: 24.408943633909075,
    mag_ab_err: 0.029850539830428725 as number | null,
    st_mag: null,
    area_arcsec2: null,
    geometric_area_arcsec2: null,
    sb_mag_arcsec2: null,
    ra: null,
    dec: null,
    pa_sky_deg: null,
  };

  it("prints the calibrated flux with one prefix and the magnitude rounded to its error", () => {
    let html = show(stats({ calibrated: CALIBRATED }));
    expect(statCell(html, "Jy")).toContain(`>${formatFluxDensityJy(6.2578125e-7, 1.720483119785236e-8)}<`);
    expect(statCell(html, "AB")).toContain(`>${formatWithUncertainty(24.408943633909075, 0.029850539830428725)}<`);
    html = show(stats({ calibrated: { ...CALIBRATED, flux_err_jy: null, mag_ab_err: null } }));
    expect(statCell(html, "Jy")).toContain(`>${formatFluxDensityJy(6.2578125e-7, null)}<`);
    expect(statCell(html, "AB")).toContain(">24.409<");
  });

  it("shows the clipped statistics the clip fields control when pixels were rejected", () => {
    live.measured = { ...measuredFor([BOX]), sigma: 2.5, maxiters: 10 };
    const cell = statCell(show(stats({ n_rejected: 12, clipped_mean: 0.004, clipped_sigma: 0.0071 })), "sigma-clip");
    expect(cell).toContain(">mean 0.004, sigma 0.0071, 12 rejected<");
    expect(cell).toContain(
      'title="After sigma clipping at 2.5 sigma and at most 10 iterations: 12 of 400 pixels rejected. Every other value of this row uses all the pixels."',
    );
    expect(show(stats({ n_rejected: 0, clipped_mean: 0.004, clipped_sigma: 0.0071 }))).not.toContain(">sigma-clip <");
    live.measured = { ...measuredFor([BOX]), sigma: null, maxiters: null };
    expect(statCell(show(stats({ n_rejected: 12, clipped_mean: 0.004, clipped_sigma: 0.0071 })), "sigma-clip")).toContain(
      "3 sigma and at most 5 iterations",
    );
  });

  it("says partly off image for a region cut by the image edge", () => {
    const html = show(stats({ clipped: true }));
    expect(html).toMatch(/title="Part of the region lies outside the image; the statistics use the pixels inside"[^>]*>partly off image</);
    expect(html).not.toContain(">clipped<");
  });

  it("counts the zeros and the pixels without data it left out", () => {
    let cell = statCell(show(stats({ count: 320, n_padding: 72, n_nan: 8 })), "n");
    expect(cell).toContain(">320 (−72 zero) (−8 no data)<");
    expect(cell).toContain('title="exact zeros are treated as padding and left out; pixels without data (NaN or infinite) are left out"');
    cell = statCell(show(stats({ count: 320, n_padding: 72, n_nan: 8, n_excluded: 5 })), "n");
    expect(cell).toContain(">320 (−5 excl) (−72 zero) (−8 no data)<");
    expect(cell).toContain('title="pixels removed by DQ flags and/or exclude regions; ');
    cell = statCell(show(stats({ count: 320, n_excluded: 5 })), "n");
    expect(cell).toContain(`title="${EXCLUDED_CELL_TITLE}"`);
  });
});

describe("RegionsPanel CSV provenance (trust-7)", () => {
  const writeText = vi.fn((_text: string) => Promise.resolve());

  beforeEach(() => {
    hooks.mount();
    savedTexts.length = 0;
    writeText.mockClear();
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    live.stats = new Map([["r1", { id: "r1", stats: stats(), error: null }]]);
    live.units = { bunit: "MJy/sr", summedUnit: "MJy/sr x pix" };
    live.notes = [DQ_NOTE];
  });

  afterEach(() => vi.unstubAllGlobals());

  function click(text: string): void {
    const tree = hooks.render(panelFunction, PROPS);
    const found = findElement<Record<string, unknown>>(tree, (e) => e.type === "button" && textOf(e).trim() === text);
    expect(found?.props.disabled, `${text} is enabled`).not.toBe(true);
    (found!.props.onClick as () => void)();
  }

  const withoutExportTime = (text: string) =>
    text
      .split("\r\n")
      .filter((line) => !line.startsWith("# exported_utc: "))
      .join("\r\n");

  function copyText(): string | null {
    const copyButton = findElement<Record<string, unknown>>(hooks.render(panelFunction, PROPS), (e) => e.type === CopyCsvButton);
    expect(copyButton, "Copy CSV button").toBeDefined();
    return (copyButton!.props.text as () => string | null)();
  }

  it("copies the region table with its provenance", () => {
    const copied = copyText()!;
    expect(copied.startsWith(`# software: AstroBurst ${APP_VERSION}\r\n`)).toBe(true);
    const lines = copied.split("\r\n");
    for (const line of ["# file: sky.fits", "# dq_exclusion: off", `# note: ${DQ_NOTE}`, "# unit sum: MJy/sr x pix"]) expect(lines).toContain(line);
    expect(copied.endsWith(regionsTableCsv(live.regions, live.stats))).toBe(true);
  });

  it("saves the same text", async () => {
    const copied = copyText()!;
    click("Save CSV");
    await vi.waitFor(() => expect(savedTexts).toHaveLength(1));
    expect(withoutExportTime(savedTexts[0])).toBe(withoutExportTime(copied));
  });
});

describe("RegionsPanel error box (visual-8, a11y-8)", () => {
  it("announces a failure through the shared error alert", () => {
    live.error = "region statistics failed: HDU 3 is not an image";
    const html = show(stats());
    expect(html).toMatch(/role="alert"[^>]*>region statistics failed: HDU 3 is not an image</);
    expect(html).not.toContain("text-red-400 bg-red-900/20");
  });

  it("holds no hand-made failure box", () => {
    expect(adHocErrorBoxLines(regionsPanelSource)).toEqual([]);
  });

  it("announces a statistics failure through the compact alert", () => {
    live.error = "region 1 lies outside the image";
    expect(show(stats())).toContain('<div class="ab-error-alert ab-error-alert-compact" role="alert">region 1 lies outside the image</div>');
  });
});

describe("RegionsPanel shared controls (flows-14, a11y-17, visual-16)", () => {
  it("confirms the region copy through the shared button and keeps its gate", () => {
    const html = show(stats());
    expect(button(html, "Copy CSV")).toContain(
      'class="flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded text-zinc-400 hover:text-zinc-200 disabled:opacity-40"',
    );
    expect(html).toContain('Copy CSV</button><span role="status" class="sr-only" data-copy-status=""></span>');
  });

  it("gives the delete icon a 24 px target", () => {
    const chunk = show(stats())
      .split("<button")
      .find((part) => part.includes('aria-label="Delete region"'));
    expect(chunk, "delete button").toBeDefined();
    const deleteButton = chunk!.slice(0, chunk!.indexOf("</button>"));
    expect(deleteButton).toContain("min-w-6");
    expect(deleteButton).toContain("min-h-6");
  });

  it("gives the clip fields a focus ring of their own", () => {
    const clipInputs = show(stats())
      .split("<input")
      .slice(1)
      .map((part) => part.slice(0, part.indexOf(">")))
      .filter((tag) => /class="[^"]*\bw-9\b[^"]*\btext-right\b/.test(tag));
    expect(clipInputs).toHaveLength(2);
    for (const tag of clipInputs) {
      expect(tag).toContain("focus-visible:ring-1");
      expect(tag).toContain("focus-visible:ring-sky-500/60");
    }
  });
});
