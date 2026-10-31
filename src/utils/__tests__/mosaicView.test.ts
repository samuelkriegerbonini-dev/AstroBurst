import { describe, expect, it } from "vitest";
import type { ProcessedFile } from "../../shared/types";
import type { MosaicEstimate, MosaicResult, MosaicTileRow } from "../../shared/types/stacking";
import { binsFromMasters } from "../pipelineToCompose";
import { DEFAULT_BINS } from "../wizard";
import type { SessionMosaic } from "../mosaicStore";
import {
  CHOOSE_CANVAS,
  DEFAULT_MOSAIC_SETTINGS,
  NEW_CANVAS,
  canvasChoices,
  canvasSummary,
  composePlan,
  estimateDisabledReason,
  estimateNotes,
  groupViews,
  memoryText,
  mixedFilterLine,
  mosaicChannelOf,
  mosaicOutputName,
  mosaicProgressText,
  mosaicRequest,
  mosaicRequestKey,
  mosaicSolveRequest,
  mosaicViewOutput,
  panelFilterGroups,
  rejectionLine,
  residualViews,
  runDisabledReason,
  tileViews,
  withReference,
  type MosaicEstimateRecord,
  type MosaicGate,
} from "../mosaicView";

const PATHS = ["C:/m42/panels/a.fits", "C:/m42/panels/b.fits", "C:/m42/panels/c.fits"];
const SOLVE = mosaicSolveRequest({ onlineGaia: false, useTwomass: true, maxSipOrder: 3 });

function tile(name: string, extra: Partial<MosaicTileRow> = {}): MosaicTileRow {
  return {
    name,
    path: `C:/m42/panels/${name}.fits`,
    width: 4000,
    height: 3000,
    planes: 1,
    bunit: null,
    single_exposure: false,
    solve: null,
    coverage_frac: 0.4,
    scale_ratio: 1,
    rotation_deg: 0.04,
    wcs_kind: "header",
    window: { x0: 0, y0: 0, cols: 4000, rows: 3000 },
    ...extra,
  };
}

const SOLVED = { solver: "local", tier: 0, catalog: "tier0-tycho2", matched: 42, rms_px: 0.314 };

const ESTIMATE: MosaicEstimate = {
  cols: 9000,
  rows: 3200,
  megapixels: 28.8,
  ram_bytes: 1.5 * 2 ** 30,
  canvas_cards: [["CTYPE1", "'RA---TAN'"]],
  tiles: [
    tile("m42_p1_Ha"),
    tile("m42_p2_Ha", { wcs_kind: "gwcs", scale_ratio: 1.04, rotation_deg: -1.24, coverage_frac: 0.355 }),
    tile("m42_p3_Ha", { solve: SOLVED, rotation_deg: null, single_exposure: true }),
  ],
  pairs: [{ i: 0, j: 1, frac: 0.2 }],
  components: [{ tiles: [0, 1], reference: 0 }, { tiles: [2], reference: 2 }],
  isolated: [2],
  warnings: [],
  refusal: null,
  default_reject: null,
  reject: null,
  planes: 1,
};

