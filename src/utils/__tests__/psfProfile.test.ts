import { describe, it, expect } from "vitest";
import { profilePoints, starStatusLabel } from "../psfProfile";

describe("profilePoints", () => {
  it("spreads the radii over the width and scales the peak to the top", () => {
    expect(profilePoints([4, 2, 1, 0], 30, 10)).toBe("0,0 10,5 20,7.5 30,10");
  });

  it("skips non-finite bins and clamps negatives to the baseline", () => {
    expect(profilePoints([2, Number.NaN, -1, 1], 30, 10)).toBe("0,0 20,10 30,5");
  });

  it("returns nothing when the profile has no positive value", () => {
    expect(profilePoints([], 30, 10)).toBe("");
    expect(profilePoints([0, -1, Number.NaN], 30, 10)).toBe("");
  });

  it("puts a single bin at the left edge", () => {
    expect(profilePoints([3], 30, 10)).toBe("0,0");
  });
});

describe("starStatusLabel", () => {
  it("names the centroid of a used star and the reason of a rejected one", () => {
    expect(starStatusLabel({ used: true, rejected: null, centroid: "quadratic" })).toBe("used (quadratic centroid)");
    expect(starStatusLabel({ used: true, rejected: null, centroid: "peak" })).toBe("used (peak fallback)");
    expect(starStatusLabel({ used: false, rejected: "the cutout lies outside the image", centroid: null })).toBe("rejected: the cutout lies outside the image");
  });
});
