import { describe, it, expect } from "vitest";
import {
  ACCEPTANCE,
  QSLOPE_ACCURACY_NOTE,
  ampFaintRows,
  compareVerdict,
  draftFromParams,
  fittedLinePoints,
  formatTableCell,
  groupTimeSummary,
  irs2Badge,
  paramsFromDraft,
  rampIdentity,
  rampShape,
  formatCompareBin,
  formatConfusion,
  formatGroupTime,
  formatRampFrameLabel,
  formatShiftCheck,
  frameForIntegration,
  frameToPosition,
  histogramSeries,
  integrationOptions,
  isRampProductName,
  parseUncalRows,
  positionToFrame,
  qslopeOutputName,
  quickFlaggedShare,
  quickSlopeUnavailableReason,
  rampInspectorSeries,
  rampKeyHint,
  rampOpenPlan,
  rampTimeAxis,
  siblingRateName,
  tgroupSourceNote,
  validateQuickSlopeParams,
  compareVerdictTitle,
  cubePanelGates,
  framePublishers,
  histogramSummary,
  irs2ScienceRowCount,
  rampCompareKey,
  rampDisplayMapping,
  rampDisplayNote,
  rampFrameFromPlotChannel,
  rampFrameMarker,
  rampInspectLabel,
  rampPixelTarget,
  scienceRowOfUncalRow,
  uncalRowOfScienceRow,
} from "../rampLabels";
import { createFramePublishGate } from "../cubeNavigation";
import {
  DEFAULT_QUICK_SLOPE_PARAMS,
  type AmpBins,
  type CompareBin,
  type FlagConfusion,
  type RampInfo,
  type RampPixelFit,
  type RateComparison,
} from "../../shared/types/ramp";

const TGROUP = 14.589;

const owner: RampInfo = {
  kind: "jwst_groups",
  nints: 1,
  ngroups: 10,
  nframes: 1,
  groupgap: 0,
  tframe_s: 14.58889,
  tgroup_s: TGROUP,
  tgroup_source: "tgroup",
  group_times_s: Array.from({ length: 10 }, (_, g) => g * TGROUP),
  readpatt: "NRSIRS2RAPID",
  instrument: "NIRSPEC",
  detector: "NRS2",
  exp_type: "NRS_IFU",
  datamodl: "Level1bModel",
  irs2: { nrs_norm: 16, nrs_ref: 4, noutputs: 5, fast_axis: -2, slow_axis: -1 },
  frame_width: 2048,
  frame_height: 3200,
};

const roman: RampInfo = {
  ...owner,
  kind: "roman_resultants",
  ngroups: 8,
  tframe_s: 3.04,
  tgroup_s: null,
  tgroup_source: "none",
  group_times_s: null,
  readpatt: "MA_TABLE_1",
  instrument: "WFI",
  detector: "WFI01",
  exp_type: null,
  datamodl: null,
  irs2: null,
  frame_width: 4096,
  frame_height: 4096,
};

function bin(lo: number | null, hi: number | null, median_delta: number, median_rel: number | null, n = 100000): CompareBin {
  const rate = lo === null ? 0.01 : hi === null ? lo * 2 : (lo + hi) / 2;
  return {
    lo,
    hi,
    n,
    median_rate: rate,
    median_err: 0.06,
    median_delta,
    median_rel,
    p16_delta: median_delta - 0.05,
    p84_delta: median_delta + 0.05,
  };
}

function ampBins(amplifier: number, faint: number, low: number): AmpBins {
  return {
    amplifier,
    science_rows: [512 * amplifier, 512 * amplifier + 512],
    bins: [bin(null, 0.05, faint, null), bin(0.05, 0.3, low, null), bin(1, 3, 0.03, 0.02)],
  };
}

const NO_CONFUSION: FlagConfusion = { tp: 0, fp: 0, fn_: 0, recall: null, precision: null };

function comparison(overrides: Partial<RateComparison> = {}): RateComparison {
  return {
    qslope_path: "C:/out/jw01266005001_02103_00001_nrs2_qslope.fits",
    rate_path: "C:/fits/jw01266005001_02103_00001_nrs2_rate.fits",
    detector: "NRS2",
    good_pixels: 4_000_000,
    quick_flagged_in_good: 6400,
    bins: [
      bin(null, 0.05, 0.0079, null),
      bin(0.05, 0.3, 0.0074, null),
      bin(0.3, 1, 0.01, 0.018),
      bin(1, 3, 0.04, 0.0223),
      bin(3, 10, 0.1, 0.0173),
      bin(10, 30, 0.2, 0.0124),
      bin(30, null, 1.5, 0.05, 20),
    ],
    amp_bins: [ampBins(0, 0.0192, 0.0153), ampBins(1, -0.014, -0.0126), ampBins(2, 0.0097, 0.012), ampBins(3, 0.0064, -0.008)],
    zero_shift: { best_dy: 0, corr_at_zero: 0.9996, corr_best: 0.9996, second_dy: 1, second_corr: 0.945, passed: true },
    jump: NO_CONFUSION,
    saturated: NO_CONFUSION,
    saturated_rule: "all_groups (stcal 1.15.2)",
    saturated_any_group: NO_CONFUSION,
    rel_hist: { edges: [], counts: [] },
    delta_hist: { edges: [], counts: [] },
    science_rows: null,
    ratio_png_path: "C:/out/x_ratio.png",
    ratio_fits_path: "C:/out/x_ratio.fits",
    elapsed_ms: 1200,
    ...overrides,
  };
}