function lightFile(name: string, filter: string | null): ProcessedFile {
  const header: Record<string, string> = filter === null ? { IMAGETYP: "LIGHT" } : { IMAGETYP: "LIGHT", FILTER: filter };
  return {
    id: name,
    name,
    path: `C:/m42/panels/${name}`,
    sourcePath: `C:/m42/panels/${name}`,
    imageRef: null,
    size: 0,
    status: "done",
    result: { header } as ProcessedFile["result"],
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function sessionMosaic(name: string, label: SessionMosaic["label"], planes = 1): SessionMosaic {
  return { fitsPath: `C:/out/${name}.fits`, written: [`C:/out/${name}.fits`, `C:/out/${name}.png`], name, label, planes, dimensions: [9000, 3200], panels: 3 };
}

describe("mosaicRequest", () => {
  it("turns the Solve options into the tile solve of the request", () => {
    expect(SOLVE).toEqual({ online: false, use_twomass: true, max_sip_order: 3 });
    expect(mosaicSolveRequest({ onlineGaia: true, useTwomass: false, maxSipOrder: "auto" })).toEqual({ online: true, use_twomass: false, max_sip_order: null });
  });

  it("sends a new canvas, offset levels, a 64 px feather, the DQ mask and automatic rejection by default", () => {
    expect(mosaicRequest(PATHS, DEFAULT_MOSAIC_SETTINGS, SOLVE)).toEqual({
      canvas: { mode: "auto", north_up: false, reference: null },
      levels: "offset",
      references: [],
      feather_px: 64,
      dq_mask: true,
      reject: { mode: "auto" },
      solve: SOLVE,
    });
  });

  it("lines the canvas axes up with the chosen panel unless north is up or the panel is not picked", () => {
    expect(mosaicRequest(PATHS, { ...DEFAULT_MOSAIC_SETTINGS, axesOf: "C:\\m42\\panels\\b.fits" }, SOLVE).canvas).toEqual({ mode: "auto", north_up: false, reference: 1 });
    expect(mosaicRequest(PATHS, { ...DEFAULT_MOSAIC_SETTINGS, axesOf: PATHS[1], northUp: true }, SOLVE).canvas).toEqual({ mode: "auto", north_up: true, reference: null });
    expect(mosaicRequest(PATHS, { ...DEFAULT_MOSAIC_SETTINGS, axesOf: "C:/elsewhere/d.fits" }, SOLVE).canvas).toEqual({ mode: "auto", north_up: false, reference: null });
  });

  it("uses the canvas of a previous mosaic, offset + scale, chosen references and rejection set by hand", () => {
    const settings = { ...DEFAULT_MOSAIC_SETTINGS, canvasOf: "C:/out/ha_mosaic.fits", levels: "offset_scale" as const, references: [null, 2], featherPx: 128, dqMask: false, reject: "on" as const, snr: 4, scale: 0.8 };
    expect(mosaicRequest(PATHS, settings, SOLVE)).toEqual({
      canvas: { mode: "like", path: "C:/out/ha_mosaic.fits" },
      levels: "offset_scale",
      references: [null, 2],
      feather_px: 128,
      dq_mask: false,
      reject: { mode: "on", snr: 4, scale: 0.8 },
      solve: SOLVE,
    });
    expect(mosaicRequest(PATHS, { ...DEFAULT_MOSAIC_SETTINGS, reject: "off" }, SOLVE).reject).toEqual({ mode: "off" });
  });

  it("sets the level reference of one group and leaves the other groups to their first panel", () => {
    expect(withReference([], 2, 3)).toEqual([null, null, 3]);
    expect(withReference([0, null], 1, 4)).toEqual([0, 4]);
  });

  it("keys the request by panels and settings, not by the solve options", () => {
    const key = mosaicRequestKey(PATHS, mosaicRequest(PATHS, DEFAULT_MOSAIC_SETTINGS, SOLVE));
    expect(mosaicRequestKey(PATHS, mosaicRequest(PATHS, DEFAULT_MOSAIC_SETTINGS, { ...SOLVE, online: true }))).toBe(key);
    expect(mosaicRequestKey(PATHS.slice(0, 2), mosaicRequest(PATHS, DEFAULT_MOSAIC_SETTINGS, SOLVE))).not.toBe(key);
    expect(mosaicRequestKey(PATHS, mosaicRequest(PATHS, { ...DEFAULT_MOSAIC_SETTINGS, featherPx: 32 }, SOLVE))).not.toBe(key);
  });
});

describe("disabled reasons", () => {
  const record = (result: MosaicEstimate, key = "now"): MosaicEstimateRecord => ({ key, paths: PATHS, request: mosaicRequest(PATHS, DEFAULT_MOSAIC_SETTINGS, SOLVE), result });
  const gate = (extra: Partial<MosaicGate>): MosaicGate => ({ panels: 3, busy: null, estimate: null, key: "now", ...extra });

  it("Estimate needs two panels and waits for a run in flight", () => {
    expect(estimateDisabledReason(gate({ panels: 1 }))).toBe("Pick at least two panels.");
    expect(estimateDisabledReason(gate({ busy: "run" }))).toBe("Wait for the mosaic run to finish.");
    expect(estimateDisabledReason(gate({ busy: "estimate" }))).toBeNull();
    expect(estimateDisabledReason(gate({}))).toBeNull();
  });

  it("Run waits for an estimate of the current panels and settings that refuses nothing", () => {
    expect(runDisabledReason(gate({ panels: 1 }))).toBe("Pick at least two panels.");
    expect(runDisabledReason(gate({}))).toBe("Estimate first: Run unlocks once the estimate has checked the canvas, the memory and the panel groups.");
    expect(runDisabledReason(gate({ busy: "estimate" }))).toBe("Wait for the estimate to finish.");
    expect(runDisabledReason(gate({ estimate: record(ESTIMATE, "before") }))).toBe("The panels or settings changed since the estimate: Estimate again.");
    expect(runDisabledReason(gate({ estimate: record({ ...ESTIMATE, refusal: "The canvas would be 52000x3200 px." }) }))).toBe(
      "The estimate refused this mosaic: change what it names, then Estimate again.",
    );
    expect(runDisabledReason(gate({ estimate: record(ESTIMATE) }))).toBeNull();
    expect(runDisabledReason(gate({ busy: "run", estimate: record(ESTIMATE) }))).toBeNull();
  });
});

describe("estimate text", () => {
  it("states the memory in the backend's binary units", () => {
    expect(memoryText(1.5 * 2 ** 30)).toBe("1.5 GB");
    expect(memoryText(300 * 2 ** 20)).toBe("300.0 MB");
  });

  it("sums up the canvas size, megapixels and memory, and says nothing for a canvas that could not be built", () => {
    expect(canvasSummary(ESTIMATE)).toBe("Canvas 9,000 × 3,200 px · 28.8 MP · about 1.5 GB of memory");
    expect(canvasSummary({ ...ESTIMATE, planes: 3 })).toBe("Canvas 9,000 × 3,200 px · 28.8 MP · about 1.5 GB of memory · RGB");
    expect(canvasSummary({ ...ESTIMATE, cols: 0, rows: 0 })).toBeNull();
  });

  it("lists the groups with their level references and warns about an isolated panel", () => {
    expect(groupViews(ESTIMATE)).toEqual([
      { component: 0, number: 1, panels: [{ index: 0, name: "m42_p1_Ha" }, { index: 1, name: "m42_p2_Ha" }], reference: 0 },
    ]);
    expect(estimateNotes(ESTIMATE)).toEqual(["Panel 'm42_p3_Ha' overlaps no other panel: it keeps its own level, not matched to the others."]);
  });

  it("numbers only the groups of two or more panels and says levels are not matched between groups", () => {
    const split: MosaicEstimate = {
      ...ESTIMATE,
      tiles: ["a", "b", "c", "d", "e"].map((name) => tile(name)),
      components: [{ tiles: [0, 1], reference: 1 }, { tiles: [2], reference: 2 }, { tiles: [3, 4], reference: 3 }],
      isolated: [2],
    };
    expect(groupViews(split).map((g) => [g.component, g.number, g.reference])).toEqual([[0, 1, 1], [2, 2, 3]]);
    expect(estimateNotes(split)).toEqual([
      "The panels form 2 groups that do not overlap: levels are matched within each group, not between groups.",
      "Panel 'c' overlaps no other panel: it keeps its own level, not matched to the others.",
    ]);
  });

  it("gives every panel its group, WCS kind, scale, rotation, coverage and solve", () => {
    expect(tileViews(ESTIMATE)).toEqual([
      { index: 0, name: "m42_p1_Ha", group: "Group 1", reference: true, wcs: "Header WCS", scale: "1.00×", rotation: "0.0°", coverage: "40.0 % of the canvas", solve: null, singleExposure: false },
      { index: 1, name: "m42_p2_Ha", group: "Group 1", reference: false, wcs: "gWCS", scale: "1.04×", rotation: "-1.2°", coverage: "35.5 % of the canvas", solve: null, singleExposure: false },
      {
        index: 2,
        name: "m42_p3_Ha",
        group: "Isolated",
        reference: false,
        wcs: "Solved from its header",
        scale: "1.00×",
        rotation: "--",
        coverage: "40.0 % of the canvas",
        solve: "Offline solve · Tier 0: Tycho-2 (bundled) · 42 stars · RMS 0.31 px",
        singleExposure: true,
      },
    ]);
  });

  it("keeps the panel rows of a canvas that could not be built", () => {
    const bare: MosaicTileRow = { name: "far", path: "C:/m42/panels/far.fits", width: 10, height: 10, planes: 1, bunit: null, single_exposure: false, solve: SOLVED };
    expect(tileViews({ tiles: [bare], components: [], isolated: [] })).toEqual([
      { index: 0, name: "far", group: null, reference: false, wcs: "Solved from its header", scale: "--", rotation: "--", coverage: "--", solve: "Offline solve · Tier 0: Tycho-2 (bundled) · 42 stars · RMS 0.31 px", singleExposure: false },
    ]);
  });

  it("explains the rejection that Auto picks from the panel kind, and the hand-set modes", () => {
    const level2 = { ...ESTIMATE, tiles: ESTIMATE.tiles.map((t) => ({ ...t, single_exposure: true })) };
    expect(rejectionLine("auto", ESTIMATE)).toBe("Auto: outlier rejection is off for stacked panels; choose On to reject outliers where three or more panels overlap.");
    expect(rejectionLine("auto", { ...level2, reject: { snr: 3.5, scale: 1.2 } })).toBe(
      "Auto: outlier rejection is on (SNR 3.5, scale 1.2), as every panel is a single exposure and three or more panels overlap.",
    );
    expect(rejectionLine("auto", level2)).toBe("Auto: outlier rejection is off, as no three panels overlap.");
    expect(rejectionLine("on", { ...ESTIMATE, reject: { snr: 4, scale: 0.8 } })).toBe(
      "Outlier rejection is on (SNR 4, scale 0.8) where three or more panels overlap; where only two overlap nothing is rejected.",
    );
    expect(rejectionLine("off", ESTIMATE)).toBe("Outlier rejection is off.");
  });
});

describe("mosaicProgressText", () => {
  it("names the stage and the panel of every progress event", () => {
    expect(mosaicProgressText("", "estimate")).toBe("Reading the panels...");
    expect(mosaicProgressText("", "run")).toBe("Starting the mosaic...");
    expect(mosaicProgressText("solve:2:2/3:m42_p2_Ha", "estimate")).toBe("Solving m42_p2_Ha (2/3)");
    expect(mosaicProgressText("overlaps:1:1/3:m42_p1_Ha", "run")).toBe("Measuring overlaps: m42_p1_Ha (1/3)");
    expect(mosaicProgressText("levels:0:1/1:", "run")).toBe("Matching levels");
    expect(mosaicProgressText("reject:3:3/3:m42_p3_Ha", "run")).toBe("Finding outliers: m42_p3_Ha (3/3)");
    expect(mosaicProgressText("warp:2:2/3:m42_p2_Ha", "run")).toBe("Reprojecting m42_p2_Ha (2/3)");
    expect(mosaicProgressText("residuals:3:1/2:m42_p3_Ha", "run")).toBe("Checking star positions (1/2 overlaps)");
    expect(mosaicProgressText("saving:0:1/1:m42_p1_Ha_mosaic3.fits", "run")).toBe("Saving m42_p1_Ha_mosaic3.fits");
    expect(mosaicProgressText("complete", "run")).toBe("Finishing");
    expect(mosaicProgressText("other:1", "run")).toBe("other");
  });
});

describe("after the run", () => {
  it("reports the star offset of every overlap and flags the pairs above 0.5 px by name", () => {
    const result = {
      tiles: [tile("left"), tile("right"), tile("top")],
      pair_residuals: [
        { i: 0, j: 1, dx: 0.8, dy: 0.6, matched: 34, rms_px: 0.2 },
        { i: 0, j: 2, dx: 0.1, dy: -0.05, matched: 1, rms_px: 0.1 },
        { i: 1, j: 2, dx: null, dy: null, matched: 0, rms_px: null },
      ],
    };
    expect(residualViews(result)).toEqual([
      { pair: "left ↔ right", shift: "1.00 px (dx 0.80, dy 0.60)", stars: "34 stars", warn: true },
      { pair: "left ↔ top", shift: "0.11 px (dx 0.10, dy -0.05)", stars: "1 star", warn: false },
      { pair: "right ↔ top", shift: "not measured", stars: "no shared stars", warn: false },
    ]);
  });

  it("names the output after the first panel, the panel count and the time, so no run overwrites another", () => {
    expect(mosaicOutputName("C:/m42/panels/m42 p1 Ha.fits", 4, new Date(2026, 9, 31, 9, 5, 7, 42))).toBe("m42_p1_Ha_mosaic4_20261031-090507-042");
  });

  it("shows the mosaic as a stacking result on its own grid", () => {
    const run = { fits_path: "C:/out/m42_mosaic3.fits", previewUrl: "asset://m42_mosaic3.png", dimensions: [9000, 3200], planes: 1 } as MosaicResult;
    expect(mosaicViewOutput(run, 3)).toEqual({
      output: { fitsPath: "C:/out/m42_mosaic3.fits", previewUrl: "asset://m42_mosaic3.png", dimensions: [9000, 3200], kind: "stacking", inputPath: "C:/out/m42_mosaic3.fits", planes: 1 },
      label: "Mosaic · 3 panels",
    });
  });
});

describe("panel filters", () => {
  const files = [lightFile("a_Ha.fits", "Ha"), lightFile("b_Ha.fits", "Ha"), lightFile("c_OIII.fits", "OIII"), lightFile("d.fits", null)];
  const [a, b, c, d] = files.map((f) => f.path);

  it("groups the picked panels by filter and asks for one filter per mosaic", () => {
    const groups = panelFilterGroups([a, b, c, d], files);
    expect(groups).toEqual([{ lane: "Ha", paths: [a, b] }, { lane: "OIII", paths: [c] }, { lane: null, paths: [d] }]);
    expect(mixedFilterLine(groups)).toBe("The picked panels hold 2 filters (Ha 2, OIII 1): a mosaic joins panels of one filter, so build one mosaic per filter.");
    expect(mixedFilterLine(panelFilterGroups([a, b, d], files))).toBeNull();
  });

  it("labels a mosaic with the filter all its panels share, else Mono", () => {
    expect(mosaicChannelOf([a, b], files)).toBe("Ha");
    expect(mosaicChannelOf([a, c], files)).toBe("Mono");
    expect(mosaicChannelOf([a, d], files)).toBe("Mono");
  });
});

describe("canvas picker and Compose", () => {
  const ha = sessionMosaic("m42_ha_mosaic3", "Ha");
  const oiii = sessionMosaic("m42_oiii_mosaic3", "OIII");

  it("offers a new canvas, the canvas of every mosaic of the session and of another FITS", () => {
    expect(canvasChoices([oiii, ha], null)).toEqual([
      { value: NEW_CANVAS, label: "New, from these panels" },
      { value: oiii.fitsPath, label: "Use the canvas of m42_oiii_mosaic3" },
      { value: ha.fitsPath, label: "Use the canvas of m42_ha_mosaic3" },
      { value: CHOOSE_CANVAS, label: "Use the canvas of another FITS..." },
    ]);
    expect(canvasChoices([oiii, ha], "D:/archive/ha_mosaic.fits")).toContainEqual({ value: "D:/archive/ha_mosaic.fits", label: "Use the canvas of ha_mosaic.fits" });
    expect(canvasChoices([oiii, ha], ha.fitsPath).filter((choice) => choice.value === ha.fitsPath)).toHaveLength(1);
  });

  it("hands Compose the newest mono mosaic of each channel, which binsFromMasters puts in its filter's bin", () => {
    const olderHa = sessionMosaic("m42_ha_mosaic2", "Ha");
    const rgb = sessionMosaic("m42_rgb_mosaic4", "Mono", 3);
    const plan = composePlan([oiii, ha, olderHa, rgb]);
    expect(plan.masters).toEqual([{ label: "Ha", fits_path: ha.fitsPath }, { label: "OIII", fits_path: oiii.fitsPath }]);
    expect(plan.notes).toEqual([
      "Compose takes the newest mosaic of each channel: 1 older mosaic is left out.",
      "RGB mosaics stay out of Compose: m42_rgb_mosaic4.",
    ]);
    const bins = binsFromMasters(plan.masters, DEFAULT_BINS);
    expect(Object.fromEntries(bins.filter((bin) => bin.files.length > 0).map((bin) => [bin.id, bin.files]))).toEqual({ ha: [ha.fitsPath], oiii: [oiii.fitsPath] });
    expect(composePlan([])).toEqual({ masters: [], notes: [] });
  });
});
