import { describe, expect, it } from "vitest";
import {
  DISPLAY_MEMORY_FILE_CAP,
  DISPLAY_MEMORY_GROUP_CAP,
  IDENTITY_STF,
  defaultsLook,
  displayGroupKey,
  editWithLinear,
  emptyDisplayMemory,
  fileEntry,
  forgetStf,
  linearTitle,
  linearToggle,
  parseDisplayMemory,
  patchTouchesLook,
  pickLook,
  rememberLook,
  rememberRgbEdit,
  rememberStf,
  resolveFileDisplay,
  resolveStf,
  rgbLinearToggle,
  rgbSeed,
  serializeDisplayMemory,
  setDefaults,
  setRgbLinear,
  setSameGroup,
  stfSourceKey,
  touchFile,
  withLook,
  type DisplayMemoryDoc,
  type FileDisplayEntry,
  type LinearUndo,
  type RgbStf,
  type StfSlot,
} from "../displayMemory";
import {
  DEFAULT_DISPLAY_SETTINGS,
  DISPLAY_LOOK_KEYS,
  type DisplayLook,
  type DisplaySettings,
} from "../../shared/types/display";
import { reconcileDisplayPatch } from "../displayTransfer";
import type { StfParams } from "../../shared/types";

const F = DEFAULT_DISPLAY_SETTINGS;
const FACTORY: DisplayLook = {
  stretch: "mtf",
  limits: "minmax",
  percentileLow: 1,
  percentileHigh: 99.5,
  userLo: null,
  userHi: null,
  zscaleContrast: 0.25,
  asinhA: 0.1,
  power: 2,
  colormap: "gray",
  invert: false,
  symmetric: false,
  centre: 0,
};
const ASINH: DisplayLook = { ...FACTORY, stretch: "asinh", limits: "zscale", colormap: "viridis", invert: true };
const MAGMA: DisplayLook = { ...FACTORY, colormap: "magma" };
const LINEAR_FROM_MTF: LinearUndo = { stretch: "mtf", limits: "minmax" };

const R: StfParams = { shadow: 0.02, midtone: 0.017, highlight: 1 };
const G: StfParams = { shadow: 0.01, midtone: 0.05, highlight: 1 };
const B: StfParams = { shadow: 0.03, midtone: 0.02, highlight: 1 };
const AUTO_RGB = { r: R, g: G, b: B };
const AUTO_UNLINKED: RgbStf = { ...AUTO_RGB, linked: false };
const ID_UNLINKED: RgbStf = { r: IDENTITY_STF, g: IDENTITY_STF, b: IDENTITY_STF, linked: false };
const USER_RGB: RgbStf = { r: { ...R, midtone: 0.3 }, g: G, b: B, linked: false };

const SLOT: StfSlot = { source: "C:/a.fits|0|0", params: { shadow: 0.1, midtone: 0.2, highlight: 0.9 } };
const SLOT_PROCESSED: StfSlot = { source: "C:/out/bg.fits|3|0", params: { shadow: 0.05, midtone: 0.4, highlight: 1 } };

function header(h: Record<string, string>) {
  return { name: "x.fits", path: "C:/x.fits", result: { header: h } };
}

function keys(list: [string, unknown][]): string[] {
  return list.map(([k]) => k);
}

function fresh(): DisplayMemoryDoc {
  return emptyDisplayMemory(null);
}

