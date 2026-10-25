import { describe, it, expect, vi } from "vitest";
import {
  GAIA_REFINE_CANCELLED_NOTE,
  GAIA_REFINE_CONSENT,
  NO_SOLVE_YET,
  NO_WCS_TO_REFINE,
  appliedRefinement,
  gaiaRefineBlocker,
  gaiaRefineFields,
  gaiaRefineHeading,
  gaiaRefineProgressText,
  parseEpochYear,
  refineSolveCards,
  wcsRevision,
  NO_WCS_FILE,
  SOLVED_WCS_LABEL,
  WCS_ALREADY_WRITTEN,
  rememberSolvedWcs,
  shouldResetSolve,
  solvedWcsFor,
  solvedWcsSummary,
  solvedWcsVersion,
  creditedSolveTier,
  spccSolvedWcsPath,
  spccWcsSource,
  subscribeSolvedWcs,
  wcsRewriteBlocker,
  wcsWriteBlocker,
} from "../solvedWcs";
import { INITIAL_STATE, type WizardState } from "../wizard";
import type { WriteSolvedWcsResult } from "../../shared/types/astrometry";
import type { GaiaRefineResult } from "../../services/astrometry";

function written(overrides: Partial<WriteSolvedWcsResult> = {}): WriteSolvedWcsResult {
  return {
    fits_path: "C:\\Users\\me\\AppData\\Roaming\\astroburst\\output\\wcs_nowcs_673_wcs.fits",
    png_path: "C:\\Users\\me\\AppData\\Roaming\\astroburst\\output\\wcs_nowcs_673_wcs.png",
    dimensions: [1600, 1600],
    center_ra: 83.822,
    center_dec: -5.391,
    pixel_scale_arcsec: 1.99876,
    sip_present: true,
    elapsed_ms: 42,
    ...overrides,
  };
}

describe("wcsWriteBlocker", () => {
  it("asks for a solve when there is no result", () => {
    expect(wcsWriteBlocker(null)).toBe(NO_SOLVE_YET);
    expect(NO_SOLVE_YET).toBe("Plate solve the image first.");
  });

  it("names the missing WCS file when the solve carries no cards", () => {
    expect(wcsWriteBlocker({})).toBe(NO_WCS_FILE);
    expect(wcsWriteBlocker({ wcs_cards: [] })).toBe(NO_WCS_FILE);
    expect(NO_WCS_FILE).toBe("astrometry.net returned no WCS file for this job; solve again to write the WCS.");
  });

  it("lets the write run with at least one card", () => {
    expect(wcsWriteBlocker({ wcs_cards: [["CTYPE1", "RA---TAN-SIP"]] })).toBeNull();
  });
});

describe("wcsRewriteBlocker", () => {
  it("blocks a second write once the image on screen is the file this panel wrote, which would only add a _wcs_wcs copy", () => {
    const path = written().fits_path;
    expect(wcsRewriteBlocker(path, path)).toBe(WCS_ALREADY_WRITTEN);
    expect(WCS_ALREADY_WRITTEN).toBe("The image on screen already carries this solved WCS.");
  });

  it("lets the write run before anything was written", () => {
    expect(wcsRewriteBlocker(null, "/a.fits")).toBeNull();
  });

  it("lets the write run while the written file is not the image on screen", () => {
    expect(wcsRewriteBlocker("/a_wcs.fits", "/a.fits")).toBeNull();
  });

  it("names no rewrite reason when there is no measured image and nothing was written", () => {
    expect(wcsRewriteBlocker(null, null)).toBeNull();
  });
});

describe("solvedWcsSummary", () => {
  it("names the written file, the centre, the scale and SIP for a Windows path", () => {
    expect(solvedWcsSummary(written())).toBe(
      "Wrote wcs_nowcs_673_wcs.fits: centre 83.82200, -5.39100 deg, 1.999\"/px, SIP",
    );
  });

  it("omits SIP when the solution has no distortion terms", () => {
    expect(solvedWcsSummary(written({ fits_path: "/tmp/out/m42_wcs.fits", sip_present: false, pixel_scale_arcsec: 0.4 }))).toBe(
      "Wrote m42_wcs.fits: centre 83.82200, -5.39100 deg, 0.4000\"/px",
    );
  });

  it("labels the published result", () => {
    expect(SOLVED_WCS_LABEL).toBe("Plate-solved WCS");
  });
});