function pixelFit(overrides: Partial<RampPixelFit> = {}): RampPixelFit {
  return {
    x: 10,
    y: 700,
    integration: 0,
    group_times_s: [0, 10, 20, 30],
    raw: [1100, 1150, 1200, 1250],
    corrected: [100, 150, 200, 250],
    ref_offsets: [1000, 1000, 1000, 1000],
    band_scale_dn: 15,
    fit: { slope: 5, ngood: 4, dq: 0, noise: 0.02, n_usable: 4, first_saturated: null },
    flagged_diffs: [],
    amplifier: 0,
    science_row: 48,
    ...overrides,
  };
}

describe("frame position", () => {
  it("splits a flattened frame into a group and an integration and back", () => {
    expect(frameToPosition(13, 10)).toEqual({ group: 3, integration: 1 });
    expect(positionToFrame({ group: 3, integration: 1 }, 10)).toBe(13);
    expect(frameToPosition(0, 10)).toEqual({ group: 0, integration: 0 });
    for (let z = 0; z < 30; z++) expect(positionToFrame(frameToPosition(z, 10), 10)).toBe(z);
  });

  it("keeps the group when the integration changes", () => {
    expect(frameForIntegration(13, 2, 10)).toBe(23);
    expect(frameForIntegration(3, 0, 10)).toBe(3);
  });

  it("treats a missing group count as one group per frame", () => {
    expect(frameToPosition(4, 0)).toEqual({ group: 0, integration: 4 });
  });
});

describe("ramp labels", () => {
  it("names groups and integrations one-based", () => {
    expect(formatRampFrameLabel(2, owner)).toBe("group 3 of 10, integration 1 of 1");
    expect(formatRampFrameLabel(13, { ...owner, nints: 2 })).toBe("group 4 of 10, integration 2 of 2");
  });

  it("names Roman resultants without integrations", () => {
    expect(formatRampFrameLabel(2, roman)).toBe("resultant 3 of 8");
  });

  it("gives a key hint in the ramp vocabulary", () => {
    expect(rampKeyHint(owner)).toBe("ArrowLeft / ArrowRight step the group, Shift steps by 10; integration from the Ramp panel");
    expect(rampKeyHint(roman)).toBe("ArrowLeft / ArrowRight step the resultant, Shift steps by 10");
  });

  it("formats the group time from group_times_s and returns null without it", () => {
    expect(formatGroupTime(2, owner)).toBe("29.18 s");
    expect(formatGroupTime(12, { ...owner, nints: 2 })).toBe("29.18 s");
    expect(formatGroupTime(2, { ...owner, group_times_s: null })).toBeNull();
    expect(formatGroupTime(1, { ...roman, ngroups: 3, group_times_s: [3.04, 7.6, 16.72] })).toBe("7.60 s");
  });

  it("explains where the group time came from", () => {
    expect(tgroupSourceNote(owner)).toBe("TGROUP from header");
    expect(tgroupSourceNote({ ...owner, tgroup_source: "tframe_product" })).toBe("TFRAME x (NFRAMES + GROUPGAP)");
    expect(tgroupSourceNote({ ...owner, tgroup_s: null, tgroup_source: "none", group_times_s: null })).toBe(
      "group time unknown: slope disabled",
    );
    expect(tgroupSourceNote({ ...roman, group_times_s: [3.04, 7.6, 16.72] })).toBe("resultant times from read_pattern");
    expect(tgroupSourceNote(roman)).toBe("group time unknown: slope disabled");
  });

  it("lists one option per integration", () => {
    expect(integrationOptions({ ...owner, nints: 3 })).toEqual([
      { value: 0, label: "integration 1 of 3" },
      { value: 1, label: "integration 2 of 3" },
      { value: 2, label: "integration 3 of 3" },
    ]);
  });
});

describe("product names", () => {
  it("mirrors the Rust output name: _uncal stripped, integration named only above one integration", () => {
    expect(qslopeOutputName("jw01266005001_02103_00001_nrs1_uncal.fits", 1, 0)).toBe("jw01266005001_02103_00001_nrs1_qslope.fits");
    expect(qslopeOutputName("jw01266005001_02103_00001_nrs1_uncal.fits", 3, 1)).toBe(
      "jw01266005001_02103_00001_nrs1_int002_qslope.fits",
    );
    expect(qslopeOutputName("other.fits", 1, 0)).toBe("other_qslope.fits");
    expect(qslopeOutputName("C:\\data\\x_UNCAL.fits", 1, 0)).toBe("x_qslope.fits");
    expect(qslopeOutputName("C:/data/a_uncal.fits.gz", 1, 0)).toBe("a_qslope.fits");
    expect(qslopeOutputName("C:/data/a_uncal.fits#hdu=1", 1, 0)).toBe("a_qslope.fits");
  });

  it("names the sibling rate of an uncal", () => {
    expect(siblingRateName("C:/fits/jw01266005001_02103_00001_nrs1_uncal.fits")).toBe("jw01266005001_02103_00001_nrs1_rate.fits");
  });

  it("recognises a quick-slope product by its suffix", () => {
    expect(isRampProductName("jw_nrs1_qslope.fits")).toBe(true);
    expect(isRampProductName("JW_NRS1_QSLOPE.FITS")).toBe(true);
    expect(isRampProductName("jw_nrs1_rate.fits")).toBe(false);
    expect(isRampProductName("jw_nrs1_qslope.png")).toBe(false);
  });
});