describe("display look", () => {
  it("pickLook keeps the 13 look fields and withLook keeps the overlays of the target", () => {
    const source: DisplaySettings = {
      ...F,
      stretch: "asinh",
      limits: "percentile",
      percentileLow: 2,
      percentileHigh: 99,
      userLo: -1,
      userHi: 4,
      zscaleContrast: 0.3,
      asinhA: 0.05,
      power: 3,
      colormap: "inferno",
      invert: true,
      symmetric: true,
      centre: 1.5,
      grid: true,
      gridFrame: "galactic",
      gridDensity: 5,
      compass: true,
      colorbar: false,
    };
    const look = pickLook(source);
    expect(Object.keys(look).sort()).toEqual([...DISPLAY_LOOK_KEYS].sort());
    expect(DISPLAY_LOOK_KEYS).toHaveLength(13);
    expect(look).toEqual({
      stretch: "asinh",
      limits: "percentile",
      percentileLow: 2,
      percentileHigh: 99,
      userLo: -1,
      userHi: 4,
      zscaleContrast: 0.3,
      asinhA: 0.05,
      power: 3,
      colormap: "inferno",
      invert: true,
      symmetric: true,
      centre: 1.5,
    });
    const target: DisplaySettings = { ...F, grid: false, gridFrame: "fk5", gridDensity: 2, compass: false, colorbar: true };
    expect(withLook(target, look)).toEqual({ ...look, grid: false, gridFrame: "fk5", gridDensity: 2, compass: false, colorbar: true });
    expect(withLook(source, FACTORY)).toEqual({ ...FACTORY, grid: true, gridFrame: "galactic", gridDensity: 5, compass: true, colorbar: false });
  });

  it("patchTouchesLook is false for grid, gridFrame, gridDensity, compass and colorbar patches and true for colormap", () => {
    for (const patch of [{ grid: true }, { gridFrame: "ecliptic" as const }, { gridDensity: 4 }, { compass: true }, { colorbar: false }, {}]) {
      expect(patchTouchesLook(patch)).toBe(false);
    }
    expect(patchTouchesLook({ colormap: "magma" })).toBe(true);
    expect(patchTouchesLook({ grid: true, invert: true })).toBe(true);
    expect(patchTouchesLook(F)).toBe(true);
  });

  it("displayGroupKey gives WFPC2|F502N for FILTNAM1, NIRCAM|F444W for PUPIL CLEAR, NIRCAM|F470N for PUPIL F470N, null without INSTRUME or filter, and trims/uppercases", () => {
    expect(displayGroupKey(header({ INSTRUME: "WFPC2", FILTNAM1: "F502N" }))).toBe("WFPC2|F502N");
    expect(displayGroupKey(header({ INSTRUME: "NIRCAM", FILTER: "F444W", PUPIL: "CLEAR" }))).toBe("NIRCAM|F444W");
    expect(displayGroupKey(header({ INSTRUME: "NIRCAM", FILTER: "F444W", PUPIL: "F470N" }))).toBe("NIRCAM|F470N");
    expect(displayGroupKey(header({ FILTNAM1: "F502N" }))).toBeNull();
    expect(displayGroupKey(header({ INSTRUME: "   ", FILTNAM1: "F502N" }))).toBeNull();
    expect(displayGroupKey(header({ INSTRUME: "WFPC2" }))).toBeNull();
    expect(displayGroupKey({ name: "x.fits", path: "C:/x.fits", result: null })).toBeNull();
    expect(displayGroupKey(null)).toBeNull();
    expect(displayGroupKey(header({ INSTRUME: " wfpc2 ", FILTNAM1: " f656n " }))).toBe("WFPC2|F656N");
  });
});

