import { describe, expect, it } from "vitest";
import { middleEllipsis } from "../filenameDisplay";

const NRCB1 = "jw02739001001_02105_00001_nrcb1_rate.fits";
const NRCA1 = "jw02739001001_02105_00001_nrca1_rate.fits";

describe("middleEllipsis", () => {
  it("keeps the program prefix and the detector and product suffix of a long JWST name within the limit", () => {
    const shown = middleEllipsis(NRCB1, 40);
    expect(shown).toHaveLength(40);
    expect(shown).toContain("…");
    expect(shown.startsWith("jw02739001001_02105")).toBe(true);
    expect(shown.endsWith("_nrcb1_rate.fits")).toBe(true);
  });

  it("tells sibling detectors apart once they are shortened", () => {
    expect(middleEllipsis(NRCB1, 30)).not.toBe(middleEllipsis(NRCA1, 30));
    expect(middleEllipsis(NRCB1, 30)).toHaveLength(30);
  });

  it("returns names that fit unchanged", () => {
    expect(middleEllipsis("cube_s3d.fits", 40)).toBe("cube_s3d.fits");
    expect(middleEllipsis("jw02739001001_02105_00001_nrcb1_cal.fits", 40)).toBe("jw02739001001_02105_00001_nrcb1_cal.fits");
  });

  it("never returns more than the limit, even for a tiny one", () => {
    expect(middleEllipsis(NRCB1, 2)).toBe("…s");
    expect(middleEllipsis(NRCB1, 1)).toBe("…");
  });
});
