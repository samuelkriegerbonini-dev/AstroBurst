import { describe, it, expect } from "vitest";
import { fileListNames } from "../fileListNames";

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

describe("fileListNames", () => {
  it("keeps both optical elements so clear-f444w and f444w-f470n differ", () => {
    expect(fileListNames([
      "jw02739-o001_t001_nircam_clear-f444w_i2d.fits",
      "jw02739-o001_t001_nircam_f444w-f470n_i2d.fits",
    ])).toEqual(["...clear-f444w_i2d", "...f444w-f470n_i2d"]);
  });

  it("shortens 3- and 4-digit MIRI filters the same way", () => {
    expect(fileListNames([
      "jw02016-o023_t023_miri_f560w_i2d.fits",
      "jw02016-o023_t023_miri_f770w_i2d.fits",
      "jw02016-o023_t023_miri_f1130w_i2d.fits",
    ])).toEqual(["...f560w_i2d", "...f770w_i2d", "...f1130w_i2d"]);
  });

  it("a cutout keeps the cutout word and its coordinates", () => {
    expect(fileListNames(["f444w_cutout_1232_2381.fits"])).toEqual(["f444w_cutout_1232_2381"]);
    expect(fileListNames([
      "jw02739-o001_t001_nircam_clear-f444w_i2d.fits",
      "jw02739-o001_t001_nircam_clear-f444w_i2d_cutout.fits",
    ])).toEqual(["...clear-f444w_i2d", "...clear-f444w_i2d_cutout"]);
  });

  it("files that differ only in observation, association or program get the first differing token", () => {
    expect(fileListNames([
      "jw06565-o002_t001_nircam_clear-f090w_i2d.fits",
      "jw06565-o003_t001_nircam_clear-f090w_i2d.fits",
      "jw06565-o005_t002_nircam_clear-f090w_i2d.fits",
    ])).toEqual(["o002...clear-f090w_i2d", "o003...clear-f090w_i2d", "o005...clear-f090w_i2d"]);
    expect(fileListNames([
      "jw02016-c1012_t023_miri_ch3-short_s3d.fits",
      "jw02016-o023_t023_miri_ch3-short_s3d.fits",
    ])).toEqual(["c1012...ch3-short_s3d", "o023...ch3-short_s3d"]);
    expect(fileListNames([
      "jw02016-o023_t023_miri_f1130w_i2d.fits",
      "jw06565-o001_t001_miri_f1130w_i2d.fits",
    ])).toEqual(["jw02016...f1130w_i2d", "jw06565...f1130w_i2d"]);
  });

  it("falls back to the full stem when the first differing token is still shared", () => {
    expect(fileListNames([
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
    expect(fileListNames([".fits", ".fit", "502nmos.fits"])).toEqual([".fits", ".fit", "502nmos"]);
  });

  it("every name in the real JWST folders is unique", () => {
    expect([HEAVY_TEST.length, M104.length, NIRSPEC_1266.length]).toEqual([6, 96, 52]);
    for (const folder of [HEAVY_TEST, M104, NIRSPEC_1266, EVERY_FOLDER]) {
      const shown = fileListNames(folder);
      const repeated = shown.filter((text, i) => shown.indexOf(text) !== i);
      expect(repeated).toEqual([]);
    }
  });

  it("the shown text is a literal piece of the file name", () => {
    const shown = fileListNames(EVERY_FOLDER);
    const notLiteral = EVERY_FOLDER.filter((name, i) => !shown[i].split("...").every((part) => name.includes(part)));
    expect(notLiteral).toEqual([]);
  });

  it("shows '...' exactly on the rows where part of the name was left out", () => {
    const shown = fileListNames(EVERY_FOLDER);
    const mismarked = EVERY_FOLDER
      .map((name, i) => ({ name, text: shown[i] }))
      .filter(({ name, text }) => text.includes("...") === (text === stemOf(name)));
    expect(mismarked).toEqual([]);
  });
});