describe("display memory document", () => {
  it("emptyDisplayMemory migrates the legacy look (asinh/zscale/viridis/invert) and turns user limits into minmax with null lo/hi", () => {
    const legacy = JSON.stringify({ ...F, stretch: "asinh", asinhA: 0.05, limits: "zscale", colormap: "viridis", invert: true, colorbar: false, grid: true });
    expect(emptyDisplayMemory(legacy)).toEqual({
      v: 1,
      defaults: { ...FACTORY, stretch: "asinh", asinhA: 0.05, limits: "zscale", colormap: "viridis", invert: true },
      sameGroup: true,
      files: [],
      groups: [],
    });
    const user = emptyDisplayMemory(JSON.stringify({ ...F, stretch: "linear", limits: "user", userLo: 1, userHi: 5 }));
    expect(user.defaults).toEqual({ ...FACTORY, stretch: "linear", limits: "minmax", userLo: null, userHi: null });
    expect(defaultsLook({ ...FACTORY, limits: "user", userLo: 1, userHi: 5 })).toEqual(FACTORY);
    expect(emptyDisplayMemory(null).defaults).toEqual(FACTORY);
    expect(emptyDisplayMemory("{not json").defaults).toEqual(FACTORY);
  });

  it("parseDisplayMemory migrates for absent, unparsable and v=2 text", () => {
    const legacy = JSON.stringify({ ...F, colormap: "magma" });
    const v2 = JSON.stringify({ v: 2, defaults: { ...FACTORY, colormap: "heat" }, sameGroup: false, files: [["a.fits", { look: ASINH }]], groups: [] });
    for (const text of [null, "", "not json", v2, "[]", "null"]) {
      const doc = parseDisplayMemory(text, legacy);
      expect(doc).toEqual(emptyDisplayMemory(legacy));
      expect(doc.defaults.colormap).toBe("magma");
      expect(doc.sameGroup).toBe(true);
    }
    const v1 = JSON.stringify({ v: 1, defaults: { ...FACTORY, colormap: "heat" }, sameGroup: false, files: [], groups: [] });
    expect(parseDisplayMemory(v1, legacy)).toEqual({ v: 1, defaults: { ...FACTORY, colormap: "heat" }, sameGroup: false, files: [], groups: [] });
  });

  it("parseDisplayMemory sanitizes looks (bad stretch, NaN power) and drops rows with a non-string key or a malformed STF", () => {
    const text = JSON.stringify({
      v: 1,
      defaults: { ...FACTORY, stretch: "bogus", power: "NaN" },
      sameGroup: true,
      files: [
        ["a.fits", { look: { ...FACTORY, stretch: "warp", power: "NaN", colormap: "magma" }, linear: { stretch: "nope", limits: "minmax" } }],
        [7, { look: FACTORY }],
        ["b.fits", { stf: { source: "b|0|0", params: { shadow: "x", midtone: 0.5, highlight: 1 } } }],
        ["b2.fits", { stfProcessed: { source: 5, params: IDENTITY_STF } }],
        ["b3.fits", { rgb: { r: R, g: G, b: null, linked: false } }],
        ["c.fits", { stf: SLOT, linear: LINEAR_FROM_MTF }],
        "garbage",
        ["d.fits"],
      ],
      groups: [
        ["WFPC2|F502N", { look: { ...FACTORY, stretch: "nope" }, linear: null }],
        [3, { look: FACTORY, linear: null }],
        ["NIRCAM|F444W", { look: ASINH, linear: { stretch: "asinh", limits: "zscale" } }],
      ],
    });
    const doc = parseDisplayMemory(text, null);
    expect(doc.defaults).toEqual(FACTORY);
    expect(keys(doc.files)).toEqual(["a.fits", "c.fits"]);
    expect(fileEntry(doc, "a.fits")).toEqual({ look: MAGMA });
    expect(fileEntry(doc, "c.fits")).toEqual({ stf: SLOT, linear: LINEAR_FROM_MTF });
    expect(doc.groups).toEqual([
      ["WFPC2|F502N", { look: FACTORY, linear: null }],
      ["NIRCAM|F444W", { look: ASINH, linear: { stretch: "asinh", limits: "zscale" } }],
    ]);

    const many = JSON.stringify({
      v: 1,
      defaults: FACTORY,
      sameGroup: true,
      files: Array.from({ length: DISPLAY_MEMORY_FILE_CAP + 5 }, (_, i) => [`f${i}`, { look: FACTORY }]),
      groups: Array.from({ length: DISPLAY_MEMORY_GROUP_CAP + 3 }, (_, i) => [`G${i}`, { look: FACTORY, linear: null }]),
    });
    const capped = parseDisplayMemory(many, null);
    expect(capped.files).toHaveLength(DISPLAY_MEMORY_FILE_CAP);
    expect(capped.files[0][0]).toBe("f5");
    expect(capped.groups).toHaveLength(DISPLAY_MEMORY_GROUP_CAP);
    expect(capped.groups[0][0]).toBe("G3");
  });

  it("resolveFileDisplay prefers the file look, then the group look with sameGroup on, then the defaults; a null group key never inherits", () => {
    let doc = setDefaults(fresh(), MAGMA);
    doc = rememberLook(doc, "a.fits", "WFPC2|F502N", ASINH, null);
    expect(resolveFileDisplay(doc, "a.fits", "WFPC2|F502N")).toEqual({ look: ASINH, linear: null, source: "file" });
    expect(resolveFileDisplay(doc, "b.fits", "WFPC2|F502N")).toEqual({ look: ASINH, linear: null, source: "group" });
    expect(resolveFileDisplay(doc, "c.fits", "WFPC2|F656N")).toEqual({ look: MAGMA, linear: null, source: "default" });
    expect(resolveFileDisplay(doc, "b.fits", null)).toEqual({ look: MAGMA, linear: null, source: "default" });
    expect(resolveFileDisplay(doc, null, null)).toEqual({ look: MAGMA, linear: null, source: "default" });

    const groupNewer = rememberLook(doc, "a2.fits", "WFPC2|F502N", { ...ASINH, colormap: "heat" }, null);
    expect(resolveFileDisplay(groupNewer, "a.fits", "WFPC2|F502N").look).toEqual(ASINH);
    expect(resolveFileDisplay(groupNewer, "b.fits", "WFPC2|F502N").look.colormap).toBe("heat");

    const off = setSameGroup(doc, false);
    expect(resolveFileDisplay(off, "b.fits", "WFPC2|F502N")).toEqual({ look: MAGMA, linear: null, source: "default" });
    expect(resolveFileDisplay(off, "a.fits", "WFPC2|F502N").source).toBe("file");
  });

  it("rememberLook updates the file and its group and moves both to the most recent end", () => {
    let doc = fresh();
    doc = rememberLook(doc, "a", "G1", ASINH, null);
    doc = rememberLook(doc, "b", "G2", MAGMA, null);
    expect(keys(doc.files)).toEqual(["a", "b"]);
    expect(keys(doc.groups)).toEqual(["G1", "G2"]);
    doc = rememberLook(doc, "a", "G1", MAGMA, LINEAR_FROM_MTF);
    expect(keys(doc.files)).toEqual(["b", "a"]);
    expect(keys(doc.groups)).toEqual(["G2", "G1"]);
    expect(fileEntry(doc, "a")).toEqual({ look: MAGMA, linear: LINEAR_FROM_MTF });
    expect(doc.groups[1][1]).toEqual({ look: MAGMA, linear: LINEAR_FROM_MTF });

    doc = rememberLook(doc, "a", "G1", ASINH, null);
    expect(fileEntry(doc, "a")).toEqual({ look: ASINH });
    expect(doc.groups[1][1]).toEqual({ look: ASINH, linear: null });

    doc = rememberStf(doc, "a", "stf", SLOT);
    const withSlot = rememberLook(doc, "a", null, MAGMA, null);
    expect(fileEntry(withSlot, "a")).toEqual({ look: MAGMA, stf: SLOT });
    expect(withSlot.groups).toBe(doc.groups);
    expect(rememberLook(withSlot, "a", null, { ...MAGMA }, null)).toBe(withSlot);
  });

  it("rememberLook evicts the oldest file past DISPLAY_MEMORY_FILE_CAP and the oldest group past DISPLAY_MEMORY_GROUP_CAP", () => {
    let doc = fresh();
    for (let i = 0; i < DISPLAY_MEMORY_FILE_CAP; i++) doc = rememberLook(doc, `f${i}`, null, ASINH, null);
    doc = rememberLook(doc, "f0", null, MAGMA, null);
    doc = rememberLook(doc, "new", null, ASINH, null);
    expect(doc.files).toHaveLength(DISPLAY_MEMORY_FILE_CAP);
    expect(fileEntry(doc, "f1")).toBeNull();
    expect(fileEntry(doc, "f0")).toEqual({ look: MAGMA });
    expect(doc.files[0][0]).toBe("f2");
    expect(doc.files[DISPLAY_MEMORY_FILE_CAP - 1][0]).toBe("new");

    let groups = fresh();
    for (let i = 0; i <= DISPLAY_MEMORY_GROUP_CAP; i++) groups = rememberLook(groups, "same.fits", `G${i}`, ASINH, null);
    expect(groups.groups).toHaveLength(DISPLAY_MEMORY_GROUP_CAP);
    expect(groups.groups[0][0]).toBe("G1");
    expect(groups.groups[DISPLAY_MEMORY_GROUP_CAP - 1][0]).toBe(`G${DISPLAY_MEMORY_GROUP_CAP}`);
    expect(groups.files).toHaveLength(1);
  });

  it("touchFile reorders a known file and ignores an unknown path (same doc)", () => {
    let doc = fresh();
    for (const p of ["a", "b", "c"]) doc = rememberLook(doc, p, null, ASINH, null);
    const touched = touchFile(doc, "a");
    expect(keys(touched.files)).toEqual(["b", "c", "a"]);
    expect(fileEntry(touched, "a")).toBe(fileEntry(doc, "a"));
    expect(touchFile(touched, "unknown.fits")).toBe(touched);
    expect(touchFile(touched, "a")).toBe(touched);
  });
});

