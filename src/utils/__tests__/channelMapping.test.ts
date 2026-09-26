import { describe, it, expect } from "vitest";
import {
  assignedPaths,
  colorBinsEmpty,
  detectChannel,
  detectChannelByFilename,
  detectChannelByHeader,
  displayFilterValue,
  exclusivelyAssignedPaths,
  filenameChannelSlot,
  groupByWavelength,
  headerFilterValues,
  resolveFileFilter,
  resolveFilterFromName,
  mapFilesByWavelength,
  runAutoMap,
  splitSpectralThirds,
} from "../channelMapping";
import { DEFAULT_BINS, type FrequencyBin } from "../wizard";

const NIRCAM_NAME = "jw06565-o003_t001_nircam_clear-f444w_i2d.fits";

function fileWithHeader(name: string, header: Record<string, string> | null) {
  return { name, path: `C:/data/${name}`, result: header ? { header } : null };
}

describe("detectChannelByFilename", () => {
  it("does not read the JWST program id as a narrowband wavelength", () => {
    expect(detectChannelByFilename(fileWithHeader(NIRCAM_NAME, null))).toBeNull();
    expect(detectChannelByFilename(fileWithHeader("jw01502-o001_t001_nircam_f200w_i2d.fits", null))).toBeNull();
    expect(detectChannelByFilename(fileWithHeader("jw06730-o001_t001_nircam_f200w_i2d.fits", null))).toBeNull();
    expect(detectChannelByFilename(fileWithHeader("jw02673-o001_t001_nircam_f200w_i2d.fits", null))).toBeNull();
  });

  it("ignores digits that are part of a date stamp or an exposure time", () => {
    expect(detectChannelByFilename(fileWithHeader("M42_20250502_300s.fits", null))).toBeNull();
    expect(detectChannelByFilename(fileWithHeader("M42_673s_stack.fits", null))).toBeNull();
  });

  it("still maps a separated narrowband wavelength", () => {
    expect(detectChannelByFilename(fileWithHeader("M42_656_300s.fits", null))).toBe("ha");
    expect(detectChannelByFilename(fileWithHeader("M42_501nm.fits", null))).toBe("oiii");
    expect(detectChannelByFilename(fileWithHeader("M42_673.fits", null))).toBe("sii");
  });

  it("maps a target name ending in _L to luminance, not to ha", () => {
    expect(detectChannelByFilename(fileWithHeader("ngc6565_L.fits", null))).toBe("l");
  });

  it("treats CLEAR as luminance only outside the MAST pupil slot", () => {
    expect(detectChannelByFilename(fileWithHeader("M42_Clear_300s.fits", null))).toBe("l");
    expect(detectChannelByFilename(fileWithHeader(NIRCAM_NAME, null))).toBeNull();
    expect(detectChannelByFilename(fileWithHeader("jw06565-o003_t001_nircam_f444w-clear_i2d.fits", null))).toBeNull();
  });
});

describe("detectChannelByHeader", () => {
  it("does not map a NIRCam exposure to luminance because of its empty pupil slot", () => {
    const file = fileWithHeader(NIRCAM_NAME, { FILTER: "F444W", INSTRUME: "NIRCAM", PUPIL: "CLEAR" });
    expect(detectChannelByHeader(file)).toBeNull();
  });

  it("keeps mapping a genuine clear filter to luminance", () => {
    const file = fileWithHeader("M42_Clear_300s.fits", { FILTER: "Clear", INSTRUME: "ASI2600" });
    expect(detectChannelByHeader(file)).toBe("l");
  });

  it("does not read a narrowband filter out of an unrelated header word", () => {
    expect(detectChannelByHeader(fileWithHeader("x.fits", { FILTER: "SHARP" }))).toBeNull();
    expect(detectChannelByHeader(fileWithHeader("x.fits", { FILTER: "ALPHA" }))).toBeNull();
    expect(detectChannelByHeader(fileWithHeader("x.fits", { FILTER: "CHANNEL" }))).toBeNull();
    expect(detectChannelByHeader(fileWithHeader("x.fits", { FILTER: "NO3" }))).toBeNull();
  });

  it("keeps mapping a narrowband filter written with a bandwidth suffix", () => {
    expect(detectChannelByHeader(fileWithHeader("x.fits", { FILTER: "Ha_3nm" }))).toBe("ha");
    expect(detectChannelByHeader(fileWithHeader("x.fits", { FILTER: "H-alpha 7nm" }))).toBe("ha");
    expect(detectChannelByHeader(fileWithHeader("x.fits", { FILTER: "O3 6nm" }))).toBe("oiii");
  });

  it("keeps mapping narrowband header values", () => {
    expect(detectChannelByHeader(fileWithHeader("a.fits", { FILTER: "Halpha" }))).toBe("ha");
    expect(detectChannelByHeader(fileWithHeader("b.fits", { FILTER: "F656N" }))).toBe("ha");
    expect(detectChannelByHeader(fileWithHeader("c.fits", { FILTER: "OIII" }))).toBe("oiii");
  });
});