describe("rampOpenPlan", () => {
  it("opens a FITS cube first, an ASDF multi-plane array as a ramp, and nothing for single planes or RGB", () => {
    expect(rampOpenPlan("C:/fits/x_uncal.fits", 10, false)).toBe("cube");
    expect(rampOpenPlan("C:/roman/r_l1.asdf#array=roman.data", 8, false)).toBe("asdf_ramp");
    expect(rampOpenPlan("C:/roman/R_L1.ASDF", 8, false)).toBe("asdf_ramp");
    expect(rampOpenPlan("C:/fits/x_cal.fits", 1, false)).toBe("none");
    expect(rampOpenPlan("C:/fits/rgb.fits", 3, true)).toBe("none");
  });
});

describe("quick slope availability", () => {
  it("allows a FITS ramp with a group time", () => {
    expect(quickSlopeUnavailableReason(owner, "fits")).toBeNull();
  });

  it("refuses ASDF and Roman ramps and ramps without a group time", () => {
    expect(quickSlopeUnavailableReason(owner, "asdf")).toMatch(/FITS/);
    expect(quickSlopeUnavailableReason({ ...roman, group_times_s: [1, 2, 3] }, "fits")).toMatch(/Roman/);
    expect(quickSlopeUnavailableReason({ ...owner, tgroup_s: null, tgroup_source: "none" }, "fits")).toBe(
      "group time unknown: slope disabled",
    );
  });
});

