import { readFileSync } from "node:fs";
import { describe, it, expect } from "vitest";
import { detectProductTypes, extractProductType, fileSearchText, filtersRevealing, matchesActiveFilters, metadataFilterable, processedFilterable, productFilterStore } from "../useProductFilter";
import { displayFilterValue } from "../../utils/channelMapping";

const WFPC2 = [
  { name: "502nmos.fits", filter: "F502N", instrument: "WFPC2" },
  { name: "656nmos.fits", filter: "F656N", instrument: "WFPC2" },
  { name: "673nmos.fits", filter: "F673N", instrument: "WFPC2" },
];

const NIRCAM = { name: "jw02739-o001_t001_nircam_clear-f200w_i2d.fits", filter: "F200W", instrument: "NIRCAM" };
const NIRCAM_BARE = { name: "jw02739-o001_t001_nrcb1_i2d.fits", filter: "F200W", instrument: "NIRCAM" };
const TSO_CALINTS = "jw01366004001_04101_00001-seg001_nis_calints.fits";
const TSO_X1DINTS = "jw01366004001_04101_00001-seg001_nis_x1dints.fits";

describe("extractProductType", () => {
  it("gives the quick-slope product its own chip next to uncal and rate", () => {
    expect(extractProductType("jw_nrs1_qslope.fits")).toBe("qslope");
    expect(extractProductType("jw_nrs1_uncal.fits")).toBe("uncal");
    expect(extractProductType("jw_nrs1_rate.fits")).toBe("rate");
  });

  it("tags a gzip-compressed FITS product like its plain file", () => {
    expect(extractProductType("jw01_nrs1_s3d.fits.gz")).toBe("s3d");
    expect(extractProductType("jw01_nrs1_x1d.FIT.GZ")).toBe("x1d");
    expect(extractProductType("jw01_nrcb1_cal.fts.gz")).toBe("cal");
  });

  it("allows the gzip suffix after the FITS spellings only", () => {
    expect(extractProductType("jw01_nrs1_cal.asdf.gz")).toBeNull();
    expect(extractProductType("jw01_nrs1_s3d.gz")).toBeNull();
    expect(extractProductType("jw01_nrs1_s3d.tar.gz")).toBeNull();
  });

  it("gives the per-integration products of a time series their own chips", () => {
    expect(extractProductType(TSO_CALINTS)).toBe("calints");
    expect(extractProductType(TSO_X1DINTS)).toBe("x1dints");
    expect(extractProductType("jw01366004001_04101_00001-seg001_nis_rateints.fits")).toBe("rateints");
  });

  it("gives the remaining time-series products their own chips", () => {
    expect(extractProductType("jw02358001001_04101_00001-seg001_nrs1_crfints.fits")).toBe("crfints");
    expect(extractProductType("jw02358001001_04101_00001-seg001_nrs1_crf.fits")).toBe("crf");
    expect(extractProductType("jw01366-o003_t001_nirspec_f290lp-g395h_c1d.fits")).toBe("c1d");
  });

  it("tags the white-light table by the ECSV file the pipeline writes", () => {
    expect(extractProductType("jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_whtlt.ecsv")).toBe("whtlt");
    expect(extractProductType("x_whtlt.ECSV")).toBe("whtlt");
    expect(extractProductType("x_whtlt.csv")).toBeNull();
    expect(extractProductType("x_whtlt.ecsv.gz")).toBeNull();
    expect(extractProductType("jw1_cat.ecsv")).toBeNull();
    expect(extractProductType("jw1_cal.ecsv")).toBeNull();
    expect(extractProductType("jw_whtlt.fits")).toBe("whtlt");
  });

  it("tags a bzip2-compressed FITS product like its plain file and nothing else", () => {
    expect(extractProductType("jw01_nrs1_x1d.fits.bz2")).toBe("x1d");
    expect(extractProductType("jw01_nrs1_cal.FIT.BZ2")).toBe("cal");
    expect(extractProductType("frame-r-001462-4-0165.fits.bz2")).toBeNull();
    expect(extractProductType("jw01_nrs1_s3d.bz2")).toBeNull();
    expect(extractProductType("jw01_nrs1_cal.asdf.bz2")).toBeNull();
  });
});

