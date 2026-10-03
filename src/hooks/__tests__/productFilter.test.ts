import { describe, it, expect } from "vitest";
import { extractProductType, fileSearchText, filtersRevealing, matchesActiveFilters, metadataFilterable, processedFilterable, productFilterStore } from "../useProductFilter";
import { displayFilterValue } from "../../utils/channelMapping";

const WFPC2 = [
  { name: "502nmos.fits", filter: "F502N", instrument: "WFPC2" },
  { name: "656nmos.fits", filter: "F656N", instrument: "WFPC2" },
  { name: "673nmos.fits", filter: "F673N", instrument: "WFPC2" },
];

const NIRCAM = { name: "jw02739-o001_t001_nircam_clear-f200w_i2d.fits", filter: "F200W", instrument: "NIRCAM" };
const NIRCAM_BARE = { name: "jw02739-o001_t001_nrcb1_i2d.fits", filter: "F200W", instrument: "NIRCAM" };

describe("extractProductType", () => {
  it("gives the quick-slope product its own chip next to uncal and rate", () => {
    expect(extractProductType("jw_nrs1_qslope.fits")).toBe("qslope");
    expect(extractProductType("jw_nrs1_uncal.fits")).toBe("uncal");
    expect(extractProductType("jw_nrs1_rate.fits")).toBe("rate");
  });
});

describe("fileSearchText", () => {
  it("joins the name, the filter and the instrument in lower case", () => {
    const text = fileSearchText(WFPC2[1]);
    expect(text).toContain("656nmos.fits");
    expect(text).toContain("f656n");
    expect(text).toContain("wfpc2");
  });

  it("does not let a query span two fields", () => {
    expect(fileSearchText({ name: "a.fits", filter: "F200W" }).includes("fits f200w")).toBe(false);
  });

  it("works without metadata", () => {
    expect(fileSearchText({ name: "M42.FITS" })).toBe("m42.fits");
  });
});

describe("matchesActiveFilters", () => {
  it("keeps every WFPC2 file under a pinned instrument chip", () => {
    expect(WFPC2.filter((f) => matchesActiveFilters(f, ["wfpc2"], "or"))).toHaveLength(3);
  });

  it("keeps a JWST file under a pinned filter chip that is not in its name", () => {
    expect(matchesActiveFilters(NIRCAM_BARE, ["f200w"], "or")).toBe(true);
  });

  it("still matches the product type from the bare file name", () => {
    expect(matchesActiveFilters(NIRCAM, ["i2d"], "or")).toBe(true);
    expect(matchesActiveFilters({ name: "jw_x_cal.fits", filter: "F200W", instrument: "NIRCAM" }, ["cal"], "or")).toBe(true);
  });

  it("combines chips with AND and OR", () => {
    expect(WFPC2.filter((f) => matchesActiveFilters(f, ["wfpc2", "f656n"], "and")).map((f) => f.name)).toEqual(["656nmos.fits"]);
    expect(WFPC2.filter((f) => matchesActiveFilters(f, ["f502n", "f673n"], "or")).map((f) => f.name)).toEqual(["502nmos.fits", "673nmos.fits"]);
  });

  it("matches the same text the Files search matches for a real WFPC2 header", () => {
    const header = { FILTNAM1: "F656N", FILTNAM2: "", FILTER1: "31", FILTER2: "0", INSTRUME: "WFPC2" };
    const processed = { name: "656nmos.fits", path: "C:/s/656nmos.fits", result: { header } };
    const listed = { name: "656nmos.fits", metadata: { filter: displayFilterValue(processed) ?? undefined, instrument: header.INSTRUME } };
    expect(processedFilterable(processed)).toEqual(metadataFilterable(listed));
    expect(fileSearchText(metadataFilterable(listed)).includes("f656n")).toBe(true);
    expect(matchesActiveFilters(processedFilterable(processed), ["f656n"], "or")).toBe(true);
    expect(matchesActiveFilters(processedFilterable(processed), ["wfpc2"], "or")).toBe(true);
  });

  it("builds the same match input for a processed file without a header", () => {
    expect(processedFilterable({ name: "a.fits", result: null })).toEqual({ name: "a.fits", filter: null, instrument: undefined });
    expect(metadataFilterable({ name: "a.fits" })).toEqual({ name: "a.fits", filter: undefined, instrument: undefined });
  });

  it("keeps every file when no chip is active", () => {
    expect(matchesActiveFilters({ name: "x.fits" }, [], "and")).toBe(true);
  });

  it("finds a frame by a numeric central wavelength in FILTER", () => {
    const processed = { name: "light_0001.fits", result: { header: { FILTER: "656.3" } } };
    expect(processedFilterable(processed).filter).toBe("656.3");
    expect(matchesActiveFilters(processedFilterable(processed), ["656"], "or")).toBe(true);
  });
});

describe("filtersRevealing", () => {
  const cube = { name: "jw01266005001_02103_00001_nrs1_s3d.fits", filter: "F170LP", instrument: "NIRSPEC" };

  it("keeps the same filter list when the file already matches", () => {
    const filters = ["s3d"];
    expect(filtersRevealing(cube, filters, "or")).toBe(filters);
  });

  it("adds the file's product chip in OR mode so the hidden cube appears beside the x1d files", () => {
    const next = filtersRevealing(cube, ["x1d"], "or");
    expect(next).toEqual(["x1d", "s3d"]);
    expect(matchesActiveFilters(cube, next, "or")).toBe(true);
  });

  it("clears the filters in AND mode, where another chip cannot reveal the file", () => {
    expect(filtersRevealing(cube, ["x1d", "nirspec"], "and")).toEqual([]);
  });

  it("clears the filters when the file has no product chip to add", () => {
    expect(filtersRevealing({ name: "cube.fits", instrument: "NIRSPEC" }, ["x1d"], "or")).toEqual([]);
  });
});

describe("productFilterStore.setActiveFilters", () => {
  it("replaces the active filters and notifies, keeping the custom chips", () => {
    productFilterStore.reset();
    productFilterStore.addCustomChip("m42");
    let calls = 0;
    const off = productFilterStore.subscribe(() => { calls += 1; });
    productFilterStore.setActiveFilters(["x1d", "s3d"]);
    off();
    expect(productFilterStore.getActiveFilters()).toEqual(["x1d", "s3d"]);
    expect(productFilterStore.getSnapshot().customChips).toEqual(["m42"]);
    expect(calls).toBe(1);
    productFilterStore.reset();
  });
});