describe("detectChannel on the real MAST file", () => {
  it("resolves the F444W wavelength channel instead of ha or l", () => {
    const file = fileWithHeader(NIRCAM_NAME, { FILTER: "F444W", INSTRUME: "NIRCAM", PUPIL: "CLEAR" });
    expect(detectChannel(file)).toBeNull();
    expect(resolveFileFilter(file)).toEqual({ code: "F444W", nm: 4440 });
    expect(displayFilterValue(file)).toBe("F444W");
  });

  it("resolves the filter from the MAST filename when no header is loaded", () => {
    expect(resolveFilterFromName(NIRCAM_NAME)).toEqual({ code: "F444W", nm: 4440 });
    expect(resolveFilterFromName("jw06565_t001_miri_f1130w_i2d.fits")).toEqual({ code: "F1130W", nm: 11300 });
  });
});

describe("HST filter wheels", () => {
  const WFPC2_656 = { FILTNAM1: "F656N", FILTNAM2: "", FILTER1: "31", FILTER2: "0", INSTRUME: "WFPC2" };

  it("shows the WFPC2 filter name instead of the wheel number", () => {
    expect(displayFilterValue(fileWithHeader("656nmos.fits", WFPC2_656))).toBe("F656N");
    expect(displayFilterValue(fileWithHeader("502nmos.fits", { ...WFPC2_656, FILTNAM1: "F502N", FILTER1: "23" }))).toBe("F502N");
  });

  it("resolves the WFPC2 filter wavelength from FILTNAM1", () => {
    expect(resolveFileFilter(fileWithHeader("673nmos.fits", { ...WFPC2_656, FILTNAM1: "F673N", FILTER1: "33" }))).toEqual({ code: "F673N", nm: 673 });
  });

  it("skips purely numeric wheel positions", () => {
    expect(displayFilterValue(fileWithHeader("x.fits", { FILTER1: "31", FILTER2: "0" }))).toBeNull();
    expect(displayFilterValue(fileWithHeader("x.fits", { FILTER: "3" }))).toBeNull();
  });

  it("keeps an unresolvable filter name when the wheel number is the only other value", () => {
    expect(displayFilterValue(fileWithHeader("x.fits", { FILTNAM1: "F555W", FILTER1: "17" }))).toBe("F555W");
  });

  it("keeps the ACS filter from FILTER1 and ignores its clear slot", () => {
    expect(displayFilterValue(fileWithHeader("j_flt.fits", { FILTER1: "F658N", FILTER2: "CLEAR2L", INSTRUME: "ACS" }))).toBe("F658N");
  });

  it("drops the three WFPC2 sample wheel numbers and keeps only the filter names", () => {
    for (const [name, wheel] of [["F502N", "23"], ["F656N", "31"], ["F673N", "33"]]) {
      expect(headerFilterValues(fileWithHeader("x.fits", { ...WFPC2_656, FILTNAM1: name, FILTER1: wheel }))).toEqual([name]);
    }
  });
});

