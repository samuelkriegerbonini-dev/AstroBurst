import { describe, expect, it } from "vitest";
import { DQ_NOT_APPLIED_PREFIX, STATISTICS_DESCRIPTION, dqExclusionGap } from "../statisticsNotes";

const FLOAT_MASK = "mask layer HDU 2 is a floating-point plane (BITPIX -32): flag bits cannot be read exactly";

describe("dqExclusionGap", () => {
  it("dqExclusionGap gives the reason that applies first", () => {
    expect(dqExclusionGap({ planeReason: FLOAT_MASK, flagTable: null })).toBe(FLOAT_MASK);
    expect(dqExclusionGap({ planeReason: null, flagTable: null })).toBe("this image has no DQ plane");
    expect(dqExclusionGap({ planeReason: null, flagTable: { label: "unknown (raw bits)", flags: [], exclusion_mask: 0 } })).toBe(
      "the flag layer has no usable bit names (unknown (raw bits)), so nothing is excluded until bits are chosen",
    );
    const afw = { label: "MP_ cards (afw mask planes)", flags: [{ bit: 1, name: "BAD" }], exclusion_mask: 0 };
    expect(dqExclusionGap({ planeReason: null, flagTable: afw })).toBe("no flag of this layer is selected for exclusion (MP_ cards (afw mask planes))");
    expect(dqExclusionGap({ planeReason: null, flagTable: { ...afw, dq_unavailable: "mask layer x.fits has no exact integer form" } })).toBe(
      "mask layer x.fits has no exact integer form",
    );
    expect(dqExclusionGap({ planeReason: null, flagTable: { label: "jwst", flags: [{ bit: 1, name: "DO_NOT_USE" }], exclusion_mask: 1 } })).toBe(
      "no flagged pixel could be excluded",
    );
    expect(DQ_NOT_APPLIED_PREFIX).toBe("DQ exclusion requested but not applied: ");
  });
});

describe("STATISTICS_DESCRIPTION", () => {
  it("the description does not claim that zeros are included", () => {
    expect(STATISTICS_DESCRIPTION).toContain("exact zeros are counted as padding (0)");
    expect(STATISTICS_DESCRIPTION).toContain("left out");
    expect(STATISTICS_DESCRIPTION).not.toContain("zeros included");
  });
});