describe("STF slots", () => {
  it("stfSourceKey separates the original, each processed version and the DQ exclusion", () => {
    const original = stfSourceKey({ filePath: "C:/a.fits", processedPath: null, version: 7, excludeDq: false });
    expect(original).toBe("C:/a.fits|0|0");
    const dq = stfSourceKey({ filePath: "C:/a.fits", processedPath: null, version: 0, excludeDq: true });
    expect(dq).toBe("C:/a.fits|0|1");
    const v3 = stfSourceKey({ filePath: "C:/a.fits", processedPath: "C:/out/bg.fits", version: 3, excludeDq: false });
    const v4 = stfSourceKey({ filePath: "C:/a.fits", processedPath: "C:/out/bg.fits", version: 4, excludeDq: false });
    expect(v3).toBe("C:/out/bg.fits|3|0");
    expect(new Set([original, dq, v3, v4]).size).toBe(4);
  });

  it("resolveStf returns identity while Linear is on, the matching slot, auto on a source mismatch, and null without auto", () => {
    const auto: StfParams = { shadow: 0.02, midtone: 0.1, highlight: 1 };
    const entry: FileDisplayEntry = { stf: SLOT, stfProcessed: SLOT_PROCESSED };
    expect(resolveStf(entry, SLOT.source, false, auto, true)).toEqual(IDENTITY_STF);
    expect(resolveStf(null, SLOT.source, false, null, true)).toEqual(IDENTITY_STF);
    expect(resolveStf(entry, SLOT.source, false, auto, false)).toEqual(SLOT.params);
    expect(resolveStf(entry, SLOT_PROCESSED.source, true, auto, false)).toEqual(SLOT_PROCESSED.params);
    expect(resolveStf(entry, "C:/a.fits|0|1", false, auto, false)).toEqual(auto);
    expect(resolveStf(entry, "C:/out/bg.fits|4|0", true, auto, false)).toEqual(auto);
    expect(resolveStf(entry, SLOT.source, true, auto, false)).toEqual(auto);
    expect(resolveStf(null, SLOT.source, false, null, false)).toBeNull();
  });

  it("forgetStf drops both STF slots and keeps the look", () => {
    let doc = rememberLook(fresh(), "a", null, ASINH, LINEAR_FROM_MTF);
    doc = rememberStf(doc, "a", "stf", SLOT);
    doc = rememberStf(doc, "a", "stfProcessed", SLOT_PROCESSED);
    expect(fileEntry(doc, "a")).toEqual({ look: ASINH, linear: LINEAR_FROM_MTF, stf: SLOT, stfProcessed: SLOT_PROCESSED });
    expect(fileEntry(rememberStf(doc, "a", "stf", null), "a")).toEqual({ look: ASINH, linear: LINEAR_FROM_MTF, stfProcessed: SLOT_PROCESSED });
    const forgot = forgetStf(doc, "a");
    expect(fileEntry(forgot, "a")).toEqual({ look: ASINH, linear: LINEAR_FROM_MTF });
    expect(forgetStf(forgot, "a")).toBe(forgot);
    expect(forgetStf(forgot, "unknown.fits")).toBe(forgot);
    const stfOnly = rememberStf(fresh(), "s", "stf", SLOT);
    expect(fileEntry(stfOnly, "s")).toEqual({ stf: SLOT });
    expect(fileEntry(forgetStf(stfOnly, "s"), "s")).toBeNull();
  });
});

