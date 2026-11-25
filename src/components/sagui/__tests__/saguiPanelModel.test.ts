import { describe, expect, it } from "vitest";
import type { SaguiInputInfo, SaguiPlaneDqFlags, SaguiResult } from "../../../shared/types/sagui";
import { DQ_PRESETS, dqAdvice, dqPlaneRows, psfTargetWarning, recommendedDqMask, regionSedCsv, resultSummary, runtimeAdvice, runtimeIssuesText, wardVerdict } from "../saguiPanelModel";

const INPUT: SaguiInputInfo = { width: 40, height: 30, bands: [{ filter: "F115W", pivot_um: 1.154 }, { filter: "F150W", pivot_um: 1.501 }, { filter: "F200W", pivot_um: 1.989 }], unit: "Jy/pixel", has_err: true, has_dq: true, dq_flags: [], estimated_memory_bytes: 1, warnings: [] };
const MIXED: SaguiPlaneDqFlags[] = [
  { filter: "F814W", table: "hst", flagged_finite: { COSMIC_RAY: 12, REED_SOLOMON: 2, HOT: 3 } },
  { filter: "F150W", table: "jwst", flagged_finite: { JUMP_DET: 7, DO_NOT_USE: 5 } },
  { filter: "F444W", table: "jwst", flagged_finite: {} },
];

describe("SAGUI PSF target", () => {
  it("warns when the target names no band of the cube (PREPUI-04)", () => {
    expect(psfTargetWarning(INPUT, "F444W")).toBe("F444W is not a band of this cube (F115W, F150W, F200W). The PSF checks cannot use it; name a PSF outside the cube only when every band was matched to it outside Cube Prep.");
    expect(psfTargetWarning(INPUT, " f150w ")).toBeNull();
    expect(psfTargetWarning(INPUT, "")).toBeNull();
  });
});

describe("SAGUI DQ policy (PREPUI-07)", () => {
  it("recommends the union of the masks Cube Prep uses for each DQ table", () => {
    expect(recommendedDqMask(MIXED)).toBe(16383);
    expect(recommendedDqMask(MIXED.slice(1))).toBe(1);
    expect(recommendedDqMask([{ filter: "X", table: "unknown (raw bits)", flagged_finite: {} }])).toBe(1);
    expect(recommendedDqMask([])).toBeNull();
    expect(recommendedDqMask(undefined)).toBeNull();
  });

  it("lists the flags of each band in bit order and marks the ones the mask excludes", () => {
    expect(dqPlaneRows(MIXED, 1)).toEqual([
      { filter: "F814W", table: "HST", flags: [{ name: "REED_SOLOMON", count: 2, excluded: true }, { name: "HOT", count: 3, excluded: false }, { name: "COSMIC_RAY", count: 12, excluded: false }] },
      { filter: "F150W", table: "JWST", flags: [{ name: "DO_NOT_USE", count: 5, excluded: true }, { name: "JUMP_DET", count: 7, excluded: false }] },
      { filter: "F444W", table: "JWST", flags: [] },
    ]);
    expect(dqPlaneRows(MIXED, 16383)[0].flags.every((flag) => flag.excluded)).toBe(true);
    expect(dqPlaneRows([{ filter: "X", table: "unknown (raw bits)", flagged_finite: { BIT31: 1, BIT0: 4 } }], 4294967295)).toEqual([{ filter: "X", table: "unknown", flags: [{ name: "BIT0", count: 4, excluded: true }, { name: "BIT31", count: 1, excluded: true }] }]);
  });

  it("says what one mask does to planes with different DQ tables and when it leaves out flags Cube Prep excludes", () => {
    expect(dqAdvice(MIXED, 1)).toEqual({ recommended: 16383, text: "The planes use different DQ tables (HST: F814W; JWST: F150W, F444W) and one mask applies to all of them. Cube Prep excludes 0x3FFF (REED_SOLOMON to COSMIC_RAY_SINGLE) on HST planes and DO_NOT_USE (bit 0) on the other planes; their union is 16383." });
    expect(dqAdvice(MIXED, 16383)).toEqual({ recommended: 16383, text: "The planes use different DQ tables (HST: F814W; JWST: F150W, F444W) and one mask applies to all of them. 16383 is the union of the masks Cube Prep uses, so on the JWST and Roman planes it also excludes SATURATED to LOW_QE (bits 2 to 8192)." });
    expect(dqAdvice([MIXED[0]], 1)).toEqual({ recommended: 16383, text: "Cube Prep excludes 0x3FFF (REED_SOLOMON to COSMIC_RAY_SINGLE) on HST planes; the same flags here need 16383." });
    expect(dqAdvice(MIXED.slice(1), 1)).toBeNull();
    expect(dqAdvice(MIXED.slice(1), 4294967295)).toBeNull();
    expect(dqAdvice([], 1)).toBeNull();
    expect(DQ_PRESETS.map((preset) => preset.bits)).toEqual([1, 16383, 4294967295, 0]);
  });
});