describe("product chips", () => {
  const exposures = ["00001", "00002", "00003", "00004"].flatMap((exposure) => ["nrs1", "nrs2"].map((detector) => `jw01266005001_02103_${exposure}_${detector}`));
  const jw01266 = [
    ...exposures.flatMap((exposure) => ["cal", "uncal", "rate", "rateints", "s3d", "x1d"].map((product) => `${exposure}_${product}.fits`)),
    ...["g235h-f170lp", "g395h-f290lp"].flatMap((grating) => ["s3d", "x1d"].map((product) => `jw01266-o005_t001_nirspec_${grating}_${product}.fits`)),
  ];
  const kept = (names: string[], filters: string[], mode: "or" | "and" = "or") => names.filter((name) => matchesActiveFilters({ name }, filters, mode));

  it("finds the six product chips of the 52 jw01266 NIRSpec IFU files", () => {
    expect(jw01266).toHaveLength(52);
    expect(detectProductTypes(jw01266)).toEqual(["cal", "rate", "rateints", "s3d", "uncal", "x1d"]);
  });

  it.each([
    ["cal", 8],
    ["uncal", 8],
    ["rate", 8],
    ["rateints", 8],
    ["s3d", 10],
    ["x1d", 10],
  ])("keeps only the %s files under their chip", (chip, count) => {
    const files = kept(jw01266, [chip]);
    expect(files.filter((name) => extractProductType(name) !== chip)).toEqual([]);
    expect(files).toHaveLength(count);
  });

  it("leaves the per-integration calints and x1dints products out of the cal and x1d chips", () => {
    expect(kept([TSO_CALINTS, TSO_X1DINTS], ["cal"])).toEqual([]);
    expect(kept([TSO_CALINTS, TSO_X1DINTS], ["x1d"])).toEqual([]);
    expect(kept([TSO_CALINTS, TSO_X1DINTS], ["calints"])).toEqual([TSO_CALINTS]);
    expect(kept([TSO_CALINTS, TSO_X1DINTS], ["x1dints"])).toEqual([TSO_X1DINTS]);
  });

  it("keeps the white-light tables under their chip and out of the x1dints chip", () => {
    const names = [
      "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_whtlt.ecsv",
      "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits",
      "jw02358001001_04101_00001-seg001_nrs1_crfints.fits",
      "jw02358001001_04101_00001-seg001_nrs1_crf.fits",
    ];
    expect(detectProductTypes(names)).toEqual(["crf", "crfints", "whtlt", "x1dints"]);
    expect(kept(names, ["whtlt"])).toEqual([names[0]]);
    expect(kept(names, ["crf"])).toEqual([names[3]]);
    expect(kept(names, ["x1dints"])).toEqual([names[1]]);
  });

  it("combines a product chip with a text chip on the file name", () => {
    expect(kept(jw01266, ["cal", "nrs1"], "and")).toEqual(exposures.filter((exposure) => exposure.endsWith("nrs1")).map((exposure) => `${exposure}_cal.fits`));
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

describe.skipIf(!process.env.ASTROBURST_SCIENCE_DATA_DIR)("the science data set", () => {
  it("finds the two product chips of the 72 manifest names", () => {
    const manifest = readFileSync(`${process.env.ASTROBURST_SCIENCE_DATA_DIR}/MANIFEST.tsv`, "utf8");
    const names = manifest.split(/\r?\n/).filter((line) => line.length > 0).slice(1).map((line) => line.split("\t")[1]);
    expect(names).toHaveLength(72);
    expect(detectProductTypes(names)).toEqual(["whtlt", "x1dints"]);
    expect(names.filter((name) => matchesActiveFilters({ name }, ["whtlt"], "or"))).toHaveLength(16);
    expect(names.filter((name) => matchesActiveFilters({ name }, ["x1dints"], "or"))).toHaveLength(16);
    const untyped = names.filter((name) => extractProductType(name) === null);
    expect(untyped).toHaveLength(40);
    expect(names.filter((name) => /^frame-.*\.fits\.bz2$/.test(name)).every((name) => untyped.includes(name))).toBe(true);
    expect(names.filter((name) => /\.csv$|_asn\.json$/.test(name)).every((name) => untyped.includes(name))).toBe(true);
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