describe("Linear", () => {
  it("linearToggle from mtf/minmax and from asinh/zscale patches linear+minmax and gives back the saved stretch and limits", () => {
    expect(linearToggle(F, null)).toEqual({ patch: { stretch: "linear", limits: "minmax" }, linear: LINEAR_FROM_MTF, restoreStf: false });
    const asinh: DisplaySettings = { ...F, stretch: "asinh", limits: "zscale", colormap: "magma" };
    const on = linearToggle(asinh, null);
    expect(on).toEqual({ patch: { stretch: "linear", limits: "minmax" }, linear: { stretch: "asinh", limits: "zscale" }, restoreStf: false });
    const shown = reconcileDisplayPatch(asinh, on.patch);
    expect(shown).toMatchObject({ stretch: "linear", limits: "minmax", colormap: "magma" });
    expect(linearToggle(shown, on.linear)).toEqual({ patch: { stretch: "asinh", limits: "zscale" }, linear: null, restoreStf: true });
    expect(linearToggle(reconcileDisplayPatch(F, { stretch: "linear", limits: "minmax" }), LINEAR_FROM_MTF).patch).toEqual({ stretch: "mtf", limits: "minmax" });
    expect(linearTitle(null)).toBe(
      "Linear: show the data linearly from its minimum to its maximum (stretch linear, limits minmax, STF reset). Click again to go back",
    );
    expect(linearTitle(LINEAR_FROM_MTF)).toBe("Back to mtf");
    expect(linearTitle({ stretch: "asinh", limits: "zscale" })).toBe("Back to asinh with zscale limits");
  });

  it("editWithLinear clears Linear and restores the STF on a stretch or a limits edit and keeps it on a colormap edit", () => {
    expect(editWithLinear({ stretch: "mtf" }, LINEAR_FROM_MTF)).toEqual({ patch: { stretch: "mtf" }, linear: null, restoreStf: true });
    expect(editWithLinear({ limits: "zscale" }, LINEAR_FROM_MTF)).toEqual({ patch: { limits: "zscale" }, linear: null, restoreStf: true });
    expect(editWithLinear(F, LINEAR_FROM_MTF)).toEqual({ patch: F, linear: null, restoreStf: true });
    for (const patch of [{ colormap: "magma" as const }, { invert: true }, { percentileHigh: 98 }, { grid: true }]) {
      expect(editWithLinear(patch, LINEAR_FROM_MTF)).toEqual({ patch, linear: LINEAR_FROM_MTF, restoreStf: false });
    }
    expect(editWithLinear({ stretch: "asinh" }, null)).toEqual({ patch: { stretch: "asinh" }, linear: null, restoreStf: false });
  });

  it("an untouched file of a group whose last look has Linear on resolves with that Linear undo", () => {
    const shownOnA: DisplaySettings = { ...F, stretch: "asinh", limits: "zscale", colormap: "magma" };
    const edit = linearToggle(shownOnA, null);
    const look = pickLook(reconcileDisplayPatch(shownOnA, edit.patch));
    const doc = rememberLook(fresh(), "a.fits", "WFPC2|F502N", look, edit.linear);
    const onB = resolveFileDisplay(doc, "b.fits", "WFPC2|F502N");
    expect(onB).toEqual({ look, linear: { stretch: "asinh", limits: "zscale" }, source: "group" });
    expect(look).toMatchObject({ stretch: "linear", limits: "minmax", colormap: "magma" });
    expect(linearToggle(withLook(F, onB.look), onB.linear).patch).toEqual({ stretch: "asinh", limits: "zscale" });
  });
});