describe("SAGUI runtime advice (PREPUI-13)", () => {
  it("gives the action that fixes each known runtime issue", () => {
    expect(runtimeAdvice("Expected R 4.6.2; found 4.5.1")).toBe("install R 4.6.2 alongside your current R (the Windows installer is in the CRAN archive) and choose its Rscript.");
    expect(runtimeAdvice("Expected R 4.6.2 before installing packages; found 4.6.3")).toBe("install R 4.6.2 alongside your current R (the Windows installer is in the CRAN archive) and choose its Rscript.");
    expect(runtimeAdvice("Expected the pinned SAGUI commit")).toBe("use Install / repair SAGUI library to build the pinned commit into a new private library.");
    expect(runtimeAdvice("SAGUI is missing or a dependency could not be loaded")).toBe("use Install / repair SAGUI library to install SAGUI and its dependencies into a new private library.");
    expect(runtimeAdvice("SAGUI must be in the private library")).toBe("choose the private library that holds SAGUI, or use Install / repair SAGUI library.");
    expect(runtimeAdvice("CPU profile excludes torch")).toBe("the library holds torch, which the CPU profile leaves out; use Install / repair SAGUI library to make a clean private library.");
    expect(runtimeAdvice("Rscript not installed")).toBeNull();
    expect(runtimeIssuesText(["Rscript not installed", "Expected the pinned SAGUI commit"])).toBe("Rscript not installed Expected the pinned SAGUI commit: use Install / repair SAGUI library to build the pinned commit into a new private library.");
  });
});

describe("SAGUI regions", () => {
  it("says before the run whether exact Ward can take the footprint (proposal 18)", () => {
    expect(wardVerdict({ ...INPUT, width: 200, height: 300 }, "ward")).toEqual({ tone: "ok", text: "Exact Ward: the full footprint of 60,000 pixels is within its 65,536 pixel limit." });
    expect(wardVerdict({ ...INPUT, width: 500, height: 500 }, "ward")).toEqual({ tone: "warn", text: "Exact Ward refuses more than 65,536 support pixels and the full footprint has 250,000. The starlet and DQ masks may bring the support under the limit; otherwise crop the field in Cube Prep or choose Sparse Ward." });
    expect(wardVerdict({ ...INPUT, width: 500, height: 500 }, "sparse")).toBeNull();
  });

  it("summarises a result and writes the SED of one region as CSV (proposal 22)", () => {
    const result = { unit: "Jy", warnings: ["a", "b", "c"], regions: [{ region: 1, n_pixels: 300, bands: [] }, { region: 2, n_pixels: 41, bands: [] }] } as unknown as SaguiResult;
    expect(resultSummary(result)).toBe("2 regions, 341 support pixels, 3 warnings");
    expect(resultSummary({ ...result, warnings: [], regions: [result.regions[0]] })).toBe("1 region, 300 support pixels, no warnings");
    const region = { region: 3, n_pixels: 120, bands: [{ filter: "F200W", pivot_um: 1.989, flux: 0.5, error: 0.1, error_correlated: 0.2 }, { filter: "F444W", pivot_um: 4.404, flux: -0.25, error: null }] };
    expect(regionSedCsv(region, "Jy")).toBe("# SAGUI region 3, 120 pixels, flux unit Jy\nfilter,pivot_um,flux,error,error_correlated\nF200W,1.989,0.5,0.1,0.2\nF444W,4.404,-0.25,,\n");
  });
});
