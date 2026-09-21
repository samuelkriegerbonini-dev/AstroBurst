import { describe, it, expect } from "vitest";
import {
  rejectionFrameHint,
  rejectionUsesSigma,
  subframeWeightsFor,
  selectableStackPaths,
  stackProgressText,
  appendMissingPaths,
  REJECTION_OPTIONS,
  COMBINE_OPTIONS,
  NORMALIZATION_OPTIONS,
} from "../stackingRejection";

describe("rejectionFrameHint", () => {
  it("warns below the percentile minimum and is fine from three frames", () => {
    expect(rejectionFrameHint("percentile_clip", 2).severity).toBe("warn");
    expect(rejectionFrameHint("percentile_clip", 2).text).toContain("3");
    expect(rejectionFrameHint("percentile_clip", 3).severity).toBe("ok");
  });

  it("recommends five frames for sigma and winsorized clipping", () => {
    expect(rejectionFrameHint("sigma_clip", 4).severity).toBe("warn");
    expect(rejectionFrameHint("winsorized_sigma_clip", 4).severity).toBe("warn");
    expect(rejectionFrameHint("sigma_clip", 5).severity).toBe("ok");
  });

  it("says linear fit falls back below five and recommends eight", () => {
    const tooFew = rejectionFrameHint("linear_fit_clip", 4);
    expect(tooFew.severity).toBe("warn");
    expect(tooFew.text).toContain("sigma clipping");
    expect(rejectionFrameHint("linear_fit_clip", 6).severity).toBe("warn");
    expect(rejectionFrameHint("linear_fit_clip", 8).severity).toBe("ok");
  });

  it("requires more frames than low plus high for min/max", () => {
    expect(rejectionFrameHint("min_max", 2, 1, 1).severity).toBe("warn");
    expect(rejectionFrameHint("min_max", 3, 1, 1).severity).toBe("ok");
    expect(rejectionFrameHint("min_max", 5, 2, 3).severity).toBe("warn");
    expect(rejectionFrameHint("min_max", 6, 2, 3).severity).toBe("ok");
  });

  it("never warns for no rejection", () => {
    expect(rejectionFrameHint("none", 1).severity).toBe("ok");
  });
});

describe("rejectionUsesSigma", () => {
  it("is true only for the sigma-based methods", () => {
    expect(rejectionUsesSigma("sigma_clip")).toBe(true);
    expect(rejectionUsesSigma("winsorized_sigma_clip")).toBe(true);
    expect(rejectionUsesSigma("linear_fit_clip")).toBe(true);
    expect(rejectionUsesSigma("percentile_clip")).toBe(false);
    expect(rejectionUsesSigma("min_max")).toBe(false);
    expect(rejectionUsesSigma("none")).toBe(false);
  });
});

describe("subframeWeightsFor", () => {
  it("returns undefined without any known weight", () => {
    expect(subframeWeightsFor(["a", "b"], undefined)).toBeUndefined();
    expect(subframeWeightsFor(["a", "b"], { c: 0.5 })).toBeUndefined();
  });

  it("aligns weights with the selected paths and defaults unknown ones to 1", () => {
    expect(subframeWeightsFor(["a", "b", "c"], { a: 0.25, c: 0.75 })).toEqual([0.25, 1.0, 0.75]);
  });
});

describe("select options", () => {
  it("cover every backend name exactly once", () => {
    expect(REJECTION_OPTIONS.map((o) => o.value)).toEqual([
      "none",
      "sigma_clip",
      "winsorized_sigma_clip",
      "linear_fit_clip",
      "percentile_clip",
      "min_max",
    ]);
    expect(COMBINE_OPTIONS.map((o) => o.value)).toEqual(["mean", "median", "min", "max"]);
    expect(NORMALIZATION_OPTIONS.map((o) => o.value)).toEqual([
      "none",
      "additive",
      "multiplicative",
      "additive_scaling",
      "multiplicative_scaling",
    ]);
  });
});

describe("selectableStackPaths", () => {
  it("skips frames the subframe selector rejected", () => {
    expect(selectableStackPaths(["a", "b", "c"], [], ["b"])).toEqual(["a", "c"]);
  });

  it("keeps calibrated injections that are not already in the file list", () => {
    expect(selectableStackPaths(["a"], ["a", "cal"], [])).toEqual(["a", "cal"]);
  });

  it("rejects injected paths too", () => {
    expect(selectableStackPaths(["a"], ["cal"], ["cal"])).toEqual(["a"]);
  });
});

describe("appendMissingPaths", () => {
  it("appends only the paths that are not selected yet", () => {
    expect(appendMissingPaths(["a", "b"], ["b", "c"])).toEqual(["a", "b", "c"]);
  });

  it("keeps the current selection when nothing is new", () => {
    expect(appendMissingPaths(["a"], ["a"])).toEqual(["a"]);
  });
});

describe("stackProgressText", () => {
  it("falls back to a generic label before the first tick", () => {
    expect(stackProgressText("", 0, 0)).toBe("Stacking frames...");
  });

  it("shows the frame counter once the backend ticks per frame", () => {
    expect(stackProgressText("load_frame", 7, 62)).toBe("Loading frames (7/62)");
  });

  it("shows the stage alone when the backend only reports stages", () => {
    expect(stackProgressText("save", 0, 0)).toBe("Saving");
  });
});
