import { describe, it, expect } from "vitest";
import { fileListNames, type FileListEntry } from "../fileListNames";

const HEAVY_TEST = [
  "jw02739-o001_t001_nircam_clear-f090w_i2d.fits",
  "jw02739-o001_t001_nircam_clear-f187n_i2d.fits",
  "jw02739-o001_t001_nircam_clear-f200w_i2d.fits",
  "jw02739-o001_t001_nircam_clear-f335m_i2d.fits",
  "jw02739-o001_t001_nircam_clear-f444w_i2d.fits",
  "jw02739-o001_t001_nircam_f444w-f470n_i2d.fits",
];

const M104_MRS = ["jw02016-c1012_t023", "jw02016-o023_t023"].flatMap((head) =>
  ["ch1", "ch2", "ch3", "ch4"].flatMap((channel) =>
    ["short", "medium", "long"].flatMap((band) =>
      ["s3d", "x1d"].map((product) => `${head}_miri_${channel}-${band}_${product}.fits`),
    ),
  ),
);

const M104_MIRI_IMAGING = [
  ...["f560w", "f770w", "f1130w"].map((filter) => `jw02016-o023_t023_miri_${filter}`),
  ...["f770w", "f1130w", "f1280w"].map((filter) => `jw06565-o001_t001_miri_${filter}`),
].flatMap((stem) => [`${stem}_i2d.fits`, `${stem}_segm.fits`]);

const M104_NIRCAM = ["jw06565-o002_t001", "jw06565-o003_t001", "jw06565-o005_t002"].flatMap((head) =>
  ["f090w", "f200w", "f212n", "f277w", "f335m", "f444w"].flatMap((filter) =>
    ["i2d", "segm"].map((product) => `${head}_nircam_clear-${filter}_${product}.fits`),
  ),
);

const M104 = [...M104_MRS, ...M104_MIRI_IMAGING, ...M104_NIRCAM];

const NIRSPEC_1266 = [
  ...["g235h-f170lp", "g395h-f290lp"].flatMap((setting) =>
    ["s3d", "x1d"].map((product) => `jw01266-o005_t001_nirspec_${setting}_${product}.fits`),
  ),
  ...["00001", "00002", "00003", "00004"].flatMap((exposure) =>
    ["nrs1", "nrs2"].flatMap((detector) =>
      ["cal", "rate", "rateints", "s3d", "uncal", "x1d"].map(
        (product) => `jw01266005001_02103_${exposure}_${detector}_${product}.fits`,
      ),
    ),
  ),
];

const WFPC2 = ["502nmos.fits", "656nmos.fits", "673nmos.fits"];

const CUTOUTS = [
  "f444w_cutout_1232_2381.fits",
  "jw02739-o001_t001_nircam_clear-f444w_i2d_cutout.fits",
  "jw02739-o001_t001_nircam_clear-f444w_i2d_cutout_976_2125_512x512.fits",
  "X3-cutout-512.fits",
];

const EVERY_FOLDER = [...HEAVY_TEST, ...M104, ...NIRSPEC_1266, ...WFPC2, ...CUTOUTS];

function stemOf(name: string): string {
  return name.replace(/\.[^.]+$/, "");
}

function inFolder(names: readonly string[]): FileListEntry[] {
  return names.map((name) => ({ name, path: `C:/data/${name}` }));
}

function texts(names: readonly string[]): string[] {
  return fileListNames(inFolder(names)).map((label) => label.text);
}

function at(path: string): FileListEntry {
  return { name: path.split(/[\\/]/).pop() ?? path, path };
}

function labelsOf(paths: readonly string[]): string[] {
  return fileListNames(paths.map(at)).map((label) => label.text);
}

const ASIAIR_LIGHT = "Light_M42_120.0s_Bin1_294MC_gain100_20231010-203040_-10.0C_0001.fits";

