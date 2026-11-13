import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Region, RegionStats, RegionStatsEntry } from "../../../shared/types/regions";
import type { RegionStatsMeasured } from "../../../utils/measurementLog";

const live = vi.hoisted(() => ({
  regions: [] as Region[],
  stats: new Map<string, RegionStatsEntry>(),
  measured: null as RegionStatsMeasured | null,
  loading: false,
}));

vi.mock("../../../hooks/useRegionStore", () => ({
  useRegionDoc: () => ({ regions: live.regions, selectedId: null, version: 1 }),
}));
vi.mock("../../../hooks/useRegionStats", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../hooks/useRegionStats")>()),
  useRegionStats: () => ({
    stats: live.stats,
    loading: live.loading,
    error: null,
    measured: live.measured,
    photcal: null,
    calibrationWarnings: [],
    pixelAreaArcsec2: null,
  }),
}));
vi.mock("../../../context/PreviewContext", () => ({ useDqContext: () => ({ excludeDq: false }) }));
vi.mock("../../../hooks/useMeasurementLog", () => ({ useMeasurementProvenance: () => ({ file: "sky.fits", image: null }) }));
vi.mock("../../../hooks/useAnalysisTarget", () => ({ useMeasurementSource: () => null }));

import RegionsPanel from "../RegionsPanel";

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
    expect(statCell(html, "sum")).toContain(">0.0936 ± 0.00412<");
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
    expect(cell).toContain('<span class="text-zinc-300">585 ± 8.533</span>');
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
    expect(cell).toContain('<span class="text-zinc-300">585 ± 6.169</span>');
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