describe("numeric FILTER values", () => {
  it("still reads a central wavelength in FILTER as its narrowband channel", () => {
    for (const [value, bin] of [["656", "ha"], ["656.3", "ha"], ["5007", "oiii"], ["502", "oiii"], ["673.1", "sii"]]) {
      expect(detectChannelByHeader(fileWithHeader("light_0001.fits", { FILTER: value }))).toBe(bin);
    }
  });

  it("shows a numeric wavelength as the filter value", () => {
    expect(displayFilterValue(fileWithHeader("light_0001.fits", { FILTER: "656.3" }))).toBe("656.3");
    expect(displayFilterValue(fileWithHeader("light_0001.fits", { FILTER: "5007" }))).toBe("5007");
    expect(displayFilterValue(fileWithHeader("light_0001.fits", { FILTER: "486.1" }))).toBe("486.1");
  });

  it("maps a frame whose only hint is a numeric FILTER into the Ha bin", () => {
    const file = fileWithHeader("light_0001.fits", { FILTER: "656.3" });
    const result = mapFilesByWavelength(DEFAULT_BINS, [file], new Set());
    expect(result.bins.find((b) => b.id === "ha")?.files).toEqual([file.path]);
    expect(result.headerMapped).toBe(1);
  });
});

describe("MIRI imaging filters", () => {
  it("resolves every MIRI imaging filter used by the dataset", () => {
    for (const [code, nm] of [["F560W", 5600], ["F770W", 7700], ["F1130W", 11300], ["F1280W", 12800]] as [string, number][]) {
      expect(resolveFileFilter(fileWithHeader("x.fits", { FILTER: code }))).toEqual({ code, nm });
    }
  });
});

describe("splitSpectralThirds", () => {
  it("keeps every file of a filter together and covers R, G and B", () => {
    const entries = [
      { item: "a1", code: "F090W", nm: 900 },
      { item: "a2", code: "F090W", nm: 900 },
      { item: "b1", code: "F200W", nm: 2000 },
      { item: "c1", code: "F277W", nm: 2770 },
      { item: "c2", code: "F277W", nm: 2770 },
      { item: "d1", code: "F444W", nm: 4440 },
      { item: "e1", code: "F770W", nm: 7700 },
      { item: "f1", code: "F1130W", nm: 11300 },
    ];
    const split = splitSpectralThirds(groupByWavelength(entries));
    expect(split.b).toEqual(["a1", "a2", "b1"]);
    expect(split.g).toEqual(["c1", "c2", "d1"]);
    expect(split.r).toEqual(["e1", "f1"]);
    expect([...split.r, ...split.g, ...split.b].sort()).toEqual(entries.map((e) => e.item).sort());
  });

  it("maps two filters to R and B and one filter to nothing", () => {
    const two = splitSpectralThirds(groupByWavelength([
      { item: "x", code: "F090W", nm: 900 },
      { item: "y", code: "F444W", nm: 4440 },
    ]));
    expect(two).toEqual({ r: ["y"], g: [], b: ["x"] });

    const one = splitSpectralThirds(groupByWavelength([{ item: "x", code: "F090W", nm: 900 }]));
    expect(one).toEqual({ r: [], g: [], b: [] });
  });
});

describe("assignment sets", () => {
  const bins: FrequencyBin[] = [
    { id: "r", label: "Red", shortLabel: "R", color: "#f00", files: ["/a.fits"] },
    { id: "g", label: "Green", shortLabel: "G", color: "#0f0", files: [] },
    { id: "b", label: "Blue", shortLabel: "B", color: "#00f", files: [] },
    { id: "l", label: "Luminance", shortLabel: "L", color: "#fff", files: ["/lum.fits"] },
  ];

  it("counts luminance files as assigned", () => {
    expect(assignedPaths(bins).has("/lum.fits")).toBe(true);
    expect(assignedPaths(bins).size).toBe(2);
  });

  it("keeps luminance files reusable in the colour bin dropdowns", () => {
    expect(exclusivelyAssignedPaths(bins).has("/lum.fits")).toBe(false);
    expect(exclusivelyAssignedPaths(bins).has("/a.fits")).toBe(true);
  });

  it("reports whether the colour bins still need files", () => {
    expect(colorBinsEmpty(bins)).toBe(false);
    expect(colorBinsEmpty(bins.map((b) => ({ ...b, files: b.id === "l" ? b.files : [] })))).toBe(true);
  });
});