describe("fileListNames", () => {
  it("keeps both optical elements so clear-f444w and f444w-f470n differ", () => {
    expect(texts([
      "jw02739-o001_t001_nircam_clear-f444w_i2d.fits",
      "jw02739-o001_t001_nircam_f444w-f470n_i2d.fits",
    ])).toEqual(["...clear-f444w_i2d", "...f444w-f470n_i2d"]);
  });

  it("shortens 3- and 4-digit MIRI filters the same way", () => {
    expect(texts([
      "jw02016-o023_t023_miri_f560w_i2d.fits",
      "jw02016-o023_t023_miri_f770w_i2d.fits",
      "jw02016-o023_t023_miri_f1130w_i2d.fits",
    ])).toEqual(["...f560w_i2d", "...f770w_i2d", "...f1130w_i2d"]);
  });

  it("a cutout keeps the cutout word and its coordinates", () => {
    expect(texts(["f444w_cutout_1232_2381.fits"])).toEqual(["f444w_cutout_1232_2381"]);
    expect(texts([
      "jw02739-o001_t001_nircam_clear-f444w_i2d.fits",
      "jw02739-o001_t001_nircam_clear-f444w_i2d_cutout.fits",
    ])).toEqual(["...clear-f444w_i2d", "...clear-f444w_i2d_cutout"]);
  });

  it("files that differ only in observation, association or program get the first differing token", () => {
    expect(texts([
      "jw06565-o002_t001_nircam_clear-f090w_i2d.fits",
      "jw06565-o003_t001_nircam_clear-f090w_i2d.fits",
      "jw06565-o005_t002_nircam_clear-f090w_i2d.fits",
    ])).toEqual(["o002...clear-f090w_i2d", "o003...clear-f090w_i2d", "o005...clear-f090w_i2d"]);
    expect(texts([
      "jw02016-c1012_t023_miri_ch3-short_s3d.fits",
      "jw02016-o023_t023_miri_ch3-short_s3d.fits",
    ])).toEqual(["c1012...ch3-short_s3d", "o023...ch3-short_s3d"]);
    expect(texts([
      "jw02016-o023_t023_miri_f1130w_i2d.fits",
      "jw06565-o001_t001_miri_f1130w_i2d.fits",
    ])).toEqual(["jw02016...f1130w_i2d", "jw06565...f1130w_i2d"]);
  });

  it("falls back to the full stem when the first differing token is still shared", () => {
    expect(texts([
      "jw01000-o001_t001_nircam_clear-f090w_i2d.fits",
      "jw01000-o002_t001_nircam_clear-f090w_i2d.fits",
      "jw02000-o001_t001_nircam_clear-f090w_i2d.fits",
    ])).toEqual([
      "jw01000-o001_t001_nircam_clear-f090w_i2d",
      "jw01000-o002_t001_nircam_clear-f090w_i2d",
      "jw02000...clear-f090w_i2d",
    ]);
  });

  it("keeps the whole name when it is only an extension", () => {
    expect(texts([".fits", ".fit", "502nmos.fits"])).toEqual([".fits", ".fit", "502nmos"]);
  });

  it("every name in the real JWST folders is unique", () => {
    expect([HEAVY_TEST.length, M104.length, NIRSPEC_1266.length]).toEqual([6, 96, 52]);
    for (const folder of [HEAVY_TEST, M104, NIRSPEC_1266, EVERY_FOLDER]) {
      const shown = texts(folder);
      const repeated = shown.filter((text, i) => shown.indexOf(text) !== i);
      expect(repeated).toEqual([]);
    }
  });

  it("the shown text is a literal piece of the file name", () => {
    const shown = texts(EVERY_FOLDER);
    const notLiteral = EVERY_FOLDER.filter((name, i) => !shown[i].split("...").every((part) => name.includes(part)));
    expect(notLiteral).toEqual([]);
  });

  it("shows '...' exactly on the rows where part of the name was left out", () => {
    const shown = texts(EVERY_FOLDER);
    const mismarked = EVERY_FOLDER
      .map((name, i) => ({ name, text: shown[i] }))
      .filter(({ name, text }) => text.includes("...") === (text === stemOf(name)));
    expect(mismarked).toEqual([]);
  });

  it("shortens an ASIAIR light whose temperature follows an underscore and a minus", () => {
    const shown = texts([ASIAIR_LIGHT]);
    expect(shown[0].startsWith("...")).toBe(true);
    expect(shown).toEqual(["...203040_-10.0C_0001"]);
    expect(texts([
      "Light_M42_120.0s_Bin1_294MC_gain100_20231010-203040_-10.0C_0001.fits",
      "Light_M42_120.0s_Bin1_294MC_gain100_20231010-203251_-10.0C_0002.fits",
    ])).toEqual(["...203040_-10.0C_0001", "...203251_-10.0C_0002"]);
  });

  it("ASIAIR lights from two nights with the same time and number get the date", () => {
    expect(texts([
      "Light_M42_120.0s_Bin1_294MC_gain100_20231010-203040_-10.0C_0001.fits",
      "Light_M42_120.0s_Bin1_294MC_gain100_20231011-203040_-10.0C_0001.fits",
    ])).toEqual(["20231010...203040_-10.0C_0001", "20231011...203040_-10.0C_0001"]);
  });

  it("the same file name in two folders gets its parent folder", () => {
    expect(labelsOf(["C:/night/Ha/master_flat.fits", "C:/night/OIII/master_flat.fits"])).toEqual(["Ha/master_flat", "OIII/master_flat"]);
    expect(labelsOf(["C:\\night\\Ha\\master_flat.fits", "C:\\night\\OIII\\master_flat.fits"])).toEqual(["Ha/master_flat", "OIII/master_flat"]);
  });

  it("climbs folders until the labels differ", () => {
    expect(labelsOf(["D:/n1/flats/master_flat.fits", "D:/n2/flats/master_flat.fits"])).toEqual(["n1/flats/master_flat", "n2/flats/master_flat"]);
  });

  it("keeps the short form of a long name listed from two folders", () => {
    expect(labelsOf([
      "C:/a/jw02739-o001_t001_nircam_clear-f444w_i2d.fits",
      "C:/b/jw02739-o001_t001_nircam_clear-f444w_i2d.fits",
      "C:/a/jw02739-o002_t001_nircam_clear-f444w_i2d.fits",
    ])).toEqual(["a/o001...clear-f444w_i2d", "b/o001...clear-f444w_i2d", "o002...clear-f444w_i2d"]);
  });

  it("leaves a name listed once without a folder, and the same path listed twice as it is", () => {
    expect(labelsOf(["C:/night/Ha/master_flat.fits", "C:/night/OIII/master_flat.fits", "C:/night/light_001.fits"]))
      .toEqual(["Ha/master_flat", "OIII/master_flat", "light_001"]);
    expect(labelsOf(["C:/x/master_flat.fits", "C:/x/master_flat.fits"])).toEqual(["master_flat", "master_flat"]);
  });

  it("gives a path opened again the folder label of its first row", () => {
    expect(labelsOf(["C:/n/Ha/master_flat.fits", "C:/n/OIII/master_flat.fits", "C:/n/Ha/master_flat.fits"]))
      .toEqual(["Ha/master_flat", "OIII/master_flat", "Ha/master_flat"]);
    expect(labelsOf(["C:\\n\\Ha\\master_flat.fits", "C:/n/OIII/master_flat.fits", "C:/n/Ha/master_flat.fits"]))
      .toEqual(["Ha/master_flat", "OIII/master_flat", "Ha/master_flat"]);
  });

  it("clips an unshortened name at the start and a shortened one at the end", () => {
    const labels = fileListNames(inFolder(["M42_LRGB_final.fits", "f444w_cutout_1232_2381.fits", ASIAIR_LIGHT]));
    expect(labels.map((label) => label.clipStart)).toEqual([true, true, false]);
  });

  it("clips at the end a full name kept to resolve a collision and a folder label, whose differing part comes first", () => {
    const fallback = fileListNames(inFolder([
      "jw01000-o001_t001_nircam_clear-f090w_i2d.fits",
      "jw01000-o002_t001_nircam_clear-f090w_i2d.fits",
      "jw02000-o001_t001_nircam_clear-f090w_i2d.fits",
    ]));
    expect(fallback.map((label) => label.clipStart)).toEqual([false, false, false]);
    const folders = fileListNames(["C:/night/Ha/master_flat.fits", "C:/night/OIII/master_flat.fits"].map(at));
    expect(folders.map((label) => label.clipStart)).toEqual([false, false]);
  });
});