describe("shouldResetSolve", () => {
  const solving = { regionKey: "/a.fits", solvePath: "/a.fits" };

  it("resets when the selected file changes", () => {
    expect(shouldResetSolve(solving, { regionKey: "/b.fits", solvePath: "/b.fits" }, null)).toBe(true);
  });

  it("keeps the solve when the measured image becomes the file this panel just wrote", () => {
    expect(shouldResetSolve(solving, { regionKey: "/a.fits", solvePath: "/a_wcs.fits" }, "/a_wcs.fits")).toBe(false);
  });

  it("resets when another step replaces the measured image, whose grid may differ from the solved one", () => {
    expect(shouldResetSolve(solving, { regionKey: "/a.fits", solvePath: "/a_bg_corrected.fits" }, null)).toBe(true);
  });

  it("keeps the solve for identical identities", () => {
    expect(shouldResetSolve(solving, { ...solving }, null)).toBe(false);
  });

  it("resets on a file change even when the new measured path is the written one", () => {
    expect(shouldResetSolve(solving, { regionKey: "/b.fits", solvePath: "/a_wcs.fits" }, "/a_wcs.fits")).toBe(true);
  });
});

describe("solved WCS store", () => {
  it("finds the WCS written for the solved input, whatever the slashes or case of the path", () => {
    rememberSolvedWcs("C:\\data\\R_001.fits", "C:/out/R_001_wcs.fits");
    expect(solvedWcsFor("c:/data/r_001.fits")).toBe("C:/out/R_001_wcs.fits");
    expect(solvedWcsFor("C:/data/R_002.fits")).toBeNull();
  });

  it("keeps the latest WCS written for a path", () => {
    rememberSolvedWcs("/store/a.fits", "/out/a_wcs.fits");
    rememberSolvedWcs("/store/a.fits", "/out/a_wcs_2.fits");
    expect(solvedWcsFor("/store/a.fits")).toBe("/out/a_wcs_2.fits");
  });

  it("tells its subscribers when a WCS is written, so an open SPCC panel picks it up", () => {
    const listener = vi.fn();
    const unsubscribe = subscribeSolvedWcs(listener);
    const before = solvedWcsVersion();
    rememberSolvedWcs("/store/c.fits", "/out/c_wcs.fits");
    expect(listener).toHaveBeenCalledTimes(1);
    expect(solvedWcsVersion()).toBe(before + 1);
    unsubscribe();
    rememberSolvedWcs("/store/c.fits", "/out/c_wcs_2.fits");
    expect(listener).toHaveBeenCalledTimes(1);
    expect(solvedWcsVersion()).toBe(before + 2);
  });
});

function rgbState(overrides: Partial<WizardState> = {}, files: Record<string, string[]> = {}): WizardState {
  const assigned: Record<string, string[]> = { r: ["/grid/r.fits"], g: ["/grid/g.fits"], b: ["/grid/b.fits"], ...files };
  return {
    ...INITIAL_STATE,
    compositeReady: true,
    bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: assigned[b.id] ?? [] })),
    ...overrides,
  };
}

const ALIGNED = { r: "__wizard_aligned_r", g: "__wizard_aligned_g", b: "__wizard_aligned_b" };