describe("mapFilesByWavelength on the real MAST download", () => {
  const NIRCAM_FILTERS = ["F090W", "F200W", "F212N", "F277W", "F335M", "F444W"];
  const MIRI_FILES: [string, string][] = [
    ["jw02016-o001_t023_miri_f560w_i2d.fits", "F560W"],
    ["jw02016-o001_t023_miri_f770w_i2d.fits", "F770W"],
    ["jw02016-o001_t023_miri_f1130w_i2d.fits", "F1130W"],
    ["jw06565-o004_t001_miri_f770w_i2d.fits", "F770W"],
    ["jw06565-o004_t001_miri_f1130w_i2d.fits", "F1130W"],
    ["jw06565-o004_t001_miri_f1280w_i2d.fits", "F1280W"],
  ];

  function mastFiles() {
    const files: { name: string; path: string; result: { header: Record<string, string> } }[] = [];
    for (const obs of ["o002", "o003", "o005"]) {
      for (const filter of NIRCAM_FILTERS) {
        const name = `jw06565-${obs}_t001_nircam_clear-${filter.toLowerCase()}_i2d.fits`;
        files.push({
          name,
          path: `C:/MAST/JWST/${name}`,
          result: { header: { FILTER: filter, PUPIL: "CLEAR", INSTRUME: "NIRCAM" } },
        });
      }
    }
    for (const [name, filter] of MIRI_FILES) {
      files.push({
        name,
        path: `C:/MAST/JWST/${name}`,
        result: { header: { FILTER: filter, INSTRUME: "MIRI" } },
      });
    }
    return files;
  }

  const emptyBins = (): FrequencyBin[] => DEFAULT_BINS.map((b) => ({ ...b, files: [] }));

  it("assigns all 24 files and fills R, G and B", () => {
    const files = mastFiles();
    expect(files).toHaveLength(24);

    const result = mapFilesByWavelength(emptyBins(), files, new Set());
    const byId = (id: string) => result.bins.find((b) => b.id === id)?.files ?? [];

    expect(result.unmapped).toEqual([]);
    expect(byId("ha")).toEqual([]);
    expect(byId("l")).toEqual([]);
    expect(byId("b")).toHaveLength(9);
    expect(byId("g")).toHaveLength(9);
    expect(byId("r")).toHaveLength(6);
    expect(assignedPaths(result.bins).size).toBe(24);
  });

  it("puts every file in exactly one bin instead of duplicating it into a wavelength bin", () => {
    const result = mapFilesByWavelength(emptyBins(), mastFiles(), new Set());
    const totalBinEntries = result.bins.reduce((a, b) => a + b.files.length, 0);
    expect(totalBinEntries).toBe(24);
    expect(assignedPaths(result.bins).size).toBe(24);
    expect(result.dynamicBinCount).toBe(0);
    expect(result.wavelengthMapped).toBe(0);
    expect(result.spectralMapped).toBe(24);
  });

  it("keeps a wavelength bin when a single filter cannot be split into colours", () => {
    const files = ["o002", "o003", "o005"].map((obs) => {
      const name = `jw06565-${obs}_t001_nircam_clear-f200w_i2d.fits`;
      return { name, path: `C:/MAST/JWST/${name}`, result: { header: { FILTER: "F200W", PUPIL: "CLEAR" } } };
    });
    const result = mapFilesByWavelength(emptyBins(), files, new Set());
    expect(result.bins.find((b) => b.id === "wl2000")?.files).toHaveLength(3);
    expect(result.dynamicBinCount).toBe(1);
    expect(result.bins.reduce((a, b) => a + b.files.length, 0)).toBe(3);
    expect(result.spectralMapped).toBe(0);
  });

  it("keeps the three exposures of one filter in the same colour bin", () => {
    const result = mapFilesByWavelength(emptyBins(), mastFiles(), new Set());
    const red = result.bins.find((b) => b.id === "r")?.files ?? [];
    expect(red.filter((p) => p.includes("f1130w"))).toHaveLength(2);
    const blue = result.bins.find((b) => b.id === "b")?.files ?? [];
    expect(blue.filter((p) => p.includes("f090w"))).toHaveLength(3);
    expect(blue.every((p) => !p.includes("f444w"))).toBe(true);
  });

  it("reports MIRI MRS cubes that carry no filter keyword instead of dropping them silently", () => {
    const cubes = ["ch1-long", "ch2-short"].map((band) => {
      const name = `jw02016-c1012_t023_miri_${band}_s3d.fits`;
      return { name, path: `C:/MAST/JWST/${name}`, result: { header: { INSTRUME: "MIRI", BAND: band } } };
    });
    const result = mapFilesByWavelength(emptyBins(), cubes, new Set());
    expect(result.unmapped).toEqual([
      { name: "jw02016-c1012_t023_miri_ch1-long_s3d.fits", filter: null },
      { name: "jw02016-c1012_t023_miri_ch2-short_s3d.fits", filter: null },
    ]);
    expect(assignedPaths(result.bins).size).toBe(0);
  });

  it("leaves classic narrowband sets on their own bins", () => {
    const files = [
      { name: "M42_Ha_300s.fits", path: "/M42_Ha_300s.fits", result: { header: { FILTER: "Halpha" } } },
      { name: "M42_OIII_300s.fits", path: "/M42_OIII_300s.fits", result: { header: { FILTER: "OIII" } } },
      { name: "M42_SII_300s.fits", path: "/M42_SII_300s.fits", result: { header: { FILTER: "SII" } } },
    ];
    const result = mapFilesByWavelength(emptyBins(), files, new Set());
    expect(result.bins.find((b) => b.id === "ha")?.files).toEqual(["/M42_Ha_300s.fits"]);
    expect(result.bins.find((b) => b.id === "oiii")?.files).toEqual(["/M42_OIII_300s.fits"]);
    expect(result.bins.find((b) => b.id === "sii")?.files).toEqual(["/M42_SII_300s.fits"]);
    expect(result.dynamicBinCount).toBe(0);
    expect(result.spectralMapped).toBe(0);
  });
});