describe("RGB file view STF", () => {
  it("rgbSeed and rgbLinearToggle: identity unlinked while RGB Linear is on, then the saved STF back", () => {
    expect(rgbSeed(null, AUTO_RGB)).toEqual(AUTO_UNLINKED);
    expect(rgbSeed({ look: ASINH }, AUTO_RGB)).toEqual(AUTO_UNLINKED);
    expect(rgbSeed({ rgb: USER_RGB }, AUTO_RGB)).toEqual(USER_RGB);
    const on = rgbLinearToggle(USER_RGB, null);
    expect(on).toEqual({ next: ID_UNLINKED, rgbLinear: USER_RGB });
    expect(rgbSeed({ rgb: USER_RGB, rgbLinear: USER_RGB }, AUTO_RGB)).toEqual(ID_UNLINKED);
    expect(rgbSeed({ rgbLinear: AUTO_UNLINKED }, AUTO_RGB)).toEqual(ID_UNLINKED);
    expect(rgbLinearToggle(on.next, on.rgbLinear)).toEqual({ next: USER_RGB, rgbLinear: null });
    const linked: RgbStf = { r: R, g: R, b: R, linked: true };
    expect(rgbLinearToggle(linked, null)).toEqual({ next: ID_UNLINKED, rgbLinear: linked });
  });

  it("rememberRgbEdit ignores untouched auto values and the RGB Linear identity, and clears RGB Linear on any other value", () => {
    const empty = fresh();
    expect(rememberRgbEdit(empty, "rgb.fits", AUTO_UNLINKED, AUTO_RGB)).toBe(empty);
    let doc = rememberRgbEdit(empty, "rgb.fits", USER_RGB, AUTO_RGB);
    expect(fileEntry(doc, "rgb.fits")).toEqual({ rgb: USER_RGB });
    expect(rememberRgbEdit(doc, "rgb.fits", { ...USER_RGB }, AUTO_RGB)).toBe(doc);

    doc = setRgbLinear(doc, "rgb.fits", USER_RGB);
    expect(fileEntry(doc, "rgb.fits")).toEqual({ rgb: USER_RGB, rgbLinear: USER_RGB });
    expect(rememberRgbEdit(doc, "rgb.fits", ID_UNLINKED, AUTO_RGB)).toBe(doc);

    const other: RgbStf = { ...USER_RGB, g: { ...G, midtone: 0.2 } };
    const edited = rememberRgbEdit(doc, "rgb.fits", other, AUTO_RGB);
    expect(fileEntry(edited, "rgb.fits")).toEqual({ rgb: other });
    expect(fileEntry(rememberRgbEdit(edited, "rgb.fits", AUTO_UNLINKED, AUTO_RGB), "rgb.fits")).toEqual({ rgb: AUTO_UNLINKED });

    const linearOnly = setRgbLinear(empty, "rgb.fits", AUTO_UNLINKED);
    expect(fileEntry(rememberRgbEdit(linearOnly, "rgb.fits", AUTO_UNLINKED, AUTO_RGB), "rgb.fits")).toEqual({ rgb: AUTO_UNLINKED });
    expect(fileEntry(setRgbLinear(linearOnly, "rgb.fits", null), "rgb.fits")).toBeNull();
  });
});