describe("spccWcsSource", () => {
  it("uses the single R file as the grid of the plate-solved WCS", () => {
    expect(spccWcsSource(rgbState())).toEqual({ gridPath: "/grid/r.fits", reason: null });
  });

  it("uses the first effective frame of a stacked R bin, on whose grid the stack sits", () => {
    const state = rgbState(
      { stackedPaths: { r: "/out/stack_r.fits" }, excludedFiles: { r: ["/grid/r1.fits"] } },
      { r: ["/grid/r1.fits", "/grid/r2.fits", "/grid/r3.fits"] },
    );
    expect(spccWcsSource(state)).toEqual({ gridPath: "/grid/r2.fits", reason: null });
  });

  it("refuses a solved WCS when Align warped R onto another bin", () => {
    rememberSolvedWcs("/grid/r_solved.fits", "/out/r_solved_wcs.fits");
    expect(spccWcsSource(rgbState({ alignedPaths: ALIGNED, alignRefBinId: "g" }, { r: ["/grid/r_solved.fits"] }))).toEqual({
      gridPath: null,
      reason: "SPCC can use a plate-solved WCS only on the R channel's own grid; R was aligned onto G. Make R the alignment reference and run Align again.",
    });
  });

  it("also refuses when the WCS was solved on the stacked R master", () => {
    rememberSolvedWcs("/out/stack_r_solved.fits", "/out/stack_r_solved_wcs.fits");
    const state = rgbState(
      { alignedPaths: ALIGNED, alignRefBinId: "g", stackedPaths: { r: "/out/stack_r_solved.fits" } },
      { r: ["/grid/r_unsolved_1.fits", "/grid/r_unsolved_2.fits"] },
    );
    expect(spccWcsSource(state).reason).toMatch(/^SPCC can use a plate-solved WCS only on the R channel's own grid/);
  });

  it("gives no advice after Align when R was never plate-solved, since SPCC reads R's own header", () => {
    expect(spccWcsSource(rgbState({ alignedPaths: ALIGNED, alignRefBinId: "g" }, { r: ["/grid/r_never_solved.fits"] }))).toEqual({
      gridPath: null,
      reason: null,
    });
  });

  it("keeps R's own grid when R was the alignment reference", () => {
    expect(spccWcsSource(rgbState({ alignedPaths: ALIGNED, alignRefBinId: "r" }))).toEqual({ gridPath: "/grid/r.fits", reason: null });
    expect(spccWcsSource(rgbState({ alignedPaths: ALIGNED, alignRefBinId: null }))).toEqual({ gridPath: "/grid/r.fits", reason: null });
  });

  it("has no grid without an R channel", () => {
    expect(spccWcsSource(rgbState({}, { r: [] }))).toEqual({ gridPath: null, reason: null });
  });
});

describe("spccSolvedWcsPath", () => {
  it("finds the WCS solved on the R grid file", () => {
    rememberSolvedWcs("/spcc/r.fits", "/out/r_wcs.fits");
    expect(spccSolvedWcsPath(rgbState({}, { r: ["/spcc/r.fits"] }))).toBe("/out/r_wcs.fits");
  });

  it("finds the WCS solved on the stacked R master, which sits on the same grid", () => {
    rememberSolvedWcs("/spcc/stack_r.fits", "/out/stack_r_wcs.fits");
    const state = rgbState({ stackedPaths: { r: "/spcc/stack_r.fits" } }, { r: ["/spcc/r_a.fits", "/spcc/r_b.fits"] });
    expect(spccSolvedWcsPath(state)).toBe("/out/stack_r_wcs.fits");
  });

  it("prefers the stacked R master's WCS, which also fits a drizzled master, over its first frame's", () => {
    rememberSolvedWcs("/spcc/r_first.fits", "/out/r_first_wcs.fits");
    rememberSolvedWcs("/spcc/stack_r_drizzle.fits", "/out/stack_r_drizzle_wcs.fits");
    const state = rgbState({ stackedPaths: { r: "/spcc/stack_r_drizzle.fits" } }, { r: ["/spcc/r_first.fits", "/spcc/r_second.fits"] });
    expect(spccSolvedWcsPath(state)).toBe("/out/stack_r_drizzle_wcs.fits");
  });

  it("gives no WCS when R was aligned onto another bin, even if R was solved", () => {
    rememberSolvedWcs("/spcc/r_aligned.fits", "/out/r_aligned_wcs.fits");
    const state = rgbState({ alignedPaths: ALIGNED, alignRefBinId: "b" }, { r: ["/spcc/r_aligned.fits"] });
    expect(spccSolvedWcsPath(state)).toBeNull();
  });

  it("gives no WCS when nothing was solved for R", () => {
    expect(spccSolvedWcsPath(rgbState({}, { r: ["/spcc/never_solved.fits"] }))).toBeNull();
  });
});

describe("WCS revision", () => {
  it("starts at zero and counts each write of a path, whatever its slashes or case", () => {
    expect(wcsRevision("C:/out/rev_a_wcs.fits")).toBe(0);
    rememberSolvedWcs("C:\\in\\rev_a.fits", "C:\\out\\REV_A_wcs.fits");
    rememberSolvedWcs("c:/in/rev_a.fits", "c:/out/rev_a_wcs.fits");
    expect(wcsRevision("C:/out/rev_a_wcs.fits")).toBe(2);
    expect(wcsRevision("C:/in/rev_a.fits")).toBe(2);
    expect(wcsRevision("C:/out/rev_b.fits")).toBe(0);
    expect(wcsRevision(null)).toBe(0);
  });

  it("bumps the written copy and the solved input when a WCS is written, with one notification", () => {
    const listener = vi.fn();
    const unsubscribe = subscribeSolvedWcs(listener);
    const input = wcsRevision("/store/rev_d.fits");
    const copy = wcsRevision("/out/rev_d_wcs.fits");
    rememberSolvedWcs("/store/rev_d.fits", "/out/rev_d_wcs.fits");
    expect(wcsRevision("/store/rev_d.fits")).toBe(input + 1);
    expect(wcsRevision("/out/rev_d_wcs.fits")).toBe(copy + 1);
    expect(listener).toHaveBeenCalledTimes(1);
    unsubscribe();
  });
});

const FGS_TIE: GaiaRefineResult = {
  decision: "applied",
  reason: null,
  wcs_cards: [
    ["CRVAL1", "80.6575"],
    ["CRVAL2", "-69.5173"],
  ],
  history: ["Gaia DR3 tie: rshift"],
  model: "rshift",
  wcs_kind: "header",
  n_catalog: 2378,
  n_detected: 500,
  n_matched: 104,
  n_used: 97,
  rms_before_mas: 1766.2,
  rms_after_mas: 14.9,
  shift_east_mas: 1029.4,
  shift_north_mas: 1438.5,
  shift_err_east_mas: 2.1,
  shift_err_north_mas: 2.3,
  rotation_deg: 0.00125,
  rotation_err_deg: 0.0004,
  scale_ppm: null,
  scale_err_ppm: null,
  epoch_year: 2022.8548,
  epoch_source: "header",
  catalog: "Gaia DR3 (I/355/gaiadr3) via VizieR, CDS",
  catalog_source: "network",
  attribution: "Gaia DR3, ESA/Gaia/DPAC, via VizieR",
  warnings: [],
};

const REFUSED: GaiaRefineResult = {
  ...FGS_TIE,
  decision: "refused",
  reason: "1 < 15 used pairs for rshift",
  wcs_cards: [],
  history: [],
  n_catalog: 1,
  n_matched: 1,
  n_used: 0,
  rms_before_mas: null,
  rms_after_mas: null,
  shift_east_mas: null,
  shift_north_mas: null,
  shift_err_east_mas: null,
  shift_err_north_mas: null,
  rotation_deg: null,
  rotation_err_deg: null,
  epoch_year: null,
  epoch_source: "none",
  catalog_source: "disk",
};

describe("Gaia DR3 refinement text", () => {
  it("says before the click that a star list is fetched online and the image stays local", () => {
    expect(GAIA_REFINE_CONSENT).toContain("VizieR");
    expect(GAIA_REFINE_CONSENT).toContain("star list");
    expect(GAIA_REFINE_CONSENT).toContain("the image stays on this computer");
    expect(GAIA_REFINE_CONSENT).toContain("CC BY-NC 3.0 IGO");
    expect(GAIA_REFINE_CONSENT).not.toContain(String.fromCharCode(0x2014));
  });

  it("needs a WCS to refine: a solve or the header's own", () => {
    expect(gaiaRefineBlocker(false)).toBe(NO_WCS_TO_REFINE);
    expect(gaiaRefineBlocker(true)).toBeNull();
  });

  it("hands Write WCS only an applied refinement that carries cards", () => {
    expect(appliedRefinement(FGS_TIE)).toBe(FGS_TIE);
    expect(appliedRefinement(REFUSED)).toBeNull();
    expect(appliedRefinement({ ...FGS_TIE, decision: "no_significant_correction" })).toBeNull();
    expect(appliedRefinement({ ...FGS_TIE, wcs_cards: [] })).toBeNull();
    expect(appliedRefinement(null)).toBeNull();
    expect(wcsWriteBlocker(appliedRefinement(FGS_TIE))).toBeNull();
  });

  it("heads the block with the decision", () => {
    expect(gaiaRefineHeading(FGS_TIE)).toBe("Refined with Gaia DR3");
    expect(gaiaRefineHeading({ ...FGS_TIE, decision: "no_significant_correction" })).toBe("No significant correction");
    expect(gaiaRefineHeading(REFUSED)).toBe("Not refined");
  });

  it("prints matched, RMS in mas, shift and rotation with their 1 sigma, the model, the epoch and the catalogue", () => {
    const fields = Object.fromEntries(gaiaRefineFields(FGS_TIE).map((f) => [f.key, f.value]));
    expect(fields).toEqual({
      "refine-matched": "97 used of 104 matched (2378 Gaia DR3 stars)",
      "refine-rms": "1766.2 mas before, 14.9 mas after",
      "refine-shift": "+1029.4 mas E, +1438.5 mas N (1σ 2.1, 2.3)",
      "refine-rotation": "+0.00125° (+4.50\") (1σ 0.00040°)",
      "refine-model": "Shift + rotation (rshift)",
      "refine-epoch": "2022.855 (header)",
      "refine-wcs": "Header WCS",
      "refine-catalog": "Gaia DR3 (I/355/gaiadr3) via VizieR, CDS (network)",
    });
  });

  it("prefers the hold-out RMS, which keeps the offset the tie removes", () => {
    const fields = gaiaRefineFields({ ...FGS_TIE, holdout_rms_before_mas: 1772.0, holdout_rms_after_mas: 34.58 });
    expect(fields.find((f) => f.key === "refine-rms")?.value).toBe("1772.0 mas before, 34.6 mas after (hold-out)");
  });

  it("adds the scale for the models that fit it", () => {
    const fields = gaiaRefineFields({ ...FGS_TIE, model: "rscale", scale_ppm: 12.34, scale_err_ppm: 5 });
    expect(fields.find((f) => f.key === "refine-scale")?.value).toBe("+12.3 ppm (1σ 5.0)");
    expect(fields.find((f) => f.key === "refine-model")?.value).toBe("Shift + rotation + scale (rscale)");
  });

  it("keeps only what a refusal measured", () => {
    const fields = Object.fromEntries(gaiaRefineFields(REFUSED).map((f) => [f.key, f.value]));
    expect(fields).toEqual({
      "refine-matched": "0 used of 1 matched (1 Gaia DR3 stars)",
      "refine-model": "Shift + rotation (rshift)",
      "refine-epoch": "J2016.0 (no date in the header)",
      "refine-wcs": "Header WCS",
      "refine-catalog": "Gaia DR3 (I/355/gaiadr3) via VizieR, CDS (disk)",
    });
  });

  it("names how the WCS was updated and the epoch term of an undated frame", () => {
    const fields = gaiaRefineFields({
      ...FGS_TIE,
      representation: "exact header update (CRVAL and CD)",
      epoch_year: null,
      epoch_source: "none",
      epoch_systematic_mas: 67.4,
    });
    const byKey = Object.fromEntries(fields.map((f) => [f.key, f.value]));
    expect(byKey["refine-representation"]).toBe("exact header update (CRVAL and CD)");
    expect(byKey["refine-epoch-term"]).toBe("67.4 mas for an epoch uncertain by 10 yr");
    expect(fields.map((f) => f.key).indexOf("refine-epoch-term")).toBe(fields.map((f) => f.key).indexOf("refine-epoch") + 1);
  });

  it("names the WCS the tie started from", () => {
    const refined = (wcs_kind: string | null) => gaiaRefineFields({ ...FGS_TIE, wcs_kind }).find((f) => f.key === "refine-wcs");
    expect(refined("solve")).toEqual({ key: "refine-wcs", label: "Refined WCS", value: "Plate solve" });
    expect(refined("header")?.value).toBe("Header WCS");
    expect(refined("gwcs")?.value).toBe("gWCS (JWST distortion model)");
    expect(refined(null)).toBeUndefined();
  });

  it("names a typed epoch", () => {
    const epoch = gaiaRefineFields({ ...FGS_TIE, epoch_year: 1990, epoch_source: "typed" }).find((f) => f.key === "refine-epoch");
    expect(epoch?.value).toBe("1990.000 (typed)");
  });
});

describe("parseEpochYear", () => {
  it("leaves the epoch to the header when the field is empty", () => {
    expect(parseEpochYear("")).toEqual({ value: null, error: null });
    expect(parseEpochYear("  ")).toEqual({ value: null, error: null });
  });

  it("reads a decimal year with a point or a comma", () => {
    expect(parseEpochYear("1990.5")).toEqual({ value: 1990.5, error: null });
    expect(parseEpochYear("1990,5")).toEqual({ value: 1990.5, error: null });
  });

  it("refuses text and years outside 1900 to 2100", () => {
    const error = "Epoch: type a decimal year between 1900 and 2100, or leave it empty to use the header date";
    expect(parseEpochYear("abc")).toEqual({ value: null, error });
    expect(parseEpochYear("1850")).toEqual({ value: null, error });
    expect(parseEpochYear("2101")).toEqual({ value: null, error });
  });
});

describe("refineSolveCards", () => {
  const cards: [string, string][] = [["CRVAL1", "274.7"]];

  it("hands the solve's cards to the tie only on the path that was solved", () => {
    expect(refineSolveCards({ solveCards: cards, solvedPath: "C:\\data\\M16.fits", path: "c:/data/m16.fits", fileWcsKind: "header" })).toBe(cards);
    expect(refineSolveCards({ solveCards: cards, solvedPath: "C:/data/m16.fits", path: "C:/data/m16.fits", fileWcsKind: null })).toBe(cards);
  });

  it("refines the file's own WCS on the written copy, where the header already holds the tie", () => {
    expect(refineSolveCards({ solveCards: cards, solvedPath: "C:/data/m16.fits", path: "C:/out/m16_wcs.fits", fileWcsKind: "header" })).toBeNull();
  });

  it("keeps a gWCS and its distortion model instead of the solve", () => {
    expect(refineSolveCards({ solveCards: cards, solvedPath: "C:/data/fgs_cal.fits", path: "C:/data/fgs_cal.fits", fileWcsKind: "gwcs" })).toBeNull();
  });

  it("sends nothing without a solve", () => {
    expect(refineSolveCards({ solveCards: [], solvedPath: "C:/data/m16.fits", path: "C:/data/m16.fits", fileWcsKind: "header" })).toBeNull();
    expect(refineSolveCards({ solveCards: cards, solvedPath: null, path: "C:/data/m16.fits", fileWcsKind: "header" })).toBeNull();
  });
});

describe("Gaia DR3 refinement progress", () => {
  it("words every refine stage the command reports", () => {
    const cases: [Parameters<typeof gaiaRefineProgressText>[0], string | null][] = [
      [{ stage: "load", current: 0, total: 0 }, "Reading the image and its WCS"],
      [{ stage: "detect", current: 0, total: 0 }, "Detecting stars; the Gaia DR3 star list is fetched at the same time"],
      [{ stage: "measure", current: 812, total: 2000 }, "Measuring stars: 812 of 2000"],
      [{ stage: "measure", current: 0, total: 0 }, "Measuring stars"],
      [{ stage: "vizier", current: 41, total: 150 }, "Waiting for VizieR: 0:41 / 2:30"],
      [{ stage: "fit", current: 0, total: 0 }, "Fitting the tie to Gaia DR3"],
      [{ stage: "represent", current: 0, total: 0 }, "Computing the refined WCS cards"],
      [{ stage: "complete", current: 1, total: 1 }, null],
      [null, null],
    ];
    for (const [input, output] of cases) expect(gaiaRefineProgressText(input), JSON.stringify(input)).toBe(output);
    expect(GAIA_REFINE_CANCELLED_NOTE).toBe("Gaia DR3 refinement cancelled; the WCS is unchanged");
  });

  it("passes the solve's tier for a credited solve, and only when the written WCS derives from the solve", () => {
    const credited = { catalog: "tier1-gaia-siril", credit: { text: "Gaia DR3 credit", citation: "Gaia DR3 citation" } };
    expect(creditedSolveTier(credited, null)).toBe("tier1-gaia-siril");
    expect(creditedSolveTier(credited, { ...FGS_TIE, wcs_kind: "solve" })).toBe("tier1-gaia-siril");
    expect(creditedSolveTier(credited, { ...FGS_TIE, wcs_kind: "header" })).toBeNull();
    expect(creditedSolveTier({ catalog: "tier1-gaia-siril", credit: null }, null)).toBeNull();
    expect(creditedSolveTier({ catalog: "tier0-tycho2" }, null)).toBeNull();
    expect(creditedSolveTier(null, null)).toBeNull();
  });
});