describe("filenameChannelSlot", () => {
  it("does not match a two-character needle inside an unrelated word", () => {
    expect(filenameChannelSlot("NGC7000_shark_L.fits")).toBe("L");
    expect(filenameChannelSlot("jw02016-o023_t023_nirspec_g140h_s2d.fits")).toBeNull();
    expect(filenameChannelSlot("jw01234-o001_t001_nircam_f150w2_i2d.fits")).toBeNull();
    expect(filenameChannelSlot(NIRCAM_NAME)).toBeNull();
  });

  it("still maps classic amateur channel suffixes", () => {
    expect(filenameChannelSlot("M42_R.fits")).toBe("R");
    expect(filenameChannelSlot("M42_G_300s.fits")).toBe("G");
    expect(filenameChannelSlot("M42_Ha.fits")).toBe("R");
    expect(filenameChannelSlot("M42_SII_300s.fits")).toBe("B");
  });

  it("maps a channel word that opens the filename", () => {
    expect(filenameChannelSlot("Red_M42_001.fits")).toBe("R");
    expect(filenameChannelSlot("Green_M42_001.fits")).toBe("G");
    expect(filenameChannelSlot("Blue_stack.fits")).toBe("B");
    expect(filenameChannelSlot("Luminance_M42.fits")).toBe("L");
    expect(filenameChannelSlot("OIII_M42.fits")).toBe("G");
    expect(filenameChannelSlot("656nm_M42.fits")).toBe("R");
  });

  it("accepts a dot as a channel token separator", () => {
    expect(detectChannelByFilename(fileWithHeader("M42.Ha.fits", null))).toBe("ha");
    expect(detectChannelByFilename(fileWithHeader("M42.SII.300s.fits", null))).toBe("sii");
  });
});