describe("display memory serialization", () => {
  it("serializeDisplayMemory then parseDisplayMemory is lossless", () => {
    let doc = setDefaults(fresh(), MAGMA);
    doc = setSameGroup(doc, false);
    doc = rememberLook(doc, "C:/data/a.fits#hdu=2", "WFPC2|F502N", ASINH, LINEAR_FROM_MTF);
    doc = rememberStf(doc, "C:/data/a.fits#hdu=2", "stf", SLOT);
    doc = rememberStf(doc, "C:/data/a.fits#hdu=2", "stfProcessed", SLOT_PROCESSED);
    doc = rememberLook(doc, "C:/data/b.fits", null, { ...FACTORY, limits: "user", userLo: -2.5, userHi: 1e6 }, null);
    doc = rememberRgbEdit(doc, "C:/data/rgb.fits", USER_RGB, AUTO_RGB);
    doc = setRgbLinear(doc, "C:/data/rgb.fits", USER_RGB);
    const text = serializeDisplayMemory(doc);
    const parsed = parseDisplayMemory(text, JSON.stringify({ ...F, colormap: "heat" }));
    expect(parsed).toEqual(doc);
    expect(serializeDisplayMemory(parsed)).toBe(text);
    expect(keys(parsed.files)).toEqual(["C:/data/a.fits#hdu=2", "C:/data/b.fits", "C:/data/rgb.fits"]);
  });
});