describe("validateQuickSlopeParams", () => {
  it("accepts the defaults and a whole-band window", () => {
    expect(validateQuickSlopeParams(DEFAULT_QUICK_SLOPE_PARAMS)).toEqual([]);
    expect(validateQuickSlopeParams({ ...DEFAULT_QUICK_SLOPE_PARAMS, ref_window_rows: null })).toEqual([]);
    expect(validateQuickSlopeParams({ ...DEFAULT_QUICK_SLOPE_PARAMS, ref_window_rows: 40, min_groups_ols: 2, jump_k: 1, scale_floor_dn: 0 })).toEqual([]);
    expect(validateQuickSlopeParams({ ...DEFAULT_QUICK_SLOPE_PARAMS, sat_dn: 65535 })).toEqual([]);
  });

  it("mirrors the Rust validate rules one message per field", () => {
    for (const sat_dn of [0, -1, 65536, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(validateQuickSlopeParams({ ...DEFAULT_QUICK_SLOPE_PARAMS, sat_dn })).toEqual([expect.stringMatching(/^saturation/)]);
    }
    expect(validateQuickSlopeParams({ ...DEFAULT_QUICK_SLOPE_PARAMS, jump_k: 0.5 })).toEqual([expect.stringMatching(/^jump k/)]);
    expect(validateQuickSlopeParams({ ...DEFAULT_QUICK_SLOPE_PARAMS, scale_floor_dn: -0.1 })).toEqual([expect.stringMatching(/^scale floor/)]);
    expect(validateQuickSlopeParams({ ...DEFAULT_QUICK_SLOPE_PARAMS, min_groups_ols: 1 })).toEqual([expect.stringMatching(/^min groups/)]);
    expect(validateQuickSlopeParams({ ...DEFAULT_QUICK_SLOPE_PARAMS, min_groups_ols: 2.5 })).toEqual([expect.stringMatching(/^min groups/)]);
    expect(validateQuickSlopeParams({ ...DEFAULT_QUICK_SLOPE_PARAMS, ref_window_rows: 30 })).toEqual([expect.stringMatching(/^reference window/)]);
    expect(validateQuickSlopeParams({ ...DEFAULT_QUICK_SLOPE_PARAMS, ref_window_rows: 100.5 })).toEqual([expect.stringMatching(/^reference window/)]);
    expect(validateQuickSlopeParams({ ...DEFAULT_QUICK_SLOPE_PARAMS, sat_dn: 0, jump_k: 0 })).toHaveLength(2);
  });
});

describe("ramp inspector", () => {
  it("plots against group times when they match the series and against the 0-based group otherwise", () => {
    expect(rampTimeAxis([0, 10, 20], 3)).toEqual({ t: [0, 10, 20], label: "time (s)" });
    expect(rampTimeAxis(null, 3)).toEqual({ t: [0, 1, 2], label: "group (0-based)" });
    expect(rampTimeAxis([0, 10], 3)).toEqual({ t: [0, 1, 2], label: "group (0-based)" });
  });

  it("returns the endpoints of the fitted line through the usable corrected groups", () => {
    expect(fittedLinePoints(pixelFit())).toEqual([
      { t: 0, value: 100 },
      { t: 30, value: 250 },
    ]);
    const saturated = pixelFit({ fit: { slope: 5, ngood: 3, dq: 2, noise: 0.03, n_usable: 3, first_saturated: 3 } });
    expect(fittedLinePoints(saturated)).toEqual([
      { t: 0, value: 100 },
      { t: 20, value: 200 },
    ]);
  });

  it("draws no line without a slope or with fewer than two usable groups", () => {
    expect(fittedLinePoints(pixelFit({ fit: { slope: null, ngood: 0, dq: 3, noise: null, n_usable: 0, first_saturated: 0 } }))).toEqual([]);
    expect(fittedLinePoints(pixelFit({ fit: { slope: 5, ngood: 1, dq: 3, noise: null, n_usable: 1, first_saturated: 1 } }))).toEqual([]);
  });

  it("marks the later group of each flagged difference and greys the groups past the usable ones", () => {
    const fit = pixelFit({
      raw: [1100, 1150, 1700, 65000],
      corrected: [100, 150, 700, 64000],
      flagged_diffs: [1],
      fit: { slope: 5, ngood: 2, dq: 6, noise: null, n_usable: 3, first_saturated: 3 },
    });
    const s = rampInspectorSeries(fit);
    expect(s.t).toEqual([0, 10, 20, 30]);
    expect(s.raw).toEqual([1100, 1150, 1700, 65000]);
    expect(s.corrected).toEqual([100, 150, 700, null]);
    expect(s.flagged).toEqual([null, null, 700, null]);
    expect(s.excluded).toEqual([null, null, null, 65000]);
  });
});

describe("compareVerdict", () => {
  it("passes on the window-default numbers of the owner pairs", () => {
    const verdict = compareVerdict(comparison());
    expect(verdict).toEqual({ passed: true, reasons: [] });
  });

  it("fails on the naive NRS2 offsets and names the amplifier and the bin", () => {
    const verdict = compareVerdict(
      comparison({ amp_bins: [ampBins(0, 0.01, 0.01), ampBins(1, -0.2648, -0.21), ampBins(2, 0.01, 0.01), ampBins(3, 0.01, 0.01)] }),
    );
    expect(verdict.passed).toBe(false);
    expect(verdict.reasons).toHaveLength(2);
    expect(verdict.reasons[0]).toContain("amplifier 1");
    expect(verdict.reasons[0]).toContain("[-inf,0.05)");
    expect(verdict.reasons[0]).toContain("-0.2648");
    expect(verdict.reasons[1]).toContain("[0.05,0.3)");
  });

  it("fails when the row profile is shifted and names best_dy", () => {
    const verdict = compareVerdict(
      comparison({ zero_shift: { best_dy: 3, corr_at_zero: 0.42, corr_best: 0.97, second_dy: 2, second_corr: 0.6, passed: false } }),
    );
    expect(verdict.passed).toBe(false);
    expect(verdict.reasons.join(" ")).toContain("best dy +3");
  });

  it("fails a zero shift whose correlation at dy 0 is below the gate even if the flag says passed", () => {
    const verdict = compareVerdict(
      comparison({ zero_shift: { best_dy: 0, corr_at_zero: 0.95, corr_best: 0.95, second_dy: 1, second_corr: 0.9, passed: true } }),
    );
    expect(verdict.passed).toBe(false);
    expect(verdict.reasons[0]).toContain("0.9500");
  });

  it("reads the full-frame faint bins when there is no amplifier table", () => {
    const bins = comparison().bins.map((b) => (b.hi === 0.05 ? { ...b, median_delta: -0.05 } : b));
    const verdict = compareVerdict(comparison({ amp_bins: [], bins }));
    expect(verdict.passed).toBe(false);
    expect(verdict.reasons[0]).toContain("full frame");
    expect(verdict.reasons[0]).toContain("[-inf,0.05)");
  });

  it("gates the bright bins full frame on delta/rate and ignores bins with fewer than fifty pixels", () => {
    const high = comparison().bins.map((b) => (b.lo === 1 ? { ...b, median_rel: 0.0346 } : b));
    const verdict = compareVerdict(comparison({ bins: high }));
    expect(verdict.passed).toBe(false);
    expect(verdict.reasons[0]).toContain("[1,3)");
    expect(verdict.reasons[0]).toContain("+3.46 %");

    const sparse = comparison().bins.map((b) => (b.lo === 10 ? { ...b, median_rel: 0.08, n: ACCEPTANCE.brightMinN - 1 } : b));
    expect(compareVerdict(comparison({ bins: sparse })).passed).toBe(true);

    const low = comparison().bins.map((b) => (b.lo === 3 ? { ...b, median_rel: -0.02 } : b));
    expect(compareVerdict(comparison({ bins: low })).reasons[0]).toContain("[3,10)");
  });

  it("does not pass a gate it could not evaluate", () => {
    const faintOnly = comparison().bins.filter((b) => b.hi !== null && b.hi <= 0.3);
    const verdict = compareVerdict(comparison({ bins: faintOnly }));
    expect(verdict.passed).toBe(false);
    expect(verdict.reasons).toEqual(["G3: no bright bin with n >= 50"]);

    const noFaint = compareVerdict(comparison({ amp_bins: [], bins: comparison().bins.filter((b) => b.lo !== null && b.lo >= 0.3) }));
    expect(noFaint.reasons).toEqual(["G2: no faint bin to evaluate"]);
  });
});

describe("compare formatting", () => {
  it("formats a bright bin in the critic's layout", () => {
    expect(
      formatCompareBin({
        lo: 1,
        hi: 3,
        n: 432,
        median_rate: 1.62,
        median_err: 0.12,
        median_delta: 0.018,
        median_rel: 0.011,
        p16_delta: -0.09,
        p84_delta: 0.13,
      }),
    ).toBe("[1,3) n=432 rate 1.62 err 0.12 delta +0.018 (+1.1 %) p16..p84 -0.09..+0.13");
  });

  it("formats a faint bin without delta/rate and with infinite edges sent as null", () => {
    expect(
      formatCompareBin({
        lo: null,
        hi: 0.05,
        n: 1200000,
        median_rate: 0.012,
        median_err: null,
        median_delta: -0.0014,
        median_rel: null,
        p16_delta: -0.061,
        p84_delta: 0.058,
      }),
    ).toBe("[-inf,0.05) n=1200000 rate 0.01 err n/a delta -0.001 p16..p84 -0.06..+0.06");
    expect(formatCompareBin({ ...bin(30, null, 1.5, 0.05), n: 20 })).toMatch(/^\[30,inf\) n=20 /);
  });

  it("formats the zero-shift check", () => {
    expect(formatShiftCheck({ best_dy: 0, corr_at_zero: 0.9996, corr_best: 0.9996, second_dy: 1, second_corr: 0.945, passed: true })).toBe(
      "aligned: best dy 0, corr 0.9996 (next dy +1, 0.945)",
    );
    expect(formatShiftCheck({ best_dy: 3, corr_at_zero: 0.1234, corr_best: 0.9, second_dy: -2, second_corr: 0.85, passed: false })).toBe(
      "NOT aligned: best dy +3, corr 0.9000, corr at dy 0 0.1234 (next dy -2, 0.850)",
    );
    expect(formatShiftCheck({ best_dy: 0, corr_at_zero: null, corr_best: null, second_dy: 0, second_corr: null, passed: false })).toBe(
      "NOT aligned: best dy 0, corr n/a, corr at dy 0 n/a (next dy 0, n/a)",
    );
  });

  it("formats recall and precision as percentages and n/a when undefined", () => {
    expect(formatConfusion({ tp: 120, fp: 30, fn_: 10, recall: 120 / 130, precision: 0.8 })).toBe(
      "tp 120, fp 30, fn 10, recall 92.3 %, precision 80.0 %",
    );
    expect(formatConfusion(NO_CONFUSION)).toBe("tp 0, fp 0, fn 0, recall n/a, precision n/a");
  });

  it("lists the two faint medians of every amplifier with a null for a missing bin", () => {
    const amp = ampBins(2, 0.0097, 0.012);
    const rows = ampFaintRows(comparison({ amp_bins: [amp, { ...amp, amplifier: 3, science_rows: [0, 512], bins: [amp.bins[1]] }] }));
    expect(rows).toEqual([
      { amplifier: 2, rows: "1024..1536", deltas: [0.0097, 0.012] },
      { amplifier: 3, rows: "0..512", deltas: [null, 0.012] },
    ]);
  });

  it("states the share of good pixels the quick slope flagged", () => {
    expect(quickFlaggedShare(comparison())).toBe("0.16 %");
    expect(quickFlaggedShare(comparison({ good_pixels: 0, quick_flagged_in_good: 0 }))).toBe("n/a");
  });

  it("turns histogram edges into bin centres", () => {
    expect(histogramSeries({ edges: [0, 1, 2], counts: [3, 4] })).toEqual({ x: [0.5, 1.5], y: [3, 4] });
    expect(histogramSeries({ edges: [], counts: [] })).toEqual({ x: [], y: [] });
  });
});

describe("parseUncalRows", () => {
  it("accepts both rows or neither", () => {
    expect(parseUncalRows("", " ")).toEqual({ rows: null, error: null });
    expect(parseUncalRows("1300", "1500")).toEqual({ rows: [1300, 1500], error: null });
  });

  it("rejects one row, non-integers, negatives and an empty window", () => {
    expect(parseUncalRows("1300", "").error).toMatch(/both/);
    expect(parseUncalRows("13.5", "1500").error).toMatch(/whole numbers/);
    expect(parseUncalRows("-1", "1500").error).toMatch(/whole numbers/);
    expect(parseUncalRows("1500", "1500").error).toMatch(/below/);
  });
});

describe("ramp header summary", () => {
  it("reads the owner's NRS2 header the way the Ramp section shows it", () => {
    expect(rampIdentity(owner)).toBe("NIRSPEC NRS2 NRSIRS2RAPID NRS_IFU");
    expect(rampShape(owner)).toBe("10 x 1");
    expect(groupTimeSummary(owner)).toBe("TGROUP from header 14.589 s");
    expect(irs2Badge(owner)).toBe("IRS2 16/4 reversed");
  });

  it("drops absent cards and the reversal for NRS1", () => {
    expect(rampIdentity({ ...owner, exp_type: null, readpatt: null })).toBe("NIRSPEC NRS2");
    expect(irs2Badge({ ...owner, detector: "NRS1" })).toBe("IRS2 16/4");
    expect(irs2Badge(roman)).toBeNull();
    expect(groupTimeSummary(roman)).toBe("group time unknown: slope disabled");
    expect(rampShape(roman)).toBe("8 resultants");
  });
});

describe("table cells", () => {
  it("prints integers as they are, floats to twelve significant digits and nulls as null", () => {
    expect(formatTableCell(7)).toBe("7");
    expect(formatTableCell(60123.456789123)).toBe("60123.4567891");
    expect(formatTableCell(14.589)).toBe("14.589");
    expect(formatTableCell(null)).toBe("null");
  });
});

describe("quick slope parameter drafts", () => {
  it("round-trips the defaults and sends null for the whole band", () => {
    const draft = draftFromParams(DEFAULT_QUICK_SLOPE_PARAMS);
    expect(draft).toEqual({
      sat_dn: "62258",
      jump_k: 5,
      scale_floor_dn: "3",
      min_groups_ols: "4",
      ref_correction: "auto",
      ref_window_rows: "200",
      whole_band: false,
    });
    expect(paramsFromDraft(draft)).toEqual(DEFAULT_QUICK_SLOPE_PARAMS);
    expect(paramsFromDraft({ ...draft, whole_band: true }).ref_window_rows).toBeNull();
    expect(draftFromParams({ ...DEFAULT_QUICK_SLOPE_PARAMS, ref_window_rows: null })).toMatchObject({ whole_band: true, ref_window_rows: "200" });
  });

  it("turns empty or non-numeric text into NaN so validation names the field", () => {
    const draft = { ...draftFromParams(DEFAULT_QUICK_SLOPE_PARAMS), sat_dn: " ", min_groups_ols: "four" };
    const params = paramsFromDraft(draft);
    expect(Number.isNaN(params.sat_dn)).toBe(true);
    expect(Number.isNaN(params.min_groups_ols)).toBe(true);
    expect(validateQuickSlopeParams(params)).toEqual([expect.stringMatching(/^saturation/), expect.stringMatching(/^min groups/)]);
  });
});

describe("QSLOPE_ACCURACY_NOTE", () => {
  it("tells the user the product is not a rate", () => {
    expect(QSLOPE_ACCURACY_NOTE).toContain("not a rate");
    expect(QSLOPE_ACCURACY_NOTE).toContain("8 program-1266 NIRSpec IRS2 exposures");
  });
});

const nrs1: RampInfo = {
  ...owner,
  detector: "NRS1",
  irs2: { nrs_norm: 16, nrs_ref: 4, noutputs: 5, fast_axis: 2, slow_axis: 1 },
};

const threeInts: RampInfo = { ...nrs1, nints: 3 };

function maskScienceRows(nFast: number, reversed: boolean): number[] {
  const outputLen = nFast / 5;
  const rows: number[] = [];
  for (let y = 0; y < nFast; y++) {
    const d = reversed ? nFast - 1 - y : y;
    if (d < outputLen) continue;
    const p = (d - outputLen) % 20;
    if (p < 8 || p >= 12) rows.push(y);
  }
  return rows;
}

describe("IRS2 row mapping", () => {
  it("maps quick-slope rows to the uncal rows the Rust layout pins", () => {
    expect(uncalRowOfScienceRow(528, nrs1)).toBe(1300);
    expect(uncalRowOfScienceRow(560, owner)).toBe(700);
    expect(uncalRowOfScienceRow(1040, owner)).toBe(1300);
    expect(uncalRowOfScienceRow(1000, nrs1)).toBe(1892);
    expect([640, 647, 652, 3199].map((y) => scienceRowOfUncalRow(y, nrs1))).toEqual([0, 7, 8, 2047]);
    expect([648, 649, 650, 651].map((y) => scienceRowOfUncalRow(y, nrs1))).toEqual([null, null, null, null]);
    expect(scienceRowOfUncalRow(0, owner)).toBe(0);
    expect(scienceRowOfUncalRow(2560, owner)).toBeNull();
  });

  it("enumerates exactly the science rows of the jwst mask in increasing order in both orientations", () => {
    for (const [ramp, reversed] of [
      [nrs1, false],
      [owner, true],
    ] as const) {
      const science = maskScienceRows(3200, reversed);
      expect(irs2ScienceRowCount(ramp)).toBe(2048);
      expect(Array.from({ length: 2048 }, (_, r) => uncalRowOfScienceRow(r, ramp))).toEqual(science);
      const scienceIndex = new Map(science.map((y, r) => [y, r]));
      expect(Array.from({ length: 3200 }, (_, y) => scienceRowOfUncalRow(y, ramp))).toEqual(
        Array.from({ length: 3200 }, (_, y) => scienceIndex.get(y) ?? null),
      );
    }
  });

  it("has no mapping without an unstripped five-output IRS2 layout or outside the rows", () => {
    const fourOutputs = { nrs_norm: 16, nrs_ref: 4, noutputs: 4, fast_axis: -2, slow_axis: -1 };
    expect(irs2ScienceRowCount(roman)).toBeNull();
    expect(irs2ScienceRowCount({ ...owner, frame_height: 2048 })).toBeNull();
    expect(irs2ScienceRowCount({ ...owner, irs2: fourOutputs })).toBeNull();
    expect(irs2ScienceRowCount({ ...owner, detector: null })).toBeNull();
    expect(uncalRowOfScienceRow(5, roman)).toBeNull();
    expect([-1, 2048, 1.5].map((r) => uncalRowOfScienceRow(r, nrs1))).toEqual([null, null, null]);
    expect([-1, 3200, 2.5].map((y) => scienceRowOfUncalRow(y, nrs1))).toEqual([null, null, null]);
  });
});

describe("ramp pixel target", () => {
  it("passes a click on a ramp frame through unchanged", () => {
    expect(rampPixelTarget(1000, 1000, nrs1, [2048, 3200])).toEqual({ ok: true, x: 1000, y: 1000, stripped: false });
    expect(rampPixelTarget(3, 3199, nrs1, null)).toEqual({ ok: true, x: 3, y: 3199, stripped: false });
    expect(rampDisplayMapping(nrs1, [2048, 3200])).toBe("frame");
    expect(rampDisplayNote(nrs1, [2048, 3200])).toBeNull();
  });

  it("maps a click on the stripped quick-slope or ratio image to the uncal row", () => {
    expect(rampPixelTarget(1000, 1000, nrs1, [2048, 2048])).toEqual({ ok: true, x: 1000, y: 1892, stripped: true });
    expect(rampPixelTarget(5, 560, owner, [2048, 2048])).toEqual({ ok: true, x: 5, y: 700, stripped: true });
    expect(rampDisplayMapping(owner, [2048, 2048])).toBe("stripped");
    expect(rampDisplayNote(owner, [2048, 2048])).toMatch(/uncal row/);
  });

  it("refuses a click outside the image or on an image that does not map onto the ramp frame", () => {
    expect(rampPixelTarget(2048, 0, nrs1, [2048, 3200]).ok).toBe(false);
    expect(rampPixelTarget(0, 2048, nrs1, [2048, 2048]).ok).toBe(false);
    expect(rampPixelTarget(-1, 0, nrs1, null).ok).toBe(false);
    const odd = rampPixelTarget(10, 10, nrs1, [1024, 1024]);
    expect(odd.ok).toBe(false);
    if (!odd.ok) expect(odd.reason).toMatch(/1024 x 1024.*2048 x 3200.*Back to file/);
    expect(rampDisplayMapping(roman, [4096, 2048])).toBe("unmappable");
    expect(rampDisplayMapping({ ...owner, frame_height: 2048 }, [2048, 2048])).toBe("frame");
    expect(rampDisplayNote(nrs1, [1024, 1024])).toMatch(/1024 x 1024/);
  });
});

describe("ramp inspect label", () => {
  it("names the uncal pixel and the quick-slope pixel it lands on", () => {
    expect(rampInspectLabel({ x: 1000, y: 1892 }, nrs1, 0)).toBe("uncal pixel (1000, 1892), quick-slope pixel (1000, 1000)");
    expect(rampInspectLabel({ x: 3, y: 650 }, nrs1, 0)).toBe("uncal pixel (3, 650), IRS2 reference row: no quick-slope pixel");
    expect(rampInspectLabel({ x: 3, y: 100 }, nrs1, 0)).toBe("uncal pixel (3, 100), IRS2 reference row: no quick-slope pixel");
  });

  it("drops the IRS2 part without a layout and names the integration only above one", () => {
    expect(rampInspectLabel({ x: 1, y: 2 }, roman, 0)).toBe("pixel (1, 2)");
    expect(rampInspectLabel({ x: 1000, y: 1892 }, threeInts, 1)).toBe(
      "uncal pixel (1000, 1892), quick-slope pixel (1000, 1000), integration 2 of 3",
    );
  });
});

describe("compare panel key", () => {
  it("changes with every run even when the product and rate paths repeat", () => {
    const q = "C:/out/jw_nrs1_qslope.fits";
    const r = "C:/fits/jw_nrs1_rate.fits";
    expect(rampCompareKey(1, q, r)).not.toBe(rampCompareKey(2, q, r));
    expect(rampCompareKey(1, q, r)).toBe(rampCompareKey(1, q, r));
    expect(rampCompareKey(1, q, r)).not.toBe(rampCompareKey(1, q, "C:/other_rate.fits"));
  });
});

describe("frame publishers", () => {
  it("leave the gate current after a frame so the deferred FITS load runs, and supersede it for a non-frame result", () => {
    const gate = createFramePublishGate();
    gate.commit();
    const published: string[] = [];
    const publish = framePublishers<string>(gate, (r) => published.push(r));
    publish.frame("resultant 3 png");
    expect(gate.isCurrent()).toBe(true);
    publish.frame("resultant 3 fits");
    expect(gate.isCurrent()).toBe(true);
    publish.result("quick slope");
    expect(gate.isCurrent()).toBe(false);
    expect(published).toEqual(["resultant 3 png", "resultant 3 fits", "quick slope"]);
  });
});

describe("cube panel gates", () => {
  function deferredFrame(gate: { isCurrent(): boolean }, shown: string[], record: string) {
    if (gate.isCurrent()) shown.push(record);
  }

  it("refuse the Spectrum navigator's deferred FITS frame once a Ramp quick slope or ratio image is published", () => {
    for (const label of ["Quick slope (DN/s)", "Quick / rate ratio"]) {
      const gates = cubePanelGates();
      const shown: string[] = [];
      const ramp = framePublishers<string>(gates.ramp, (r) => shown.push(r));
      gates.spectrum.commit();
      shown.push("frame 7 png");
      ramp.result(label);
      deferredFrame(gates.spectrum, shown, "frame 7 fits");
      expect(shown).toEqual(["frame 7 png", label]);
      gates.spectrum.commit();
      deferredFrame(gates.spectrum, shown, "frame 8 fits");
      expect(shown).toEqual(["frame 7 png", label, "frame 8 fits"]);
    }
  });

  it("refuse a deferred ASDF resultant once the Spectrum panel publishes a result, and keep resultant frames non-superseding", () => {
    const gates = cubePanelGates();
    const shown: string[] = [];
    const ramp = framePublishers<string>(gates.ramp, (r) => shown.push(r));
    gates.ramp.commit();
    ramp.frame("resultant 2 png");
    expect(gates.ramp.isCurrent()).toBe(true);
    gates.spectrum.supersede();
    shown.push("moment 0");
    deferredFrame(gates.ramp, shown, "resultant 2 fits");
    expect(shown).toEqual(["resultant 2 png", "moment 0"]);
  });
});

describe("compare verdict title", () => {
  it("says full-frame tolerances for a full frame and labels a row window as windowed", () => {
    const full = comparison();
    expect(compareVerdictTitle(full, compareVerdict(full))).toBe("Within the published tolerances (G1-G3)");
    const shifted = comparison({ zero_shift: { ...full.zero_shift, best_dy: 512, passed: false } });
    expect(compareVerdictTitle(shifted, compareVerdict(shifted))).toBe("Outside the published tolerances");
    const windowed = comparison({ science_rows: [528, 688] });
    expect(compareVerdictTitle(windowed, compareVerdict(windowed))).toBe(
      "Science rows 528..688 within the full-frame tolerances (G1-G3 are full-frame gates)",
    );
    const windowedShifted = comparison({ zero_shift: shifted.zero_shift, science_rows: [528, 688] });
    expect(compareVerdictTitle(windowedShifted, compareVerdict(windowedShifted))).toBe(
      "Science rows 528..688 outside the full-frame tolerances (G1-G3 are full-frame gates)",
    );
  });
});

describe("histogram summary", () => {
  it("describes the bins and the peak for screen readers", () => {
    expect(histogramSummary({ edges: [-0.1, 0, 0.1, 0.2], counts: [5, 20, 3] }, "delta / rate")).toBe(
      "delta / rate histogram: 3 bins from -0.1 to 0.2, peak bin centred at 0.05 with 20 pixels",
    );
    expect(histogramSummary({ edges: [], counts: [] }, "delta")).toBe("delta histogram: no data");
  });
});

describe("ramp frame marker", () => {
  it("keeps the channel marker for other cubes", () => {
    expect(rampFrameMarker(17, 40, null)).toEqual({ channel: 17, text: "ch 17" });
    expect(rampFrameMarker(40, 40, null)).toBeNull();
    expect(rampFrameMarker(null, 40, owner)).toBeNull();
  });

  it("marks the group of the shown frame on a per-integration ramp series", () => {
    expect(rampFrameMarker(2, 10, owner)).toEqual({ channel: 2, text: "group 3" });
    expect(rampFrameMarker(13, 10, threeInts)).toEqual({ channel: 3, text: "group 4" });
    expect(rampFrameMarker(13, 30, threeInts)).toEqual({ channel: 13, text: "group 4, integration 2" });
    expect(rampFrameMarker(2, 8, roman)).toEqual({ channel: 2, text: "resultant 3" });
  });

  it("turns a double-clicked group into the frame of the selected integration", () => {
    expect(rampFrameFromPlotChannel(3, 10, threeInts, 1)).toBe(13);
    expect(rampFrameFromPlotChannel(13, 30, threeInts, 1)).toBe(13);
    expect(rampFrameFromPlotChannel(3, 10, owner, 0)).toBe(3);
    expect(rampFrameFromPlotChannel(7, 40, null, 2)).toBe(7);
  });
});

describe("quick slope availability when the cube reader refused the file", () => {
  it("names the refusal so the run button is not offered for a run that must fail", () => {
    expect(quickSlopeUnavailableReason(owner, "fits", true)).toMatch(/cube reader refused/);
    expect(quickSlopeUnavailableReason(owner, "fits", false)).toBeNull();
  });
});
