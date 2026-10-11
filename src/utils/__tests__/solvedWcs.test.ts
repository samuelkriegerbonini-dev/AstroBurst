import { describe, it, expect, vi } from "vitest";
import {
  NO_SOLVE_YET,
  NO_WCS_FILE,
  SOLVED_WCS_LABEL,
  WCS_ALREADY_WRITTEN,
  rememberSolvedWcs,
  shouldResetSolve,
  solvedWcsFor,
  solvedWcsSummary,
  solvedWcsVersion,
  spccSolvedWcsPath,
  spccWcsSource,
  subscribeSolvedWcs,
  wcsRewriteBlocker,
  wcsWriteBlocker,
} from "../solvedWcs";
import { INITIAL_STATE, type WizardState } from "../wizard";
import type { WriteSolvedWcsResult } from "../../shared/types/astrometry";

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