describe("runAutoMap", () => {
  const emptyBins = (): FrequencyBin[] => DEFAULT_BINS.map((b) => ({ ...b, files: [] }));
  const binFiles = (result: { bins: FrequencyBin[] }, id: string) =>
    result.bins.find((b) => b.id === id)?.files ?? [];

  const narrowbandFiles = [
    { name: "M42_Ha_300s.fits", path: "/M42_Ha_300s.fits", result: null },
    { name: "M42_OIII_300s.fits", path: "/M42_OIII_300s.fits", result: null },
    { name: "M42_SII_300s.fits", path: "/M42_SII_300s.fits", result: null },
    { name: "stack_result.fits", path: "/stack_result.fits", result: null },
  ];

  it("keeps the assignments of a stage that maps files and then falls through", () => {
    const result = runAutoMap({
      bins: emptyBins(),
      files: narrowbandFiles,
      assigned: new Set(),
      detections: [
        { path: "/M42_Ha_300s.fits", filter: "Halpha" },
        { path: "/M42_OIII_300s.fits", filter: "OIII" },
        { path: "/M42_SII_300s.fits", filter: "SII" },
      ],
    });

    expect(binFiles(result, "ha")).toEqual(["/M42_Ha_300s.fits"]);
    expect(binFiles(result, "oiii")).toEqual(["/M42_OIII_300s.fits"]);
    expect(binFiles(result, "sii")).toEqual(["/M42_SII_300s.fits"]);
    expect(result.mappedCount).toBe(3);
    expect(result.sources).toEqual(["FITS Headers (Rust)"]);
    expect(result.unmapped).toEqual([{ name: "stack_result.fits", filter: null }]);
  });

  it("keeps a partial palette assignment when a later stage resolves nothing", () => {
    const result = runAutoMap({
      bins: emptyBins(),
      files: narrowbandFiles,
      assigned: new Set(),
      palette: {
        palette_name: "SHO",
        is_complete: true,
        r_file: { file_path: "/M42_SII_300s.fits" },
        g_file: { file_path: "/M42_Ha_300s.fits" },
        b_file: { file_path: "/M42_OIII_300s.fits" },
      },
    });

    expect(binFiles(result, "r")).toEqual(["/M42_SII_300s.fits"]);
    expect(binFiles(result, "g")).toEqual(["/M42_Ha_300s.fits"]);
    expect(binFiles(result, "b")).toEqual(["/M42_OIII_300s.fits"]);
    expect(result.sources).toEqual(["SHO"]);
    expect(result.mappedCount).toBe(3);
  });

  it("reports nothing mapped when no stage resolves a file", () => {
    const result = runAutoMap({
      bins: emptyBins(),
      files: [{ name: "stack_result.fits", path: "/stack_result.fits", result: null }],
      assigned: new Set(),
    });
    expect(result.mappedCount).toBe(0);
    expect(result.sources).toEqual([]);
    expect(assignedPaths(result.bins).size).toBe(0);
  });

  it("returns early once the colour bins are filled", () => {
    const result = runAutoMap({
      bins: emptyBins(),
      files: narrowbandFiles,
      assigned: new Set(),
      detections: [
        { path: "/M42_OIII_300s.fits", filter: "Red" },
        { path: "/M42_SII_300s.fits", filter: "Green" },
        { path: "/stack_result.fits", filter: "Blue" },
      ],
    });
    expect(binFiles(result, "r")).toEqual(["/M42_OIII_300s.fits"]);
    expect(binFiles(result, "g")).toEqual(["/M42_SII_300s.fits"]);
    expect(binFiles(result, "b")).toEqual(["/stack_result.fits"]);
    expect(binFiles(result, "ha")).toEqual([]);
    expect(result.unmapped).toEqual([]);
    expect(result.sources).toEqual(["FITS Headers (Rust)"]);
  });
});
